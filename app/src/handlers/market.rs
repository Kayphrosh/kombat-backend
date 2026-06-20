use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{Duration, Utc};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    handlers::notify::{notification_action, notification_payload, notify_user_best_effort},
    models::{
        ActivateReceiptListingRequest, ApiResponse, CreateReceiptListingRequest,
        MarkReceiptListingSoldRequest, MatchWithOdds, PaymentMoveCall, PaymentPtbArgument,
        PaymentPtbStep, ReceiptListingQuery, ReceiptListingResponse, ReceiptMarketListingRecord,
        ReceiptMarketPtbResponse,
    },
    services::{auth::verify_jwt_get_wallet, sui::SuiService},
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

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
}

fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}

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

pub async fn create_receipt_listing(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateReceiptListingRequest>,
) -> AppResult<ReceiptListingResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let seller_wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;
    if auth_wallet != seller_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    let receipt_id = SuiService::normalize_address(&req.receipt_id)
        .ok_or_else(|| bad_request("Invalid receipt_id"))?;
    if req.ask_amount_usdc <= 0 {
        return Err(bad_request("ask_amount_usdc must be positive"));
    }

    let network = req
        .network
        .unwrap_or_else(|| state.sui.config().active_network.clone());
    let network_config = state
        .sui
        .config()
        .network(&network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;
    let network = network_config.network.clone();

    let match_uuid = Uuid::parse_str(&req.match_id).map_err(|_| bad_request("Invalid match_id"))?;
    let opponent_uuid =
        Uuid::parse_str(&req.opponent_id).map_err(|_| bad_request("Invalid opponent_id"))?;
    let match_with_odds = load_match(&state, &req.match_id).await?;
    validate_listing_target(&match_with_odds, opponent_uuid)?;
    if state
        .db
        .has_open_receipt_listing(&receipt_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
    {
        return Err(bad_request("receipt already has an open listing"));
    }

    let expires_at = req
        .expires_at
        .unwrap_or_else(|| Utc::now() + Duration::hours(2));
    if expires_at <= Utc::now() {
        return Err(bad_request("expires_at must be in the future"));
    }

    let listing = state
        .db
        .create_receipt_listing(
            &network,
            &seller_wallet,
            &receipt_id,
            match_uuid,
            opponent_uuid,
            req.ask_amount_usdc,
            expires_at,
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Snapshot the market state at listing time so resale buyers have a
    // verifiable record of the odds/pool they bought into. Best-effort.
    let listing_id = listing.id;
    let snapshot = json!({
        "record_type": "receipt_listing_snapshot",
        "listing_id": listing_id.to_string(),
        "receipt_id": receipt_id,
        "match_id": match_uuid.to_string(),
        "opponent_id": opponent_uuid.to_string(),
        "seller_wallet": seller_wallet,
        "ask_amount_usdc": req.ask_amount_usdc,
        "network": network,
        "listed_at": Utc::now().to_rfc3339(),
        "expires_at": expires_at.to_rfc3339(),
        "market_state": serde_json::to_value(&match_with_odds).unwrap_or(json!(null)),
    });
    crate::services::agent_pipeline::archive_to_walrus(
        &state,
        "receipt_listing_snapshot",
        Some(match_uuid.to_string()),
        Some(seller_wallet.clone()),
        snapshot,
        None,
        state.walrus.config().epochs,
    )
    .await;

    let response = build_listing_response_data(listing, &match_with_odds)?;
    notify_receipt_listing_draft(&state, &response).await;

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn list_receipt_listings(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReceiptListingQuery>,
) -> AppResult<Vec<ReceiptListingResponse>> {
    let listings = state
        .db
        .list_receipt_listings(&query)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let mut responses = Vec::with_capacity(listings.len());
    for listing in listings {
        let match_with_odds = load_match(&state, &listing.match_id.to_string()).await?;
        responses.push(build_listing_response_data(listing, &match_with_odds)?);
    }

    Ok(Json(ApiResponse::ok(responses)))
}

pub async fn get_receipt_listing(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
) -> AppResult<ReceiptListingResponse> {
    let listing = load_listing(&state, &listing_id).await?;
    let match_with_odds = load_match(&state, &listing.match_id.to_string()).await?;
    build_listing_response(listing, &match_with_odds)
}

pub async fn activate_receipt_listing(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ActivateReceiptListingRequest>,
) -> AppResult<ReceiptListingResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let seller_wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;
    if auth_wallet != seller_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    let listing_uuid =
        Uuid::parse_str(&listing_id).map_err(|_| bad_request("Invalid listing id"))?;
    let existing = load_listing(&state, &listing_id).await?;
    if existing.seller_wallet != seller_wallet {
        return Err(unauthorized(
            "wallet in token does not match listing seller",
        ));
    }
    let listing_object_id = SuiService::normalize_address(&req.listing_object_id)
        .ok_or_else(|| bad_request("Invalid listing_object_id"))?;

    let listing = state
        .db
        .activate_receipt_listing(
            listing_uuid,
            &listing_object_id,
            req.listing_tx_hash.as_deref(),
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let match_with_odds = load_match(&state, &listing.match_id.to_string()).await?;
    let response = build_listing_response_data(listing, &match_with_odds)?;
    notify_receipt_listing_active(&state, &response).await;

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn mark_receipt_listing_sold(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<MarkReceiptListingSoldRequest>,
) -> AppResult<ReceiptListingResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let buyer_wallet = SuiService::normalize_address(&req.buyer_wallet)
        .ok_or_else(|| bad_request("Invalid buyer_wallet"))?;
    if auth_wallet != buyer_wallet {
        return Err(unauthorized("wallet in token does not match buyer"));
    }

    let listing_uuid =
        Uuid::parse_str(&listing_id).map_err(|_| bad_request("Invalid listing id"))?;
    let existing = load_listing(&state, &listing_id).await?;
    if existing.seller_wallet == buyer_wallet {
        return Err(bad_request("seller cannot buy their own listing"));
    }
    if existing.status != "active" {
        return Err(bad_request("listing is not active"));
    }

    let listing = state
        .db
        .mark_receipt_listing_sold(listing_uuid, &buyer_wallet, req.sale_tx_hash.as_deref())
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let match_with_odds = load_match(&state, &listing.match_id.to_string()).await?;
    let response = build_listing_response_data(listing, &match_with_odds)?;

    notify_receipt_listing_sold(&state, &response).await;

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn get_receipt_listing_list_ptb(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<ReceiptMarketPtbResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let listing = load_listing(&state, &listing_id).await?;
    if listing.seller_wallet != auth_wallet {
        return Err(unauthorized(
            "wallet in token does not match listing seller",
        ));
    }

    let response = build_market_ptb(
        &state,
        &listing,
        "list_receipt",
        vec![
            PaymentPtbArgument {
                name: "receipt".to_string(),
                kind: "owned_object".to_string(),
                value: Some(json!(listing.receipt_id.clone())),
                source: "seller-owned StakeReceipt object".to_string(),
            },
            PaymentPtbArgument {
                name: "ask_amount".to_string(),
                kind: "u64".to_string(),
                value: Some(json!(listing.ask_amount_usdc.to_string())),
                source: "receipt_market_listings.ask_amount_usdc".to_string(),
            },
            PaymentPtbArgument {
                name: "expires_at_ms".to_string(),
                kind: "u64".to_string(),
                value: Some(json!(listing.expires_at.timestamp_millis().to_string())),
                source: "receipt_market_listings.expires_at".to_string(),
            },
            clock_arg(),
        ],
        vec![
            PaymentPtbStep {
                kind: "escrow_receipt".to_string(),
                description:
                    "Seller shares a ReceiptListing object that escrows the StakeReceipt."
                        .to_string(),
            },
            PaymentPtbStep {
                kind: "market_event".to_string(),
                description:
                    "Move emits ReceiptListed; frontend stores the listing object ID on the backend."
                        .to_string(),
            },
        ],
    )?;

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn get_receipt_listing_buy_ptb(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<ReceiptMarketPtbResponse> {
    let buyer_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let listing = load_listing(&state, &listing_id).await?;
    if listing.seller_wallet == buyer_wallet {
        return Err(bad_request("seller cannot buy their own listing"));
    }

    let listing_object_id = listing.listing_object_id.clone();
    let mut args = vec![
        PaymentPtbArgument {
            name: "listing".to_string(),
            kind: "shared_object".to_string(),
            value: listing_object_id.map(|id| json!(id)),
            source: "receipt_market_listings.listing_object_id".to_string(),
        },
        PaymentPtbArgument {
            name: "payment".to_string(),
            kind: "coin".to_string(),
            value: Some(json!(listing.ask_amount_usdc.to_string())),
            source: "split exact USDC amount from buyer's coins".to_string(),
        },
        clock_arg(),
    ];

    if listing.status != "active" {
        args[0].value = None;
    }

    let mut response = build_market_ptb(
        &state,
        &listing,
        "buy_receipt",
        args,
        vec![
            PaymentPtbStep {
                kind: "pay_seller".to_string(),
                description:
                    "Buyer pays the exact ask in USDC; Move transfers payment to the seller."
                        .to_string(),
            },
            PaymentPtbStep {
                kind: "transfer_receipt".to_string(),
                description:
                    "Move updates the receipt owner and transfers the StakeReceipt to the buyer."
                        .to_string(),
            },
        ],
    )?;

    if listing.status != "active" {
        response.can_build = false;
        response.reason = Some("listing_not_active".to_string());
        response.move_call = None;
    }

    Ok(Json(ApiResponse::ok(response)))
}

async fn notify_receipt_listing_draft(state: &Arc<AppState>, response: &ReceiptListingResponse) {
    notify_user_best_effort(
        state,
        &response.listing.seller_wallet,
        "receipt_listing_draft_created",
        notification_payload(
            "Finish listing your stake",
            &format!(
                "Your {} receipt for {} is ready to escrow for sale.",
                response.opponent_name, response.match_name
            ),
            notification_action(
                "List receipt",
                "open_list_receipt",
                "GET",
                format!(
                    "/api/receipt-market/listings/{}/list-ptb",
                    response.listing.id
                ),
                json!({
                    "listing_id": response.listing.id,
                    "receipt_id": response.listing.receipt_id,
                }),
            ),
            json!({
                "listing_id": response.listing.id,
                "match_id": response.listing.match_id,
                "opponent_id": response.listing.opponent_id,
                "ask_amount_usdc": response.listing.ask_amount_usdc,
                "network": response.listing.network,
            }),
        ),
    )
    .await
}

async fn notify_receipt_listing_active(state: &Arc<AppState>, response: &ReceiptListingResponse) {
    notify_user_best_effort(
        state,
        &response.listing.seller_wallet,
        "receipt_listing_active",
        notification_payload(
            "Receipt listed",
            &format!(
                "Your {} receipt for {} is now available in the market.",
                response.opponent_name, response.match_name
            ),
            notification_action(
                "View listing",
                "open_receipt_listing",
                "GET",
                format!("/api/receipt-market/listings/{}", response.listing.id),
                json!({
                    "listing_id": response.listing.id,
                }),
            ),
            json!({
                "listing_id": response.listing.id,
                "match_id": response.listing.match_id,
                "opponent_id": response.listing.opponent_id,
                "ask_amount_usdc": response.listing.ask_amount_usdc,
                "listing_object_id": response.listing.listing_object_id,
            }),
        ),
    )
    .await
}

async fn notify_receipt_listing_sold(state: &Arc<AppState>, response: &ReceiptListingResponse) {
    let Some(buyer_wallet) = response.listing.buyer_wallet.as_deref() else {
        tracing::error!("sold listing is missing buyer wallet");
        return;
    };

    notify_user_best_effort(
        state,
        &response.listing.seller_wallet,
        "receipt_listing_sold",
        notification_payload(
            "Receipt sold",
            &format!(
                "Your {} receipt for {} sold for USDC.",
                response.opponent_name, response.match_name
            ),
            notification_action(
                "View sale",
                "open_receipt_listing",
                "GET",
                format!("/api/receipt-market/listings/{}", response.listing.id),
                json!({
                    "listing_id": response.listing.id,
                }),
            ),
            json!({
                "listing_id": response.listing.id,
                "match_id": response.listing.match_id,
                "opponent_id": response.listing.opponent_id,
                "ask_amount_usdc": response.listing.ask_amount_usdc,
                "sale_tx_hash": response.listing.sale_tx_hash,
            }),
        ),
    )
    .await;

    notify_user_best_effort(
        state,
        buyer_wallet,
        "receipt_purchase_confirmed",
        notification_payload(
            "Receipt purchase confirmed",
            &format!(
                "You now hold the {} receipt for {}.",
                response.opponent_name, response.match_name
            ),
            notification_action(
                "View tournament",
                "open_tournament",
                "GET",
                format!("/api/tournaments/{}", response.listing.match_id),
                json!({
                    "match_id": response.listing.match_id,
                    "receipt_id": response.listing.receipt_id,
                }),
            ),
            json!({
                "listing_id": response.listing.id,
                "match_id": response.listing.match_id,
                "opponent_id": response.listing.opponent_id,
                "ask_amount_usdc": response.listing.ask_amount_usdc,
                "receipt_id": response.listing.receipt_id,
                "sale_tx_hash": response.listing.sale_tx_hash,
            }),
        ),
    )
    .await
}

fn build_market_ptb(
    state: &Arc<AppState>,
    listing: &ReceiptMarketListingRecord,
    function: &str,
    arguments: Vec<PaymentPtbArgument>,
    steps: Vec<PaymentPtbStep>,
) -> Result<ReceiptMarketPtbResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let network_config = state
        .sui
        .config()
        .network(&listing.network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;
    let coin_type = network_config.usdc_coin_type.clone();
    let package_id = network_config.package_id.clone();

    let reason = if package_id.is_none() {
        Some("staking_package_not_configured".to_string())
    } else if coin_type.is_none() {
        Some("usdc_coin_type_not_configured".to_string())
    } else if Utc::now() >= listing.expires_at {
        Some("listing_expired".to_string())
    } else {
        None
    };

    let can_build = reason.is_none();
    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        Some(PaymentMoveCall {
            target: format!(
                "{}::{}::{}",
                package_id, network_config.staking_module, function
            ),
            package_id,
            module: network_config.staking_module.clone(),
            function: function.to_string(),
            type_arguments: vec![coin_type],
            arguments,
        })
    } else {
        None
    };

    Ok(ReceiptMarketPtbResponse {
        listing_id: listing.id,
        network: listing.network.clone(),
        can_build,
        reason,
        coin_type,
        expected_receipt_type: "StakeReceipt".to_string(),
        steps,
        move_call,
    })
}

fn clock_arg() -> PaymentPtbArgument {
    PaymentPtbArgument {
        name: "clock".to_string(),
        kind: "shared_object".to_string(),
        value: Some(json!("0x6")),
        source: "Sui Clock object".to_string(),
    }
}

async fn load_listing(
    state: &Arc<AppState>,
    listing_id: &str,
) -> Result<ReceiptMarketListingRecord, (StatusCode, Json<ApiResponse<()>>)> {
    let listing_uuid =
        Uuid::parse_str(listing_id).map_err(|_| bad_request("Invalid listing id"))?;
    state
        .db
        .get_receipt_listing(listing_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Receipt listing not found"))
}

async fn load_match(
    state: &Arc<AppState>,
    match_id: &str,
) -> Result<MatchWithOdds, (StatusCode, Json<ApiResponse<()>>)> {
    state
        .db
        .get_match_with_odds(match_id)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Tournament not found"))
}

fn validate_listing_target(
    match_with_odds: &MatchWithOdds,
    opponent_id: Uuid,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if match_with_odds.match_info.status != "upcoming"
        && match_with_odds.match_info.status != "live"
    {
        return Err(bad_request(format!(
            "Tournament receipts cannot be listed in status {}",
            match_with_odds.match_info.status
        )));
    }

    match_with_odds
        .opponents
        .iter()
        .find(|opponent| opponent.opponent.id == opponent_id)
        .map(|_| ())
        .ok_or_else(|| bad_request("Opponent not found for this tournament"))
}

fn build_listing_response(
    listing: ReceiptMarketListingRecord,
    match_with_odds: &MatchWithOdds,
) -> AppResult<ReceiptListingResponse> {
    build_listing_response_data(listing, match_with_odds).map(|data| Json(ApiResponse::ok(data)))
}

fn build_listing_response_data(
    listing: ReceiptMarketListingRecord,
    match_with_odds: &MatchWithOdds,
) -> Result<ReceiptListingResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let opponent = match_with_odds
        .opponents
        .iter()
        .find(|opponent| opponent.opponent.id == listing.opponent_id)
        .ok_or_else(|| bad_request("Opponent not found for this tournament"))?;
    let id = listing.id;
    let network = listing.network.clone();
    let wallet_address = listing.seller_wallet.clone();
    let seller_wallet = listing.seller_wallet.clone();
    let buyer_wallet = listing.buyer_wallet.clone();
    let receipt_id = listing.receipt_id.clone();
    let listing_object_id = listing.listing_object_id.clone();
    let match_id = listing.match_id;
    let opponent_id = listing.opponent_id;
    let ask_amount_usdc = listing.ask_amount_usdc;
    let status = listing.status.clone();
    let listing_tx_hash = listing.listing_tx_hash.clone();
    let sale_tx_hash = listing.sale_tx_hash.clone();
    let expires_at = listing.expires_at;
    let created_at = listing.created_at;
    let updated_at = listing.updated_at;

    Ok(ReceiptListingResponse {
        listing,
        id,
        network,
        wallet_address,
        seller_wallet,
        buyer_wallet,
        receipt_id,
        listing_object_id,
        match_id,
        opponent_id,
        ask_amount_usdc,
        status,
        listing_tx_hash,
        sale_tx_hash,
        expires_at,
        created_at,
        updated_at,
        match_name: match_with_odds.match_info.name.clone(),
        opponent_name: opponent.opponent.name.clone(),
    })
}
