use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use rust_decimal::Decimal;
use std::sync::Arc;

use crate::{
    models::{
        AdminAgentRunQuery, AgentOutcomeProposalRequest, AgentRunRecord, ApiResponse,
        CreateOutcomeProposalRequest, CreateWalrusArtifactRequest,
    },
    services::agent_pipeline,
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

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
}

pub async fn submit_agent_outcome_proposal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<AgentOutcomeProposalRequest>,
) -> AppResult<AgentRunRecord> {
    let agent_id = verify_agent(&headers)?;

    let match_uuid =
        uuid::Uuid::parse_str(&req.match_id).map_err(|_| bad_request("Invalid match_id"))?;
    if req.proposed_winner_opponent_id.is_none() && req.proposed_winner_name.is_none() {
        return Err(bad_request(
            "Provide proposed_winner_opponent_id or proposed_winner_name",
        ));
    }
    if let Some(confidence) = req.confidence {
        if confidence < Decimal::ZERO || confidence > Decimal::ONE {
            return Err(bad_request("confidence must be between 0 and 1"));
        }
    }

    // --- #6: Validate evidence schema before accepting ---
    let raw_output = req.raw_output.as_ref().ok_or_else(|| {
        bad_request("raw_output is required and must include match_id, winner, and source_data or timestamp")
    })?;
    agent_pipeline::validate_evidence_schema(raw_output).map_err(bad_request)?;

    // --- #1 & #3: Upload evidence to Walrus from the backend ---
    // Epoch count scales with pool size: high-stakes matches get more retention.
    let match_with_odds = state
        .db
        .get_match_with_odds(&req.match_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Match not found"))?;

    let epochs = agent_pipeline::epochs_for_pool(
        match_with_odds.total_pool_usdc,
        state.walrus.config().epochs,
    );

    let (evidence_blob_id, evidence_url) = if state.walrus.config().configured() {
        match state
            .walrus
            .store_json_with_epochs(raw_output, epochs)
            .await
        {
            Ok(stored) => {
                // Index the artifact in Postgres
                let artifact_req = CreateWalrusArtifactRequest {
                    artifact_type: "agent_evidence".to_string(),
                    owner_wallet: None,
                    match_id: Some(req.match_id.clone()),
                    outcome_proposal_id: None,
                    content_type: Some("application/json".to_string()),
                    manifest: raw_output.clone(),
                    metadata: Some(serde_json::json!({
                        "agent_id": agent_id,
                        "epochs": epochs,
                    })),
                };
                let _ = state
                    .db
                    .create_walrus_artifact(&artifact_req, &stored)
                    .await;
                (stored.blob_id.into(), stored.aggregator_url)
            }
            Err(e) => {
                tracing::warn!("Walrus upload failed, proceeding without blob: {}", e);
                (None, req.evidence_url.clone())
            }
        }
    } else {
        (None, req.evidence_url.clone())
    };

    // --- #4: PandaScore cross-check ---
    let proposed_winner = req.proposed_winner_name.as_deref().unwrap_or("");
    let (verification_status, verification_note) = agent_pipeline::cross_check_pandascore(
        &state,
        &match_with_odds.match_info,
        proposed_winner,
    )
    .await;

    // --- #2: Confidence threshold gate ---
    let confidence_f64 = req
        .confidence
        .map(|c| c.to_string().parse::<f64>().unwrap_or(0.0))
        .unwrap_or(0.0);

    let final_verification = agent_pipeline::gate_status(
        &verification_status,
        confidence_f64,
        agent_pipeline::min_auto_accept_confidence(),
    );

    let proposal_req = CreateOutcomeProposalRequest {
        proposer_wallet: None,
        proposed_winner_opponent_id: req.proposed_winner_opponent_id.clone(),
        proposed_winner_name: req.proposed_winner_name.clone(),
        source: Some("agent".to_string()),
        confidence: req.confidence,
        evidence_blob_id: evidence_blob_id.clone(),
        evidence_url: evidence_url.clone(),
        evidence_summary: req.evidence_summary.clone(),
        raw_data: Some(raw_output.clone()),
    };

    let proposal = state
        .db
        .create_outcome_proposal(match_uuid, &proposal_req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    state
        .db
        .apply_agent_verification(proposal.id, match_uuid, "agent", &final_verification)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // --- #5: Per-agent identity stored on the run record ---
    let resolved_agent_id = req.agent_id.as_deref().or(agent_id.as_deref());
    let run = state
        .db
        .create_agent_run_for_proposal(
            match_uuid,
            &req,
            proposal.id,
            resolved_agent_id,
            evidence_blob_id.as_deref(),
            evidence_url.as_deref(),
            Some(&final_verification),
            Some(&verification_note),
        )
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(run)))
}

pub async fn list_admin_agent_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminAgentRunQuery>,
) -> AppResult<Vec<AgentRunRecord>> {
    verify_admin(&headers)?;
    let runs = state
        .db
        .list_admin_agent_runs(&query)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(runs)))
}

pub async fn get_admin_agent_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> AppResult<AgentRunRecord> {
    verify_admin(&headers)?;
    let run_uuid = uuid::Uuid::parse_str(&run_id).map_err(|_| bad_request("Invalid run id"))?;
    let run = state
        .db
        .get_agent_run(run_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Agent run not found"))?;

    Ok(Json(ApiResponse::ok(run)))
}

/// Verify the agent token and return the agent id encoded in it, if any.
/// Token format: "<agent-id>:<secret>" or just "<secret>" for backwards
/// compatibility with single-token deployments.
fn verify_agent(
    headers: &HeaderMap,
) -> Result<Option<String>, (StatusCode, Json<ApiResponse<()>>)> {
    let agent_token = std::env::var("AGENT_API_TOKEN")
        .map_err(|_| internal_error("server missing AGENT_API_TOKEN"))?;
    let got = bearer_or_header(headers, "x-agent-token")
        .ok_or_else(|| unauthorized("missing agent token"))?;

    // Support "<agent-id>:<secret>" so individual agents can be identified
    // without changing the single shared secret for existing deployments.
    if let Some((id, secret)) = got.split_once(':') {
        if secret == agent_token {
            return Ok(Some(id.to_string()));
        }
    }

    if got != agent_token {
        return Err(unauthorized("invalid agent token"));
    }
    Ok(None)
}

fn verify_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let admin_token = std::env::var("AUTH_ADMIN_TOKEN")
        .map_err(|_| internal_error("server missing AUTH_ADMIN_TOKEN"))?;
    let got = bearer_or_header(headers, "x-admin-token");
    if got.as_deref() != Some(admin_token.as_str()) {
        return Err(unauthorized("invalid admin token"));
    }
    Ok(())
}

fn bearer_or_header(headers: &HeaderMap, header_name: &str) -> Option<String> {
    headers
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer ").map(ToString::to_string))
        })
}
