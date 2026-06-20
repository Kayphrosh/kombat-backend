use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    handlers::tournament::ingest_pandascore_match,
    models::{
        AgentOutcomeProposalRequest, ApiResponse, CreateOutcomeProposalRequest,
        CreateWalrusArtifactRequest,
    },
    services::{agent_pipeline, pandascore},
    state::AppState,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::err(msg)),
    )
}

fn service_unavailable(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse::err(msg)))
}

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}

// ─── Generic match-result webhook ─────────────────────────────────────────────

/// Payload shape for the generic match-result webhook.
/// Organizer systems POST this when they have a confirmed result.
#[derive(Debug, serde::Deserialize)]
pub struct MatchResultWebhookPayload {
    /// Our internal match UUID OR a legacy numeric provider id.
    pub match_id: String,
    /// Name of the winning opponent — used to find our match_opponents row.
    pub winner_name: String,
    /// Confidence 0–1. Defaults to 1.0 for organizer-confirmed results.
    pub confidence: Option<rust_decimal::Decimal>,
    /// Free-form evidence from the caller. Must include match_id, winner,
    /// and source_data or timestamp.
    pub evidence: serde_json::Value,
    /// Human-readable summary stored alongside the proposal.
    pub summary: Option<String>,
    /// Optional URL pointing to an external result page.
    pub result_url: Option<String>,
}

/// Generic match-result webhook.
/// Secured by HMAC-SHA256 signature in `X-Webhook-Signature` header:
///   signature = hex(HMAC-SHA256(WEBHOOK_SECRET, raw_body))
pub async fn handle_match_result_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<serde_json::Value> {
    verify_webhook_signature(&headers, &body)?;

    let payload: MatchResultWebhookPayload =
        serde_json::from_slice(&body).map_err(|e| bad_request(format!("Invalid JSON: {}", e)))?;

    agent_pipeline::validate_evidence_schema(&payload.evidence).map_err(bad_request)?;

    process_match_result(
        &state,
        &payload.match_id,
        &payload.winner_name,
        payload.confidence,
        &payload.evidence,
        payload.summary.as_deref(),
        payload.result_url.as_deref(),
        "organizer_webhook",
    )
    .await
}

// ─── PandaScore realtime webhook ──────────────────────────────────────────────

/// Realtime PandaScore webhook. PandaScore delivers the full updated match
/// object whenever a match changes state (scheduled → running → finished, score
/// updates, reschedules). We upsert it immediately so the DB mirrors live data
/// without waiting for the next poll, and resolve/create pools as needed.
///
/// Auth: PandaScore lets you append a secret to the callback URL. We accept the
/// token via the `?token=` query param OR an `X-PandaScore-Token` header, and
/// compare it in constant time against `PANDASCORE_WEBHOOK_TOKEN`.
pub async fn handle_pandascore_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> AppResult<serde_json::Value> {
    verify_pandascore_token(&headers, &params)?;

    let raw: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| bad_request(format!("Invalid JSON: {}", e)))?;

    // PandaScore may send the match object directly, or wrap it under a key.
    // Probe the common shapes so we are resilient to payload framing.
    let match_value = extract_match_object(&raw)
        .ok_or_else(|| bad_request("webhook payload did not contain a match object"))?;

    let match_req = pandascore::parse_match_value(match_value)
        .ok_or_else(|| bad_request("could not parse match from webhook payload"))?;

    let result = ingest_pandascore_match(&state, &match_req)
        .await
        .map_err(internal_error)?;

    Ok(Json(ApiResponse::ok(result)))
}

/// Locate the match object inside a PandaScore webhook body. Handles the object
/// being at the top level, or nested under `match`/`data`/`payload`.
fn extract_match_object(raw: &serde_json::Value) -> Option<serde_json::Value> {
    let looks_like_match =
        |v: &serde_json::Value| v.get("id").is_some() && v.get("opponents").is_some();

    if looks_like_match(raw) {
        return Some(raw.clone());
    }
    for key in ["match", "data", "payload", "object"] {
        if let Some(inner) = raw.get(key) {
            if looks_like_match(inner) {
                return Some(inner.clone());
            }
        }
    }
    // Some deliveries wrap as { "type": "match", "match": {...} } already covered;
    // fall back to the top-level object if it at least carries an id.
    if raw.get("id").is_some() {
        return Some(raw.clone());
    }
    None
}

