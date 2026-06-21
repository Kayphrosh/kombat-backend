use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;

use crate::{
    handlers::notify::{notification_action, notification_payload, notify_user_best_effort},
    models::*,
    services::{auth::verify_jwt_get_wallet, sui::SuiService},
    state::AppState,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
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

/// Extract wallet from JWT in Authorization header
fn extract_wallet_from_jwt(
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ApiResponse<()>>)> {
    let secret = std::env::var("AUTH_JWT_SECRET")
        .map_err(|_| internal_error("server missing AUTH_JWT_SECRET"))?;

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;

    verify_jwt_get_wallet(token, &secret).map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

/// Verify admin token from X-Admin-Token header or Authorization: Bearer
fn verify_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let admin_token = std::env::var("AUTH_ADMIN_TOKEN")
        .map_err(|_| internal_error("server missing AUTH_ADMIN_TOKEN"))?;

    let got = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()))
        });

    if got.as_deref() != Some(admin_token.as_str()) {
        return Err(unauthorized("invalid admin token"));
    }
    Ok(())
}

// ─── GET /api/tournaments ─────────────────────────────────────────────────────
/// List tournaments (matches) with filtering
pub async fn list_tournaments(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MatchListQuery>,
) -> AppResult<Vec<MatchWithOdds>> {
    let mut matches = state
        .db
        .list_matches(&query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    hydrate_matches_from_chain(&state, &mut matches).await;

    Ok(Json(ApiResponse::ok(matches)))
}

pub async fn get_tournament_source_grid(
    State(state): State<Arc<AppState>>,
) -> AppResult<GridSourceResponse> {
    let config = state.grid.config();
    Ok(Json(ApiResponse::ok(GridSourceResponse {
        provider: "grid".to_string(),
        enabled: config.enabled,
        configured: config.configured(),
        base_url: config.base_url.clone(),
        matches_path: config.matches_path.clone(),
        auth_header: config.auth_header.clone(),
        api_style: config.api_style.clone(),
        default_statuses: config.default_statuses.clone(),
        default_videogame_slugs: config.default_videogame_slugs.clone(),
        default_per_page: config.default_per_page,
        default_max_pages: config.default_max_pages,
    })))
}

pub async fn probe_grid_tournaments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<GridSyncRequest>,
) -> AppResult<GridProbeResponse> {
    verify_admin(&headers)?;

    let probe = state
        .grid
        .probe_matches(&req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(probe)))
}

pub async fn sync_grid_tournaments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<GridSyncRequest>,
) -> AppResult<GridSyncResponse> {
    verify_admin(&headers)?;

    let fetched = state
        .grid
        .fetch_matches(&req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let mut synced = 0usize;
    let mut synced_incomplete = 0usize;
    let skipped = 0usize;
    let mut resolved = 0usize;
    let mut errors = Vec::new();

    for match_req in fetched.iter() {
        let match_record = match state.db.upsert_match(match_req).await {
            Ok(record) => record,
            Err(e) => {
                errors.push(format!(
                    "failed to sync GRID match {}: {}",
                    match_req.pandascore_id, e
                ));
                continue;
            }
        };
        synced += 1;
        if match_req.opponents.len() != 2 {
            synced_incomplete += 1;
        }

        if match_req.pandascore_status.as_deref() == Some("finished") {
            match resolve_finished_provider_match(&state, &match_record.id.to_string(), match_req)
                .await
            {
                Ok(true) => resolved += 1,
                Ok(false) => {}
                Err(e) => errors.push(format!(
                    "failed to resolve GRID match {}: {}",
                    match_req.pandascore_id, e
                )),
            }
        }
    }

    Ok(Json(ApiResponse::ok(GridSyncResponse {
        provider: "grid".to_string(),
        fetched: fetched.len(),
        synced,
        synced_incomplete,
        skipped,
        resolved,
        errors,
    })))
}

// ─── PandaScore source handlers ───────────────────────────────────────────────

pub async fn get_tournament_source_pandascore(
    State(state): State<Arc<AppState>>,
) -> AppResult<PandascoreSourceResponse> {
    let config = state.pandascore.config();
    Ok(Json(ApiResponse::ok(PandascoreSourceResponse {
        provider: "pandascore".to_string(),
        enabled: config.enabled,
        configured: config.configured(),
        base_url: config.base_url.clone(),
        default_statuses: config.default_statuses.clone(),
        default_videogame_slugs: config.default_videogame_slugs.clone(),
        default_per_page: config.default_per_page,
        default_max_pages: config.default_max_pages,
    })))
}

pub async fn probe_pandascore_tournaments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PandascoreSyncRequest>,
) -> AppResult<PandascoreProbeResponse> {
    verify_admin(&headers)?;
    let probe = state
        .pandascore
        .probe_matches(&req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(ApiResponse::ok(probe)))
}

pub async fn sync_pandascore_tournaments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PandascoreSyncRequest>,
) -> AppResult<PandascoreSyncResponse> {
    verify_admin(&headers)?;

    let response = run_pandascore_sync(&state, &req)
        .await
        .map_err(|e| bad_request(e))?;

    Ok(Json(ApiResponse::ok(response)))
}

