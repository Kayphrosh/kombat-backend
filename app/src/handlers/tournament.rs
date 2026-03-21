
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, HeaderMap},
    Json,
};
use std::str::FromStr;
use std::sync::Arc;

use crate::{
    models::*,
    handlers::wager::AppState,
    services::auth::verify_jwt_get_wallet,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::err(msg)))
}

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}

/// Extract wallet from JWT in Authorization header
fn extract_wallet_from_jwt(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ApiResponse<()>>)> {
    let secret = std::env::var("AUTH_JWT_SECRET")
        .map_err(|_| internal_error("server missing AUTH_JWT_SECRET"))?;

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;

    verify_jwt_get_wallet(token, &secret)
        .map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

/// Verify admin token from X-Admin-Token header or Authorization: Bearer
fn verify_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let admin_token = std::env::var("AUTH_ADMIN_TOKEN")
        .map_err(|_| internal_error("server missing AUTH_ADMIN_TOKEN"))?;

    let got = headers.get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| headers.get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string())));

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
    let matches = state.db.list_matches(&query).await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(matches)))
}

// ─── POST /api/tournaments ────────────────────────────────────────────────────
/// Create or sync a tournament from PandaScore data
/// Frontend pushes tournament data here when user wants to stake
pub async fn create_tournament(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateMatchRequest>,
) -> AppResult<MatchWithOdds> {
    // Require authenticated user to prevent spam
    let _wallet = extract_wallet_from_jwt(&headers)?;

    // Validate we have exactly 2 opponents for binary betting
    if req.opponents.len() != 2 {
        return Err(bad_request("Exactly 2 opponents required for betting"));
    }

    // Check if match already exists
    if let Some(existing) = state.db.get_match_by_pandascore_id(req.pandascore_id).await
        .map_err(|e| internal_error(e.to_string()))? 
    {
        // Return existing match with odds
        let match_with_odds = state.db.get_match_with_odds(&existing.id.to_string()).await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| internal_error("Match not found after lookup"))?;
        return Ok(Json(ApiResponse::ok(match_with_odds)));
    }

    // Create new match
    let match_record = state.db.upsert_match(&req).await
        .map_err(|e| internal_error(e.to_string()))?;

    // Get with full odds data
    let match_with_odds = state.db.get_match_with_odds(&match_record.id.to_string()).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| internal_error("Match not found after creation"))?;

    tracing::info!("Tournament synced: {} (PandaScore ID: {})", req.name, req.pandascore_id);

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

// ─── GET /api/tournaments/:id ─────────────────────────────────────────────────
/// Get a single tournament with odds
pub async fn get_tournament(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<MatchWithOdds> {
    let match_with_odds = state.db.get_match_with_odds(&id).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
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

    let stake = state.db.place_stake(
        &match_id,
        &req.opponent_id,
        &req.user_wallet,
        req.amount_usdc,
    ).await.map_err(|e| bad_request(e.to_string()))?;

    // Broadcast stake notification
    let match_data = state.db.get_match_with_odds(&match_id).await.ok().flatten();
    if let Some(m) = match_data {
        let opponent_name = m.opponents.iter()
            .find(|o| o.opponent.id.to_string() == req.opponent_id)
            .map(|o| o.opponent.name.clone())
            .unwrap_or_default();

        let payload = serde_json::json!({
            "type": "stake_placed",
            "match_id": match_id,
            "match_name": m.match_info.name,
            "opponent_name": opponent_name,
            "amount_usdc": req.amount_usdc,
            "total_pool": m.total_pool_usdc,
        });

        // Best-effort broadcast
        let _ = state.notif_tx.send((match_id.clone(), payload));
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

    let calculation = state.db.calculate_payout(&match_id, &req.opponent_id, req.amount_usdc).await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(calculation)))
}

// ─── GET /api/tournaments/:id/stakes ──────────────────────────────────────────
/// List stakes for a tournament (returns match with odds showing pool info)
pub async fn list_tournament_stakes(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    Query(_query): Query<StakeListQuery>,
) -> AppResult<MatchWithOdds> {
    // Return the match with full pool statistics
    // Individual stake details are available via /api/users/:wallet/stakes
    let match_with_odds = state.db.get_match_with_odds(&match_id).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
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

    let result = state.db.resolve_match(
        &match_id,
        &req.winner_opponent_id,
        req.pandascore_winner_id,
        req.forfeit.unwrap_or(false),
    ).await.map_err(|e| bad_request(e.to_string()))?;

    // Send on-chain payouts/refunds (placeholder for future on-chain settlement)
    match result {
        crate::models::ResolveResult::Resolved(ref _payouts) => {}
        crate::models::ResolveResult::Refunded(ref _refunds) => {}
        crate::models::ResolveResult::Empty => {}
    }

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

    let refunds = state.db.cancel_match(&match_id).await
        .map_err(|e| bad_request(e.to_string()))?;

    // On-chain refunds will be handled separately

    Ok(Json(ApiResponse::ok(())))
}

// ─── POST /api/tournaments/:id/sync ───────────────────────────────────────────
/// Sync tournament status from PandaScore (frontend pushes update)
pub async fn sync_tournament(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    Json(req): Json<CreateMatchRequest>,
) -> AppResult<MatchWithOdds> {
    // Update match data
    let _ = state.db.upsert_match(&req).await
        .map_err(|e| internal_error(e.to_string()))?;

    // If status changed to finished, try auto-resolve
    if req.pandascore_status.as_deref() == Some("finished") {
        if let Some(raw_data) = &req.raw_data {
            if let Some(winner_id) = raw_data.get("winner_id").and_then(|v| v.as_i64()) {
                // Find opponent with this PandaScore ID
                let match_data = state.db.get_match_with_odds(&match_id).await.ok().flatten();
                if let Some(m) = match_data {
                    let winner_opponent = m.opponents.iter()
                        .find(|o| o.opponent.pandascore_id == winner_id as i32);
                    
                    if let Some(winner) = winner_opponent {
                        let forfeit = raw_data.get("forfeit").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Ok(result) = state.db.resolve_match(
                            &match_id,
                            &winner.opponent.id.to_string(),
                            Some(winner_id as i32),
                            forfeit,
                        ).await {
                            match result {
                                crate::models::ResolveResult::Resolved(ref _payouts) => {}
                                crate::models::ResolveResult::Refunded(ref _refunds) => {}
                                crate::models::ResolveResult::Empty => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Return updated match with odds
    let match_with_odds = state.db.get_match_with_odds(&match_id).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

// ─── GET /api/users/:wallet/stakes ────────────────────────────────────────────
/// Get user's stake history
pub async fn get_user_stakes(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    Query(query): Query<StakeListQuery>,
) -> AppResult<Vec<StakeWithMatch>> {
    let stakes = state.db.get_user_stakes(&wallet, &query).await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(stakes)))
}

// ─── GET /api/users/:wallet/stake-stats ───────────────────────────────────────
/// Get user's stake statistics
pub async fn get_user_stake_stats(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
) -> AppResult<UserStakeStats> {
    let stats = state.db.get_user_stake_stats(&wallet).await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(stats)))
}