/// Constant-time check of the PandaScore webhook token from query or header.
fn verify_pandascore_token(
    headers: &HeaderMap,
    params: &HashMap<String, String>,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let secret = std::env::var("PANDASCORE_WEBHOOK_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            service_unavailable("PandaScore webhook not configured (PANDASCORE_WEBHOOK_TOKEN unset)")
        })?;

    let provided = params
        .get("token")
        .map(String::as_str)
        .or_else(|| {
            headers
                .get("x-pandascore-token")
                .and_then(|v| v.to_str().ok())
        })
        .ok_or_else(|| unauthorized("missing PandaScore webhook token"))?;

    if !constant_time_eq(provided.as_bytes(), secret.as_bytes()) {
        return Err(unauthorized("invalid PandaScore webhook token"));
    }
    Ok(())
}

// ─── Shared processing ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn process_match_result(
    state: &AppState,
    match_id_input: &str,
    winner_name: &str,
    confidence: Option<rust_decimal::Decimal>,
    evidence: &serde_json::Value,
    summary: Option<&str>,
    result_url: Option<&str>,
    source_label: &str,
) -> AppResult<serde_json::Value> {
    // Resolve to our internal match record. UUID is preferred; legacy numeric
    // provider ids remain accepted for backfills.
    let match_record = if let Ok(uuid) = uuid::Uuid::parse_str(match_id_input) {
        state
            .db
            .get_match_with_odds(&uuid.to_string())
            .await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| bad_request("Match not found"))?
    } else if let Ok(ps_id) = match_id_input.parse::<i64>() {
        let m = state
            .db
            .get_match_by_provider_numeric_id(ps_id)
            .await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| bad_request("Match not found for provider id"))?;
        state
            .db
            .get_match_with_odds(&m.id.to_string())
            .await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| bad_request("Match not found"))?
    } else {
        return Err(bad_request(
            "match_id must be a UUID or a provider integer id",
        ));
    };

    let match_id = match_record.match_info.id;

    // Find the matching opponent row by name (case-insensitive)
    let opponents = state
        .db
        .get_match_opponents(match_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let winner_opponent = opponents
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(winner_name));

    // Upload evidence to Walrus
    let epochs =
        agent_pipeline::epochs_for_pool(match_record.total_pool_usdc, state.walrus.config().epochs);
    let (evidence_blob_id, evidence_url) = if state.walrus.config().configured() {
        match state.walrus.store_json_with_epochs(evidence, epochs).await {
            Ok(stored) => {
                let artifact_req = CreateWalrusArtifactRequest {
                    artifact_type: "webhook_evidence".to_string(),
                    owner_wallet: None,
                    match_id: Some(match_id.to_string()),
                    outcome_proposal_id: None,
                    content_type: Some("application/json".to_string()),
                    manifest: evidence.clone(),
                    metadata: Some(serde_json::json!({ "source": source_label, "epochs": epochs })),
                };
                let _ = state
                    .db
                    .create_walrus_artifact(&artifact_req, &stored)
                    .await;
                (stored.blob_id.into(), stored.aggregator_url)
            }
            Err(e) => {
                tracing::warn!("Walrus upload failed in webhook: {}", e);
                (None, result_url.map(ToString::to_string))
            }
        }
    } else {
        (None, result_url.map(ToString::to_string))
    };

    let (verification_status, verification_note) =
        agent_pipeline::cross_check_provider(state, &match_record.match_info, winner_name).await;

    // Confidence threshold gate
    let confidence_f64 = confidence
        .map(|c| c.to_string().parse::<f64>().unwrap_or(0.0))
        .unwrap_or(1.0);

    let final_status = agent_pipeline::gate_status(
        &verification_status,
        confidence_f64,
        agent_pipeline::min_auto_accept_confidence(),
    );

    let proposal_req = CreateOutcomeProposalRequest {
        proposer_wallet: None,
        proposed_winner_opponent_id: winner_opponent.map(|o| o.id.to_string()),
        proposed_winner_name: Some(winner_name.to_string()),
        source: Some(source_label.to_string()),
        confidence,
        evidence_blob_id: evidence_blob_id.clone(),
        evidence_url: evidence_url.clone(),
        evidence_summary: summary.map(ToString::to_string),
        raw_data: Some(evidence.clone()),
    };

    let proposal = state
        .db
        .create_outcome_proposal(match_id, &proposal_req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    state
        .db
        .apply_agent_verification(proposal.id, match_id, source_label, &final_status)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Synthetic request to reuse the agent run creation path
    let agent_req = AgentOutcomeProposalRequest {
        match_id: match_id.to_string(),
        agent_name: Some(source_label.to_string()),
        agent_id: Some(source_label.to_string()),
        watch_sources: None,
        proposed_winner_opponent_id: winner_opponent.map(|o| o.id.to_string()),
        proposed_winner_name: Some(winner_name.to_string()),
        confidence,
        evidence_blob_id: evidence_blob_id.clone(),
        evidence_url: evidence_url.clone(),
        evidence_summary: summary.map(ToString::to_string),
        raw_output: Some(evidence.clone()),
    };

    state
        .db
        .create_agent_run_for_proposal(
            match_id,
            &agent_req,
            proposal.id,
            Some(source_label),
            evidence_blob_id.as_deref(),
            evidence_url.as_deref(),
            Some(&final_status),
            Some(&verification_note),
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "proposal_id": proposal.id,
        "match_id": match_id,
        "verification_status": final_status,
        "verification_note": verification_note,
        "evidence_blob_id": evidence_blob_id,
        "winner_name": winner_name,
    }))))
}

