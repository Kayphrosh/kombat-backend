use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
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
    let matches = state
        .db
        .list_matches(&query)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(matches)))
}

pub async fn get_tournament_source_pandascore(
    State(state): State<Arc<AppState>>,
) -> AppResult<PandaScoreSourceResponse> {
    let config = state.pandascore.config();
    Ok(Json(ApiResponse::ok(PandaScoreSourceResponse {
        provider: "pandascore".to_string(),
        enabled: config.enabled,
        configured: config.configured(),
        base_url: config.base_url.clone(),
        default_statuses: config.default_statuses.clone(),
        default_videogame_slugs: config.default_videogame_slugs.clone(),
        default_per_page: config.default_per_page,
    })))
}

pub async fn sync_pandascore_tournaments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PandaScoreSyncRequest>,
) -> AppResult<PandaScoreSyncResponse> {
    verify_admin(&headers)?;

    let fetched = state
        .pandascore
        .fetch_matches(&req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let mut synced = 0usize;
    let mut skipped = 0usize;
    let mut resolved = 0usize;
    let mut errors = Vec::new();

    for match_req in fetched.iter() {
        if match_req.opponents.len() != 2 {
            skipped += 1;
            continue;
        }

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

        if match_req.pandascore_status.as_deref() == Some("finished") {
            match resolve_finished_pandascore_match(&state, &match_record.id.to_string(), match_req)
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

    Ok(Json(ApiResponse::ok(PandaScoreSyncResponse {
        provider: "pandascore".to_string(),
        fetched: fetched.len(),
        synced,
        skipped,
        resolved,
        errors,
    })))
}

// ─── POST /api/tournaments ────────────────────────────────────────────────────
/// Create or sync a tournament from PandaScore-shaped data.
/// Server-side PandaScore sync should be preferred; this route remains useful
/// for admin backfills and local development.
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
    if let Some(existing) = state
        .db
        .get_match_by_pandascore_id(req.pandascore_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
    {
        // Return existing match with odds
        let match_with_odds = state
            .db
            .get_match_with_odds(&existing.id.to_string())
            .await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| internal_error("Match not found after lookup"))?;
        return Ok(Json(ApiResponse::ok(match_with_odds)));
    }

    // Create new match
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
        "Tournament synced: {} (PandaScore ID: {})",
        req.name,
        req.pandascore_id
    );

    Ok(Json(ApiResponse::ok(match_with_odds)))
}

async fn resolve_finished_pandascore_match(
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
        .ok_or_else(|| "match not found after PandaScore sync".to_string())?;

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

    Ok(!matches!(result, ResolveResult::Empty))
}

// ─── GET /api/tournaments/:id ─────────────────────────────────────────────────
/// Get a single tournament with odds
pub async fn get_tournament(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<MatchWithOdds> {
    let match_with_odds = state
        .db
        .get_match_with_odds(&id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))?;

    Ok(Json(ApiResponse::ok(match_with_odds)))
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
            req.pandascore_winner_id,
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
/// Sync tournament status from PandaScore (frontend pushes update)
pub async fn sync_tournament(
    State(state): State<Arc<AppState>>,
    Path(match_id): Path<String>,
    Json(req): Json<CreateMatchRequest>,
) -> AppResult<MatchWithOdds> {
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
                // Find opponent with this PandaScore ID
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
