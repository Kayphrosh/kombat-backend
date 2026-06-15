use std::sync::Arc;
use tokio::time::{interval, Duration};

use crate::{
    models::{
        AgentOutcomeProposalRequest, CreateOutcomeProposalRequest, CreateWalrusArtifactRequest,
    },
    services::{agent_pipeline, pandascore::result_from_match},
    state::AppState,
};

/// Configuration read from environment variables.
#[derive(Debug, Clone)]
pub struct PollerConfig {
    /// How often to poll PandaScore for finished matches (seconds).
    pub interval_secs: u64,
    /// How far back to look for matches that may have finished (hours).
    pub lookback_hours: i64,
    /// Only run the poller if this is true.
    pub enabled: bool,
}

impl PollerConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("POLLER_ENABLED")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
            .unwrap_or(true);
        let interval_secs = std::env::var("POLLER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        let lookback_hours = std::env::var("POLLER_LOOKBACK_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6);
        Self {
            enabled,
            interval_secs,
            lookback_hours,
        }
    }
}

/// Spawn a background task that polls PandaScore for match results.
/// Returns immediately; the task runs until the process exits.
pub fn spawn(state: Arc<AppState>, config: PollerConfig) {
    if !config.enabled {
        tracing::info!("Match result poller is disabled");
        return;
    }
    tokio::spawn(async move {
        run(state, config).await;
    });
}

async fn run(state: Arc<AppState>, config: PollerConfig) {
    tracing::info!(
        interval_secs = config.interval_secs,
        lookback_hours = config.lookback_hours,
        "Match result poller started"
    );
    let mut ticker = interval(Duration::from_secs(config.interval_secs));
    // First tick fires immediately — skip it so we don't poll on startup
    // before the server is ready.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        if let Err(e) = poll_once(&state, &config).await {
            tracing::warn!("Match result poller error: {}", e);
        }
    }
}

async fn poll_once(state: &AppState, config: &PollerConfig) -> anyhow::Result<()> {
    if !state.pandascore.config().configured() {
        return Ok(());
    }

    // Fetch matches from our DB that are still open but whose scheduled time
    // has passed — these are candidates to check for a PandaScore result.
    let candidates = state.db.get_pollable_matches(config.lookback_hours).await?;

    if candidates.is_empty() {
        return Ok(());
    }

    tracing::debug!("Poller checking {} candidate match(es)", candidates.len());

    for m in candidates {
        let pandascore_id = match m.pandascore_id {
            Some(id) => id,
            None => continue,
        };

        let raw_match = match state.pandascore.fetch_match_by_id(pandascore_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    match_id = %m.id,
                    pandascore_id,
                    "PandaScore fetch failed: {}",
                    e
                );
                continue;
            }
        };

        let ps_result = result_from_match(&raw_match);
        if !ps_result.finished {
            continue;
        }

        let winner_name = match ps_result.winner_name.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                tracing::debug!(match_id = %m.id, "PandaScore match finished but no winner yet");
                continue;
            }
        };

        tracing::info!(
            match_id = %m.id,
            pandascore_id,
            winner = %winner_name,
            "Poller found finished match — creating outcome proposal"
        );

        if let Err(e) = process_result(state, &m, &winner_name, &raw_match).await {
            tracing::warn!(match_id = %m.id, "Failed to process polled result: {}", e);
        }
    }

    Ok(())
}

async fn process_result(
    state: &AppState,
    match_info: &crate::models::MatchRecord,
    winner_name: &str,
    raw_match: &serde_json::Value,
) -> anyhow::Result<()> {
    let match_id = match_info.id;

    // Build structured evidence from the raw PandaScore payload
    let evidence = serde_json::json!({
        "match_id": match_info.pandascore_id.map(|id| id.to_string()).unwrap_or_else(|| match_id.to_string()),
        "winner": winner_name,
        "source_data": raw_match,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    // Get pool size for epoch pinning
    let match_with_odds = state
        .db
        .get_match_with_odds(&match_id.to_string())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Match not found"))?;

    let epochs = agent_pipeline::epochs_for_pool(
        match_with_odds.total_pool_usdc,
        state.walrus.config().epochs,
    );

    let (evidence_blob_id, evidence_url) = if state.walrus.config().configured() {
        match state.walrus.store_json_with_epochs(&evidence, epochs).await {
            Ok(stored) => {
                let artifact_req = CreateWalrusArtifactRequest {
                    artifact_type: "poller_evidence".to_string(),
                    owner_wallet: None,
                    match_id: Some(match_id.to_string()),
                    outcome_proposal_id: None,
                    content_type: Some("application/json".to_string()),
                    manifest: evidence.clone(),
                    metadata: Some(serde_json::json!({ "source": "poller", "epochs": epochs })),
                };
                let _ = state
                    .db
                    .create_walrus_artifact(&artifact_req, &stored)
                    .await;
                (Some(stored.blob_id), stored.aggregator_url)
            }
            Err(e) => {
                tracing::warn!("Walrus upload failed in poller: {}", e);
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // Find the winner opponent row
    let opponents = state.db.get_match_opponents(match_id).await?;
    let winner_opponent = opponents
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(winner_name));

    // PandaScore is the source of truth here, so confidence is 1.0. The gate
    // still applies: setting AGENT_MIN_AUTO_ACCEPT_CONFIDENCE above 1.0 forces
    // every polled result to manual review.
    let verification_note = format!("PandaScore poller confirmed winner: {}", winner_name);
    let final_status = agent_pipeline::gate_status(
        "auto_verified",
        1.0,
        agent_pipeline::min_auto_accept_confidence(),
    );

    let proposal_req = CreateOutcomeProposalRequest {
        proposer_wallet: None,
        proposed_winner_opponent_id: winner_opponent.map(|o| o.id.to_string()),
        proposed_winner_name: Some(winner_name.to_string()),
        source: Some("pandascore_poller".to_string()),
        confidence: Some(rust_decimal::Decimal::ONE),
        evidence_blob_id: evidence_blob_id.clone(),
        evidence_url: evidence_url.clone(),
        evidence_summary: Some(format!("PandaScore result: {} wins", winner_name)),
        raw_data: Some(evidence.clone()),
    };

    let proposal = state
        .db
        .create_outcome_proposal(match_id, &proposal_req)
        .await?;

    state
        .db
        .apply_agent_verification(proposal.id, match_id, "pandascore_poller", &final_status)
        .await?;

    let agent_req = AgentOutcomeProposalRequest {
        match_id: match_id.to_string(),
        agent_name: Some("pandascore_poller".to_string()),
        agent_id: Some("pandascore_poller".to_string()),
        watch_sources: None,
        proposed_winner_opponent_id: winner_opponent.map(|o| o.id.to_string()),
        proposed_winner_name: Some(winner_name.to_string()),
        confidence: Some(rust_decimal::Decimal::ONE),
        evidence_blob_id: evidence_blob_id.clone(),
        evidence_url: evidence_url.clone(),
        evidence_summary: Some(format!("PandaScore result: {} wins", winner_name)),
        raw_output: Some(evidence),
    };

    state
        .db
        .create_agent_run_for_proposal(
            match_id,
            &agent_req,
            proposal.id,
            Some("pandascore_poller"),
            evidence_blob_id.as_deref(),
            evidence_url.as_deref(),
            Some(&final_status),
            Some(&verification_note),
        )
        .await?;

    Ok(())
}