/// Core PandaScore sync cycle: fetch matches, upsert them, and auto-resolve any
/// finished ones. Shared by the admin HTTP handler and the background scheduler.
pub async fn run_pandascore_sync(
    state: &Arc<AppState>,
    req: &PandascoreSyncRequest,
) -> Result<PandascoreSyncResponse, String> {
    let fetched = state
        .pandascore
        .fetch_matches(req)
        .await
        .map_err(|e| e.to_string())?;

    let mut synced = 0usize;
    let mut synced_incomplete = 0usize;
    let skipped = 0usize;
    let mut resolved = 0usize;
    let mut errors = Vec::new();

    for match_req in fetched.iter() {
        let match_record = match state.db.upsert_match(match_req).await {
            Ok(record) => record,
            Err(e) => {
                errors.push(format!(
                    "failed to sync PandaScore match {}: {}",
                    match_req.pandascore_id, e
                ));
                continue;
            }
        };
        synced += 1;
        if match_req.opponents.len() != 2 {
            synced_incomplete += 1;
        }

        if match_req.pandascore_status.as_deref() == Some("finished") {
            match resolve_finished_provider_match(state, &match_record.id.to_string(), match_req)
                .await
            {
                Ok(true) => resolved += 1,
                Ok(false) => {}
                Err(e) => errors.push(format!(
                    "failed to resolve PandaScore match {}: {}",
                    match_req.pandascore_id, e
                )),
            }
        }
    }

    Ok(PandascoreSyncResponse {
        provider: "pandascore".to_string(),
        fetched: fetched.len(),
        synced,
        synced_incomplete,
        skipped,
        resolved,
        errors,
    })
}