// ─── Auth helpers ─────────────────────────────────────────────────────────────

/// Verify HMAC-SHA256 signature: `X-Webhook-Signature: <hex>`
/// Secret is `WEBHOOK_SECRET` env var.
fn verify_webhook_signature(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let secret = std::env::var("WEBHOOK_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            service_unavailable("match-result webhook is not configured (WEBHOOK_SECRET unset)")
        })?;

    let provided = headers
        .get("x-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized("missing X-Webhook-Signature header"))?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| internal_error("HMAC key error"))?;
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    // Strip optional "sha256=" prefix that some platforms add
    let provided_hex = provided.strip_prefix("sha256=").unwrap_or(provided);
    if !constant_time_eq(provided_hex.as_bytes(), expected.as_bytes()) {
        return Err(unauthorized("invalid webhook signature"));
    }
    Ok(())
}

/// Constant-time byte comparison to prevent timing attacks on token checks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_match() {
        let raw = serde_json::json!({
            "id": 123,
            "opponents": [{ "opponent": { "id": 1, "name": "A" } }]
        });
        let found = extract_match_object(&raw).expect("should find match");
        assert_eq!(found.get("id").and_then(|v| v.as_i64()), Some(123));
    }

    #[test]
    fn extracts_nested_match() {
        let raw = serde_json::json!({
            "type": "match",
            "match": {
                "id": 456,
                "opponents": [{ "opponent": { "id": 2, "name": "B" } }]
            }
        });
        let found = extract_match_object(&raw).expect("should find nested match");
        assert_eq!(found.get("id").and_then(|v| v.as_i64()), Some(456));
    }

    #[test]
    fn falls_back_to_object_with_id() {
        // No opponents yet (TBD bracket slot) but still a match payload.
        let raw = serde_json::json!({ "id": 789, "status": "not_started" });
        let found = extract_match_object(&raw).expect("should fall back to id");
        assert_eq!(found.get("id").and_then(|v| v.as_i64()), Some(789));
    }

    #[test]
    fn rejects_non_match_payload() {
        let raw = serde_json::json!({ "ping": "pong" });
        assert!(extract_match_object(&raw).is_none());
    }
}
