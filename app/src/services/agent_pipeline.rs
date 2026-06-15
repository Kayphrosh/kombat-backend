//! Shared logic for the outcome-proposal pipeline used by the agent submission
//! endpoint, the result webhooks, and the PandaScore poller.
//!
//! These helpers were previously duplicated across `handlers/agent.rs`,
//! `handlers/webhook.rs`, and `services/poller.rs`. Centralising them keeps the
//! evidence rules, epoch pinning, and PandaScore cross-check consistent.

use serde_json::Value;

use crate::{services::pandascore::result_from_match, state::AppState};

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

/// Cross-check a proposed winner against PandaScore's reported result.
/// Returns `(verification_status, note)` where status is either
/// `"auto_verified"` (PandaScore confirms the winner) or `"pending_review"`.
pub async fn cross_check_pandascore(
    state: &AppState,
    match_info: &crate::models::MatchRecord,
    winner_name: &str,
) -> (String, String) {
    let pandascore_id = match match_info.pandascore_id {
        Some(id) => id,
        None => {
            return (
                "pending_review".to_string(),
                "No PandaScore ID on match; manual review required".to_string(),
            )
        }
    };

    if !state.pandascore.config().configured() {
        return (
            "pending_review".to_string(),
            "PandaScore not configured; manual review required".to_string(),
        );
    }

    let raw_match = match state.pandascore.fetch_match_by_id(pandascore_id).await {
        Ok(m) => m,
        Err(e) => {
            return (
                "pending_review".to_string(),
                format!("PandaScore lookup failed: {}; manual review required", e),
            )
        }
    };

    let ps_result = result_from_match(&raw_match);

    if !ps_result.finished {
        return (
            "pending_review".to_string(),
            "PandaScore reports match not yet finished".to_string(),
        );
    }

    let ps_winner_name = ps_result.winner_name.as_deref().unwrap_or("");
    if ps_winner_name.is_empty() {
        return (
            "pending_review".to_string(),
            "PandaScore returned no winner; manual review required".to_string(),
        );
    }

    if !winner_name.is_empty() && winner_name.eq_ignore_ascii_case(ps_winner_name) {
        (
            "auto_verified".to_string(),
            format!("PandaScore confirmed winner: {}", ps_winner_name),
        )
    } else {
        (
            "pending_review".to_string(),
            format!(
                "PandaScore winner '{}' does not match proposed '{}'; manual review required",
                ps_winner_name, winner_name
            ),
        )
    }
}
