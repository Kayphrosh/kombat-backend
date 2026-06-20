//! Shared logic for the outcome-proposal pipeline used by the agent submission
//! endpoint and result webhooks.
//!
//! These helpers were previously duplicated across handlers. Centralising them
//! keeps the evidence rules, epoch pinning, and verification gates consistent.

use serde_json::Value;

use crate::models::CreateWalrusArtifactRequest;
use crate::services::walrus::WalrusStoredBlob;
use crate::state::AppState;

/// Best-effort: store a JSON manifest on Walrus and persist an artifact row.
///
/// This is the single entry point every feature uses to push durable,
/// verifiable records to Walrus (settlement proofs, provider snapshots, agent
/// audit logs, marketplace snapshots, organizer media manifests). It never
/// fails the caller's primary operation — if Walrus is disabled or the upload
/// errors, it logs and returns `None`.
pub async fn archive_to_walrus(
    state: &AppState,
    artifact_type: &str,
    match_id: Option<String>,
    owner_wallet: Option<String>,
    manifest: Value,
    metadata: Option<Value>,
    epochs: u32,
) -> Option<WalrusStoredBlob> {
    if !state.walrus.config().configured() {
        return None;
    }

    let stored = match state.walrus.store_json_with_epochs(&manifest, epochs).await {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!("Walrus archive failed ({}): {}", artifact_type, e);
            return None;
        }
    };

    let artifact_req = CreateWalrusArtifactRequest {
        artifact_type: artifact_type.to_string(),
        owner_wallet,
        match_id,
        outcome_proposal_id: None,
        content_type: Some("application/json".to_string()),
        manifest,
        metadata,
    };
    if let Err(e) = state.db.create_walrus_artifact(&artifact_req, &stored).await {
        // The blob is already durably stored; only the index row failed.
        tracing::warn!("Walrus artifact row failed ({}): {}", artifact_type, e);
    }

    Some(stored)
}

/// Default minimum confidence for a proposal to be auto-accepted without review.
pub const DEFAULT_MIN_AUTO_ACCEPT_CONFIDENCE: f64 = 0.80;

/// Read the configured auto-accept confidence threshold from the environment.
/// A value above 1.0 effectively forces every proposal to manual review.
pub fn min_auto_accept_confidence() -> f64 {
    std::env::var("AGENT_MIN_AUTO_ACCEPT_CONFIDENCE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(DEFAULT_MIN_AUTO_ACCEPT_CONFIDENCE)
}

/// Pick the Walrus epoch count based on match pool size — higher stakes get
/// longer retention. Thresholds are in micro-USDC (6 decimals).
pub fn epochs_for_pool(total_pool_usdc: i64, default_epochs: u32) -> u32 {
    if total_pool_usdc > 1_000_000_000 {
        return (default_epochs * 4).max(20);
    }
    if total_pool_usdc > 100_000_000 {
        return (default_epochs * 2).max(10);
    }
    default_epochs
}

/// Validate that an evidence payload carries the minimum required fields.
/// Returns the offending message on failure so callers can map it to their
/// own error type.
pub fn validate_evidence_schema(raw: &Value) -> Result<(), String> {
    let obj = raw
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| "evidence must be a non-empty JSON object".to_string())?;

    if !obj.contains_key("match_id") {
        return Err("evidence must contain 'match_id'".to_string());
    }
    if !obj.contains_key("winner") {
        return Err("evidence must contain 'winner'".to_string());
    }
    if !obj.contains_key("source_data") && !obj.contains_key("timestamp") {
        return Err("evidence must contain 'source_data' or 'timestamp'".to_string());
    }
    Ok(())
}

/// Apply the confidence gate: an `auto_verified` proposal is downgraded to
/// `pending_review` when the supplied confidence is below the threshold.
/// Any other status passes through unchanged.
pub fn gate_status(verification_status: &str, confidence: f64, min_confidence: f64) -> String {
    if verification_status == "auto_verified" && confidence < min_confidence {
        "pending_review".to_string()
    } else {
        verification_status.to_string()
    }
}

/// Cross-check a proposed winner against the configured provider's reported
/// result. GRID result verification is intentionally conservative until the
/// exact result endpoint/payload is confirmed.
pub async fn cross_check_provider(
    state: &AppState,
    match_info: &crate::models::MatchRecord,
    winner_name: &str,
) -> (String, String) {
    let _ = (state, winner_name);
    if match_info.source == "grid" {
        return (
            "pending_review".to_string(),
            "GRID result cross-check is not configured yet; manual review required".to_string(),
        );
    }

    (
        "pending_review".to_string(),
        format!(
            "Provider '{}' result cross-check is not configured; manual review required",
            match_info.source
        ),
    )
}