// ─── POST /api/tournaments ────────────────────────────────────────────────────
/// Create or sync a tournament from provider-shaped data.
/// Server-side GRID sync should be preferred; this route remains useful for
/// admin backfills and local development.
pub async fn create_tournament(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateMatchRequest>,
) -> AppResult<MatchWithOdds> {
    verify_admin(&headers)?;

    // Validate we have exactly 2 opponents for binary betting
    if req.opponents.len() != 2 {
        return Err(bad_request("Exactly 2 opponents required for betting"));
    }
    if req.sui_pool_object_id.is_some() || req.sui_network.is_some() {
        if let Some(pool_object_id) = req.sui_pool_object_id.as_ref() {
            req.sui_pool_object_id = Some(
                SuiService::normalize_address(pool_object_id)
                    .ok_or_else(|| bad_request("Invalid Sui pool object id"))?,
            );
        }
        if let Some(network) = req.sui_network.as_ref() {
            req.sui_network = Some(
                state
                    .sui
                    .config()
                    .network(network)
                    .ok_or_else(|| bad_request("Unsupported Sui network"))?
                    .network
                    .clone(),
            );
        }
    }

    // Create or update match. Pool metadata is preserved unless the request
    // carries a newly indexed Sui pool object.
    let match_record = state
        .db
        .upsert_match(&req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Get with full odds data
    let match_with_odds = state
        .db
        .get_match_with_odds(&match_record.id.to_string())
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| internal_error("Match not found after creation"))?;

    tracing::info!(
        "Tournament synced: {} (provider ID: {})",
        req.name,
        req.pandascore_id
    );

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

async fn resolve_finished_provider_match(
    state: &Arc<AppState>,
    match_id: &str,
    req: &CreateMatchRequest,
) -> Result<bool, String> {
    let Some(raw_data) = &req.raw_data else {
        return Ok(false);
    };
    let Some(winner_id) = raw_data.get("winner_id").and_then(|value| value.as_i64()) else {
        return Ok(false);
    };

    let match_data = state
        .db
        .get_match_with_odds(match_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "match not found after provider sync".to_string())?;

    let Some(winner) = match_data
        .opponents
        .iter()
        .find(|opponent| opponent.opponent.pandascore_id == winner_id as i32)
    else {
        return Ok(false);
    };

    let forfeit = raw_data
        .get("forfeit")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let result = state
        .db
        .resolve_match(
            match_id,
            &winner.opponent.id.to_string(),
            Some(winner_id as i32),
            forfeit,
        )
        .await
        .map_err(|e| e.to_string())?;
    notify_tournament_resolution(state, match_id, &result).await;

    let resolved = !matches!(result, ResolveResult::Empty);

    // Proof-of-settlement: archive an immutable, third-party-verifiable record of
    // exactly what provider data we settled on. Best-effort — never blocks the
    // resolution itself.
    if resolved {
        let manifest = serde_json::json!({
            "record_type": "settlement_proof",
            "match_id": match_id,
            "match_name": match_data.match_info.name,
            "source": req.source.clone().unwrap_or_else(|| "provider".to_string()),
            "provider_match_id": req.pandascore_id,
            "winner_opponent_id": winner.opponent.id.to_string(),
            "winner_name": winner.opponent.name,
            "winner_provider_id": winner_id,
            "forfeit": forfeit,
            "results": raw_data.get("results").cloned(),
            "scheduled_at": match_data.match_info.scheduled_at,
            "resolved_at": chrono::Utc::now().to_rfc3339(),
            "sui_pool_object_id": match_data.match_info.sui_pool_object_id,
            "total_pool_usdc": match_data.total_pool_usdc,
            "provider_payload": raw_data,
        });
        let epochs = crate::services::agent_pipeline::epochs_for_pool(
            match_data.total_pool_usdc,
            state.walrus.config().epochs,
        );
        if let Some(stored) = crate::services::agent_pipeline::archive_to_walrus(
            state,
            "settlement_proof",
            Some(match_id.to_string()),
            None,
            manifest,
            Some(serde_json::json!({ "source": req.source, "winner": winner.opponent.name })),
            epochs,
        )
        .await
        {
            tracing::info!(
                "Settlement proof archived for match {}: blob {}",
                match_id,
                stored.blob_id
            );
        }
    }

    Ok(resolved)
}

// ─── GET /api/tournaments/:id ─────────────────────────────────────────────────
/// Get a single tournament with odds
pub async fn get_tournament(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<MatchWithOdds> {
    let mut match_with_odds = state
        .db
        .get_match_with_odds(&id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;
    hydrate_match_from_chain(&state, &mut match_with_odds).await;

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

pub async fn configure_tournament_pool(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ConfigureMatchPoolRequest>,
) -> AppResult<MatchWithOdds> {
    verify_admin(&headers)?;

    let match_uuid =
        uuid::Uuid::parse_str(&id).map_err(|_| bad_request("Invalid tournament id"))?;
    let pool_object_id = SuiService::normalize_address(&req.sui_pool_object_id)
        .ok_or_else(|| bad_request("Invalid Sui pool object id"))?;
    let sui_network = req
        .sui_network
        .unwrap_or_else(|| state.sui.config().active_network.clone());
    let network_config = state
        .sui
        .config()
        .network(&sui_network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;

    let match_record = state
        .db
        .configure_match_pool(match_uuid, Some(&network_config.network), &pool_object_id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let match_with_odds = state
        .db
        .get_match_with_odds(&match_record.id.to_string())
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| internal_error("Match not found after pool configuration"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

pub async fn backfill_tournament_pools(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<BackfillMatchPoolsRequest>,
) -> AppResult<MatchPoolBackfillResponse> {
    verify_admin(&headers)?;

    let response = run_pool_backfill(
        &state,
        req.sui_network,
        req.match_ids,
        req.limit,
        req.default_stake_window_hours,
    )
    .await
    .map_err(|e| bad_request(e))?;

    Ok(Json(ApiResponse::ok(response)))
}

/// Core pool-backfill cycle: finds matches with two known opponents but no
/// on-chain pool, creates a `tournament_staking` pool for each, and records the
/// pool object id. Shared by the admin HTTP handler and the background scheduler.
pub async fn run_pool_backfill(
    state: &Arc<AppState>,
    sui_network: Option<String>,
    match_ids: Option<Vec<uuid::Uuid>>,
    limit: Option<i64>,
    default_stake_window_hours: Option<i64>,
) -> Result<MatchPoolBackfillResponse, String> {
    let network = sui_network.unwrap_or_else(|| state.sui.config().active_network.clone());
    let network_config = state
        .sui
        .config()
        .network(&network)
        .ok_or_else(|| "Unsupported Sui network".to_string())?;
    let network = network_config.network.clone();
    let limit = limit.unwrap_or(25);
    let default_stake_window_hours = default_stake_window_hours.unwrap_or(72).clamp(1, 720);

    let matches = state
        .db
        .list_matches_missing_pool(match_ids.as_deref(), limit)
        .await
        .map_err(|e| e.to_string())?;

    let mut entries = Vec::with_capacity(matches.len());
    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for match_record in matches {
        let opponents = match state.db.get_match_opponents(match_record.id).await {
            Ok(opponents) => opponents,
            Err(e) => {
                failed += 1;
                entries.push(MatchPoolBackfillEntry {
                    match_id: match_record.id,
                    match_name: match_record.name,
                    status: match_record.status,
                    created: false,
                    pool_object_id: None,
                    tx_digest: None,
                    reason: Some(format!("failed_to_load_opponents: {}", e)),
                });
                continue;
            }
        };

        if opponents.len() != 2 {
            skipped += 1;
            entries.push(MatchPoolBackfillEntry {
                match_id: match_record.id,
                match_name: match_record.name,
                status: match_record.status,
                created: false,
                pool_object_id: None,
                tx_digest: None,
                reason: Some("expected_exactly_two_opponents".to_string()),
            });
            continue;
        }

        // Capture provider provenance before `match_record` fields are moved
        // into the result entry below.
        let prov_match_id = match_record.id;
        let prov_name = match_record.name.clone();
        let prov_source = match_record.source.clone();
        let prov_raw = match_record.raw_data.clone();
        let prov_scheduled = match_record.scheduled_at;
        let prov_team_a = opponents[0].name.clone();
        let prov_team_b = opponents[1].name.clone();

        let stake_deadline_ms = pool_stake_deadline_ms(&match_record, default_stake_window_hours);

        match state
            .sui
            .create_tournament_pool_on_chain(
                &network,
                &match_record.id.to_string(),
                &opponents[0].name,
                &opponents[1].name,
                stake_deadline_ms,
            )
            .await
        {
            Ok(pool) => {
                let pool_object_id = pool.pool_object_id.clone();
                match state
                    .db
                    .configure_match_pool(match_record.id, Some(&network), &pool_object_id)
                    .await
                {
                    Ok(_) => {
                        created += 1;

                        // Provenance: archive the source data the market opened on,
                        // so the pool's basis is verifiable and provider-immutable.
                        let manifest = serde_json::json!({
                            "record_type": "market_open_snapshot",
                            "match_id": prov_match_id.to_string(),
                            "match_name": prov_name,
                            "source": prov_source,
                            "teams": [prov_team_a, prov_team_b],
                            "scheduled_at": prov_scheduled,
                            "sui_pool_object_id": pool_object_id.clone(),
                            "sui_network": network.clone(),
                            "pool_created_at": chrono::Utc::now().to_rfc3339(),
                            "provider_payload": prov_raw,
                        });
                        crate::services::agent_pipeline::archive_to_walrus(
                            state,
                            "market_open_snapshot",
                            Some(prov_match_id.to_string()),
                            None,
                            manifest,
                            None,
                            state.walrus.config().epochs,
                        )
                        .await;

                        entries.push(MatchPoolBackfillEntry {
                            match_id: match_record.id,
                            match_name: match_record.name,
                            status: match_record.status,
                            created: true,
                            pool_object_id: Some(pool_object_id),
                            tx_digest: Some(pool.digest),
                            reason: None,
                        });
                    }
                    Err(e) => {
                        failed += 1;
                        entries.push(MatchPoolBackfillEntry {
                            match_id: match_record.id,
                            match_name: match_record.name,
                            status: match_record.status,
                            created: false,
                            pool_object_id: Some(pool_object_id),
                            tx_digest: Some(pool.digest),
                            reason: Some(format!("created_on_chain_but_db_update_failed: {}", e)),
                        });
                    }
                }
            }
            Err(e) => {
                failed += 1;
                entries.push(MatchPoolBackfillEntry {
                    match_id: match_record.id,
                    match_name: match_record.name,
                    status: match_record.status,
                    created: false,
                    pool_object_id: None,
                    tx_digest: None,
                    reason: Some(e.to_string()),
                });
            }
        }
    }

    Ok(MatchPoolBackfillResponse {
        network,
        attempted: entries.len(),
        created,
        skipped,
        failed,
        entries,
    })
}

/// Ingest a single PandaScore match delivered by the realtime webhook: upsert
/// it (advancing status/score/schedule instantly), resolve it if finished, and
/// create an on-chain pool if it just became complete and auto-backfill is on.
pub async fn ingest_pandascore_match(
    state: &Arc<AppState>,
    match_req: &CreateMatchRequest,
) -> Result<serde_json::Value, String> {
    let record = state
        .db
        .upsert_match(match_req)
        .await
        .map_err(|e| e.to_string())?;

    let is_finished = match_req.pandascore_status.as_deref() == Some("finished");
    let mut resolved = false;
    if is_finished {
        resolved = resolve_finished_provider_match(state, &record.id.to_string(), match_req)
            .await
            .unwrap_or(false);
    }

    let auto_backfill = std::env::var("POOL_AUTO_BACKFILL")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    let mut pool_created = false;
    if auto_backfill
        && !is_finished
        && match_req.opponents.len() == 2
        && record.sui_pool_object_id.is_none()
    {
        match run_pool_backfill(state, None, Some(vec![record.id]), Some(1), None).await {
            Ok(resp) => pool_created = resp.created > 0,
            Err(e) => tracing::warn!("webhook pool backfill failed for {}: {}", record.id, e),
        }
    }

    Ok(serde_json::json!({
        "match_id": record.id,
        "status": record.status,
        "resolved": resolved,
        "pool_created": pool_created,
    }))
}

pub async fn sync_tournament_stakes(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SyncMatchStakesRequest>,
) -> AppResult<SyncMatchStakesResponse> {
    verify_admin(&headers)?;
    let response = run_stake_sync(&state, &id, req.sui_network, req.limit)
        .await
        .map_err(|e| bad_request(e))?;
    Ok(Json(ApiResponse::ok(response)))
}

/// Index on-chain stake events for a single match into `pool_stakes`, so DB
/// aggregates (total_stakers, per-side pools) reflect on-chain reality. The PTB
/// staking flow settles on-chain without hitting the DB, so this reconciliation
/// is what keeps the counts correct. Shared by the admin endpoint and the
/// background reconciler. Idempotent — already-indexed stakes are skipped.
pub async fn run_stake_sync(
    state: &Arc<AppState>,
    id: &str,
    sui_network: Option<String>,
    limit: Option<usize>,
) -> Result<SyncMatchStakesResponse, String> {
    let mut match_with_odds = state
        .db
        .get_match_with_odds(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Tournament not found".to_string())?;
    let pool_object_id = match_with_odds
        .match_info
        .sui_pool_object_id
        .clone()
        .ok_or_else(|| "Tournament pool is not configured".to_string())?;
    let network = sui_network
        .or_else(|| match_with_odds.match_info.sui_network.clone())
        .unwrap_or_else(|| state.sui.config().active_network.clone());
    let network_config = state
        .sui
        .config()
        .network(&network)
        .ok_or_else(|| "Unsupported Sui network".to_string())?;
    let network = network_config.network.clone();

    let events = state
        .sui
        .stake_events_for_pool(&network, &pool_object_id, limit.unwrap_or(50))
        .await
        .map_err(|e| e.to_string())?;

    let mut entries = Vec::with_capacity(events.len());
    let mut indexed = 0usize;
    let mut skipped = 0usize;

    for event in events
        .into_iter()
        .filter(|event| event.match_id == match_with_odds.match_info.id.to_string())
    {
        let opponent = match_with_odds
            .opponents
            .iter()
            .find(|opponent| opponent.opponent.position + 1 == event.outcome as i16);
        let Some(opponent) = opponent else {
            skipped += 1;
            entries.push(SyncMatchStakeEntry {
                tx_digest: event.tx_digest,
                receipt_id: event.receipt_id,
                owner: event.owner,
                opponent_id: None,
                outcome: event.outcome,
                amount_usdc: event.amount,
                indexed: false,
                reason: Some("opponent_not_found_for_outcome".to_string()),
            });
            continue;
        };

        let odds = event_odds(&event);
        match state
            .db
            .record_indexed_pool_stake(
                match_with_odds.match_info.id,
                opponent.opponent.id,
                &event.owner,
                event.amount,
                odds,
                &event.tx_digest,
                &event.receipt_id,
            )
            .await
        {
            Ok((_, true)) => {
                indexed += 1;
                entries.push(SyncMatchStakeEntry {
                    tx_digest: event.tx_digest,
                    receipt_id: event.receipt_id,
                    owner: event.owner,
                    opponent_id: Some(opponent.opponent.id),
                    outcome: event.outcome,
                    amount_usdc: event.amount,
                    indexed: true,
                    reason: None,
                });
            }
            Ok((_, false)) => {
                skipped += 1;
                entries.push(SyncMatchStakeEntry {
                    tx_digest: event.tx_digest,
                    receipt_id: event.receipt_id,
                    owner: event.owner,
                    opponent_id: Some(opponent.opponent.id),
                    outcome: event.outcome,
                    amount_usdc: event.amount,
                    indexed: false,
                    reason: Some("already_indexed".to_string()),
                });
            }
            Err(e) => {
                skipped += 1;
                entries.push(SyncMatchStakeEntry {
                    tx_digest: event.tx_digest,
                    receipt_id: event.receipt_id,
                    owner: event.owner,
                    opponent_id: Some(opponent.opponent.id),
                    outcome: event.outcome,
                    amount_usdc: event.amount,
                    indexed: false,
                    reason: Some(e.to_string()),
                });
            }
        }
    }

    hydrate_match_from_chain(state, &mut match_with_odds).await;

    Ok(SyncMatchStakesResponse {
        match_id: match_with_odds.match_info.id,
        network,
        pool_object_id,
        seen: entries.len(),
        indexed,
        skipped,
        entries,
    })
}

fn pool_stake_deadline_ms(match_record: &MatchRecord, default_stake_window_hours: i64) -> u64 {
    let now = Utc::now();
    let deadline = [
        match_record.begin_at,
        match_record.scheduled_at,
        match_record.end_at,
    ]
    .into_iter()
    .flatten()
    .find(|dt| *dt > now)
    .unwrap_or_else(|| now + Duration::hours(default_stake_window_hours));

    deadline.timestamp_millis().max(0) as u64
}

async fn hydrate_matches_from_chain(state: &Arc<AppState>, matches: &mut [MatchWithOdds]) {
    for match_with_odds in matches {
        hydrate_match_from_chain(state, match_with_odds).await;
    }
}

async fn hydrate_match_from_chain(state: &Arc<AppState>, match_with_odds: &mut MatchWithOdds) {
    let Some(pool_object_id) = match_with_odds.match_info.sui_pool_object_id.as_deref() else {
        return;
    };
    let network = match_with_odds
        .match_info
        .sui_network
        .as_deref()
        .unwrap_or_else(|| state.sui.config().active_network.as_str());

    let snapshot = match state
        .sui
        .tournament_pool_snapshot(network, pool_object_id)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(e) => {
            tracing::warn!(
                match_id = %match_with_odds.match_info.id,
                pool_object_id,
                error = %e,
                "failed to hydrate tournament pool totals from Sui"
            );
            return;
        }
    };

    let total_pool = snapshot.total_a + snapshot.total_b;
    match_with_odds.total_pool_usdc = total_pool;
    for opponent in &mut match_with_odds.opponents {
        let pool_usdc = match opponent.opponent.position {
            0 => snapshot.total_a,
            1 => snapshot.total_b,
            _ => opponent.pool_usdc,
        };
        opponent.pool_usdc = pool_usdc;
        opponent.pool_percentage = if total_pool > 0 {
            (pool_usdc as f64 / total_pool as f64) * 100.0
        } else {
            50.0
        };
        opponent.odds = if pool_usdc > 0 {
            (total_pool as f64 / pool_usdc as f64).min(9999.0)
        } else if total_pool > 0 {
            9999.0
        } else {
            1.0
        };
    }
}

fn event_odds(event: &crate::services::sui::StakePlacedEvent) -> Option<Decimal> {
    let total = event.total_a + event.total_b;
    let side_total = match event.outcome {
        1 => event.total_a,
        2 => event.total_b,
        _ => 0,
    };
    if total <= 0 || side_total <= 0 {
        return Decimal::from_f64_retain(1.0);
    }
    Decimal::from_f64_retain((total as f64 / side_total as f64).min(9999.0))
}

pub async fn create_organizer_tournament(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateOrganizerTournamentRequest>,
) -> AppResult<OrganizerTournamentRecord> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let organizer_wallet = SuiService::normalize_address(&req.organizer_wallet)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    if auth_wallet != organizer_wallet {
        return Err(unauthorized("wallet in token does not match organizer"));
    }
    if req.name.trim().is_empty() {
        return Err(bad_request("Tournament name is required"));
    }
    req.organizer_wallet = organizer_wallet;
    require_approved_organizer(&state, &auth_wallet).await?;

    let tournament = state
        .db
        .create_organizer_tournament(&req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(tournament)))
}

pub async fn list_organizer_tournaments(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OrganizerTournamentQuery>,
) -> AppResult<Vec<OrganizerTournamentRecord>> {
    let tournaments = state
        .db
        .list_organizer_tournaments(&query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(tournaments)))
}

pub async fn create_organizer_match(
    State(state): State<Arc<AppState>>,
    Path(tournament_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<CreateOrganizerMatchRequest>,
) -> AppResult<MatchWithOdds> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let organizer_wallet = SuiService::normalize_address(&req.organizer_wallet)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    if auth_wallet != organizer_wallet {
        return Err(unauthorized("wallet in token does not match organizer"));
    }
    if req.name.trim().is_empty() {
        return Err(bad_request("Match name is required"));
    }
    if req.opponents.len() != 2 {
        return Err(bad_request("Exactly 2 opponents required for staking"));
    }
    req.organizer_wallet = organizer_wallet;
    require_approved_organizer(&state, &auth_wallet).await?;

    let tournament_uuid =
        uuid::Uuid::parse_str(&tournament_id).map_err(|_| bad_request("Invalid tournament id"))?;
    let tournament = state
        .db
        .get_organizer_tournament(tournament_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Organizer tournament not found"))?;

    if tournament.organizer_wallet != req.organizer_wallet {
        return Err(unauthorized(
            "wallet in token does not match tournament organizer",
        ));
    }

    let match_record = state
        .db
        .create_organizer_match(&tournament, &req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let match_with_odds = state
        .db
        .get_match_with_odds(&match_record.id.to_string())
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| internal_error("Match not found after creation"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

pub async fn create_outcome_proposal(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<CreateOutcomeProposalRequest>,
) -> AppResult<OutcomeProposalRecord> {
    let auth_wallet = extract_wallet_from_jwt(&headers)?;
    if let Some(ref proposer_wallet) = req.proposer_wallet {
        let proposer_wallet = SuiService::normalize_address(proposer_wallet)
            .ok_or_else(|| bad_request("Invalid proposer wallet address"))?;
        if proposer_wallet != auth_wallet {
            return Err(unauthorized("wallet in token does not match proposer"));
        }
        req.proposer_wallet = Some(proposer_wallet);
    } else {
        req.proposer_wallet = Some(auth_wallet);
    }

    let match_uuid =
        uuid::Uuid::parse_str(&match_id).map_err(|_| bad_request("Invalid match id"))?;
    if req.proposed_winner_opponent_id.is_none() && req.proposed_winner_name.is_none() {
        return Err(bad_request(
            "Provide proposed_winner_opponent_id or proposed_winner_name",
        ));
    }
    if req.source.as_deref().unwrap_or("organizer") == "organizer" {
        let proposer = req
            .proposer_wallet
            .as_deref()
            .ok_or_else(|| bad_request("proposer_wallet is required"))?;
        require_approved_organizer(&state, proposer).await?;
    }

    let proposal = state
        .db
        .create_outcome_proposal(match_uuid, &req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(proposal)))
}

async fn require_approved_organizer(
    state: &Arc<AppState>,
    wallet: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let allowed = state
        .db
        .organizer_can_create_markets(wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    if !allowed {
        return Err(unauthorized(
            "organizer must be approved and KYC verified before creating markets",
        ));
    }
    Ok(())
}

pub async fn list_outcome_proposals(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
) -> AppResult<Vec<OutcomeProposalRecord>> {
    let match_uuid =
        uuid::Uuid::parse_str(&match_id).map_err(|_| bad_request("Invalid match id"))?;
    let proposals = state
        .db
        .list_outcome_proposals(match_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(proposals)))
}

pub async fn review_outcome_proposal(
    State(state): State<Arc<AppState>>,
    Path(proposal_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ReviewOutcomeProposalRequest>,
) -> AppResult<OutcomeProposalRecord> {
    verify_admin(&headers)?;
    let proposal_uuid =
        uuid::Uuid::parse_str(&proposal_id).map_err(|_| bad_request("Invalid proposal id"))?;

    let proposal = state
        .db
        .review_outcome_proposal(proposal_uuid, &req.decision, req.reviewer_wallet.as_deref())
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    if req.decision == "approve" {
        if let Some(winner_id) = proposal.proposed_winner_opponent_id {
            let result = state
                .db
                .resolve_match(
                    &proposal.match_id.to_string(),
                    &winner_id.to_string(),
                    None,
                    false,
                )
                .await
                .map_err(|e| bad_request(e.to_string()))?;
            notify_tournament_resolution(&state, &proposal.match_id.to_string(), &result).await;
        }
    }

    Ok(Json(ApiResponse::ok(proposal)))
}

// ─── POST /api/tournaments/:id/stake ──────────────────────────────────────────
/// Place a stake on a tournament outcome
pub async fn place_stake(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PlaceStakeRequest>,
) -> AppResult<PoolStakeRecord> {
    // Verify JWT and ensure wallet matches
    let auth_wallet = extract_wallet_from_jwt(&headers)?;
    if auth_wallet != req.user_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    // Validate amount
    if req.amount_usdc <= 0 {
        return Err(bad_request("Stake amount must be positive"));
    }

    // Minimum stake: 1 USDC = 1_000_000 micro-USDC
    if req.amount_usdc < 1_000_000 {
        return Err(bad_request("Minimum stake is 1 USDC"));
    }

    let stake = state
        .db
        .place_stake(
            &match_id,
            &req.opponent_id,
            &req.user_wallet,
            req.amount_usdc,
        )
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    // Broadcast stake notification
    let match_data = state.db.get_match_with_odds(&match_id).await.ok().flatten();
    if let Some(m) = match_data {
        let opponent_name = m
            .opponents
            .iter()
            .find(|o| o.opponent.id.to_string() == req.opponent_id)
            .map(|o| o.opponent.name.clone())
            .unwrap_or_default();

        notify_user_best_effort(
            &state,
            &req.user_wallet,
            "stake_placed",
            notification_payload(
                "Stake placed",
                &format!("You backed {} in {}.", opponent_name, m.match_info.name),
                notification_action(
                    "View tournament",
                    "open_tournament",
                    "GET",
                    format!("/api/tournaments/{}", match_id),
                    json!({
                        "match_id": match_id,
                        "stake_id": stake.id,
                    }),
                ),
                json!({
                    "stake_id": stake.id,
                    "match_id": match_id,
                    "match_name": m.match_info.name,
                    "opponent_id": req.opponent_id,
                    "opponent_name": opponent_name,
                    "amount_usdc": req.amount_usdc,
                    "total_pool_usdc": m.total_pool_usdc,
                }),
            ),
        )
        .await;
    }

    Ok(Json(ApiResponse::ok(stake)))
}

// ─── POST /api/tournaments/:id/calculate ──────────────────────────────────────
/// Calculate potential payout (preview before staking)
pub async fn calculate_payout(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    Json(req): Json<CalculatePayoutRequest>,
) -> AppResult<PayoutCalculation> {
    if req.amount_usdc <= 0 {
        return Err(bad_request("Amount must be positive"));
    }

    let calculation = state
        .db
        .calculate_payout(&match_id, &req.opponent_id, req.amount_usdc)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(calculation)))
}

// ─── GET /api/tournaments/:id/stakes ──────────────────────────────────────────
/// List individual stakes for a tournament.
pub async fn list_tournament_stakes(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    Query(query): Query<StakeListQuery>,
) -> AppResult<Vec<PoolStakeRecord>> {
    state
        .db
        .get_match_with_odds(&match_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    let stakes = state
        .db
        .list_stakes_by_match(&match_id, &query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(stakes)))
}

// ─── POST /api/tournaments/:id/resolve ────────────────────────────────────────
/// Resolve a tournament and process payouts (admin only)
pub async fn resolve_tournament(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ResolveMatchRequest>,
) -> AppResult<()> {
    verify_admin(&headers)?;

    let result = state
        .db
        .resolve_match(
            &match_id,
            &req.winner_opponent_id,
            req.provider_winner_id,
            req.forfeit.unwrap_or(false),
        )
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    // Send on-chain payouts/refunds (placeholder for future on-chain settlement)
    notify_tournament_resolution(&state, &match_id, &result).await;

    Ok(Json(ApiResponse::ok(())))
}

// ─── POST /api/tournaments/:id/cancel ─────────────────────────────────────────
/// Cancel a tournament and refund all stakes (admin only)
pub async fn cancel_tournament(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<()> {
    verify_admin(&headers)?;

    let refunds = state
        .db
        .cancel_match(&match_id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    notify_tournament_refunds(
        &state,
        &match_id,
        "stake_refunded",
        "Tournament cancelled",
        "Your stake was refunded because the tournament was cancelled.",
        &refunds,
    )
    .await;

    Ok(Json(ApiResponse::ok(())))
}

// ─── POST /api/tournaments/:id/sync ───────────────────────────────────────────
/// Sync tournament status from provider/admin data.
pub async fn sync_tournament(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateMatchRequest>,
) -> AppResult<MatchWithOdds> {
    verify_admin(&headers)?;

    // Update match data
    let _ = state
        .db
        .upsert_match(&req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // If status changed to finished, try auto-resolve
    if req.pandascore_status.as_deref() == Some("finished") {
        if let Some(raw_data) = &req.raw_data {
            if let Some(winner_id) = raw_data.get("winner_id").and_then(|v| v.as_i64()) {
                // Find opponent with this provider ID.
                let match_data = state.db.get_match_with_odds(&match_id).await.ok().flatten();
                if let Some(m) = match_data {
                    let winner_opponent = m
                        .opponents
                        .iter()
                        .find(|o| o.opponent.pandascore_id == winner_id as i32);

                    if let Some(winner) = winner_opponent {
                        let forfeit = raw_data
                            .get("forfeit")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if let Ok(result) = state
                            .db
                            .resolve_match(
                                &match_id,
                                &winner.opponent.id.to_string(),
                                Some(winner_id as i32),
                                forfeit,
                            )
                            .await
                        {
                            notify_tournament_resolution(&state, &match_id, &result).await;
                        }
                    }
                }
            }
        }
    }

    // Return updated match with odds
    let match_with_odds = state
        .db
        .get_match_with_odds(&match_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

async fn notify_tournament_resolution(
    state: &Arc<AppState>,
    match_id: &str,
    result: &ResolveResult,
) {
    match result {
        ResolveResult::Resolved(payouts) => {
            notify_tournament_payouts(state, match_id, payouts).await;
            match state.db.list_stakes_by_match_status(match_id, "lost").await {
                Ok(lost_stakes) => notify_tournament_losses(state, match_id, &lost_stakes).await,
                Err(e) => tracing::error!("failed to load losing stakes for notifications: {}", e),
            }
        }
        ResolveResult::Refunded(refunds) => {
            notify_tournament_refunds(
                state,
                match_id,
                "stake_refunded",
                "Stake refunded",
                "Your stake was refunded because the pool could not be settled.",
                refunds,
            )
            .await;
        }
        ResolveResult::Empty => {}
    }
}

async fn notify_tournament_payouts(state: &Arc<AppState>, match_id: &str, payouts: &[PayoutEntry]) {
    let match_name = match_display_name(state, match_id).await;
    for payout in payouts {
        notify_user_best_effort(
            state,
            &payout.user_wallet,
            "stake_won",
            notification_payload(
                "You won",
                &format!("{} settled. Your payout is ready to review.", match_name),
                notification_action(
                    "View result",
                    "open_tournament",
                    "GET",
                    format!("/api/tournaments/{}", match_id),
                    json!({
                        "match_id": match_id,
                        "stake_id": payout.stake_id,
                    }),
                ),
                json!({
                    "match_id": match_id,
                    "stake_id": payout.stake_id,
                    "payout_usdc": payout.amount_usdc,
                }),
            ),
        )
        .await;
    }
}

async fn notify_tournament_losses(
    state: &Arc<AppState>,
    match_id: &str,
    lost_stakes: &[PoolStakeRecord],
) {
    let match_name = match_display_name(state, match_id).await;
    for stake in lost_stakes {
        notify_user_best_effort(
            state,
            &stake.user_wallet,
            "stake_lost",
            notification_payload(
                "Tournament settled",
                &format!("{} settled against your pick.", match_name),
                notification_action(
                    "View result",
                    "open_tournament",
                    "GET",
                    format!("/api/tournaments/{}", match_id),
                    json!({
                        "match_id": match_id,
                        "stake_id": stake.id,
                    }),
                ),
                json!({
                    "match_id": match_id,
                    "stake_id": stake.id,
                    "amount_usdc": stake.amount_usdc,
                }),
            ),
        )
        .await;
    }
}

async fn notify_tournament_refunds(
    state: &Arc<AppState>,
    match_id: &str,
    kind: &str,
    title: &str,
    body: &str,
    refunds: &[PayoutEntry],
) {
    for refund in refunds {
        notify_user_best_effort(
            state,
            &refund.user_wallet,
            kind,
            notification_payload(
                title,
                body,
                notification_action(
                    "View refund",
                    "open_tournament",
                    "GET",
                    format!("/api/tournaments/{}", match_id),
                    json!({
                        "match_id": match_id,
                        "stake_id": refund.stake_id,
                    }),
                ),
                json!({
                    "match_id": match_id,
                    "stake_id": refund.stake_id,
                    "refund_usdc": refund.amount_usdc,
                }),
            ),
        )
        .await;
    }
}

async fn match_display_name(state: &Arc<AppState>, match_id: &str) -> String {
    state
        .db
        .get_match_with_odds(match_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.match_info.name)
        .unwrap_or_else(|| "Tournament".to_string())
}

// ─── GET /api/users/:wallet/stakes ────────────────────────────────────────────
/// Get user's stake history
pub async fn get_user_stakes(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    Query(query): Query<StakeListQuery>,
) -> AppResult<Vec<StakeWithMatch>> {
    let stakes = state
        .db
        .get_user_stakes(&wallet, &query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(stakes)))
}

// ─── GET /api/users/:wallet/stake-stats ───────────────────────────────────────
/// Get user's stake statistics
pub async fn get_user_stake_stats(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
) -> AppResult<UserStakeStats> {
    let stats = state
        .db
        .get_user_stake_stats(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(stats)))
}
