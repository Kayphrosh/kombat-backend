use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    handlers::notify::{notification_action, notification_payload, notify_user_best_effort},
    models::{
        ApiResponse, CreatePaymentIntentRequest, MatchWithOdds, PaymentIntentFunding,
        PaymentIntentOnrampResponse, PaymentIntentPtbResponse, PaymentIntentRecord,
        PaymentIntentResponse, PaymentIntentRule, PaymentMoveCall, PaymentPtbArgument,
        PaymentPtbStep, RampSessionResponse,
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

pub async fn create_payment_intent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreatePaymentIntentRequest>,
) -> AppResult<PaymentIntentResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let requested_wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;

    if auth_wallet != requested_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    if req.amount_usdc < 1_000_000 {
        return Err(bad_request("Minimum payment intent amount is 1 USDC"));
    }

    let reserve_balance_usdc = req.reserve_balance_usdc.unwrap_or(0);
    if reserve_balance_usdc < 0 {
        return Err(bad_request("reserve_balance_usdc cannot be negative"));
    }

    let settlement_rule = req
        .settlement_rule
        .unwrap_or_else(|| "return_to_wallet".to_string())
        .to_ascii_lowercase();
    if settlement_rule != "return_to_wallet" {
        return Err(bad_request(
            "Only return_to_wallet settlement is supported right now",
        ));
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

    let match_with_odds = load_match_for_payment(&state, &req.match_id).await?;
    validate_payment_target(&match_with_odds, opponent_uuid)?;
    if !match_with_odds.pool_configured {
        return Err(bad_request(
            "Tournament pool is not configured for on-chain staking",
        ));
    }

    let current_balance_usdc = wallet_usdc_balance(&state, &network, &requested_wallet).await?;
    let funding_shortfall_usdc =
        calculate_shortfall(req.amount_usdc, reserve_balance_usdc, current_balance_usdc)?;

    let intent = state
        .db
        .create_payment_intent(
            &requested_wallet,
            &network,
            match_uuid,
            opponent_uuid,
            req.amount_usdc,
            reserve_balance_usdc,
            &settlement_rule,
            current_balance_usdc,
            funding_shortfall_usdc,
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let response = build_intent_response(
        intent,
        current_balance_usdc,
        &match_with_odds,
        opponent_uuid,
    )?;

    notify_payment_intent_created(&state, &response).await;

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn get_payment_intent(
    State(state): State<Arc<AppState>>,
    Path(intent_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<PaymentIntentResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let intent = load_intent_for_wallet(&state, &intent_id, &auth_wallet).await?;
    let previous_status = intent.status.clone();
    let response = refresh_intent_response(&state, intent).await?;
    if previous_status == "requires_funding" && response.intent.status == "ready_to_stake" {
        notify_payment_intent_ready(&state, &response).await;
    }

    Ok(Json(ApiResponse::ok(response)))
}

pub async fn create_payment_intent_onramp_session(
    State(state): State<Arc<AppState>>,
    Path(intent_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<PaymentIntentOnrampResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let intent = load_intent_for_wallet(&state, &intent_id, &auth_wallet).await?;
    let response = refresh_intent_response(&state, intent).await?;

    let onramp_required = response.funding.onramp_required;
    if onramp_required && !state.ramp.config().dynamic_onramp_enabled {
        return Err(bad_request("Dynamic onramp is disabled"));
    }

    let ramp_session = if onramp_required {
        let shortfall = response.funding.funding_shortfall_usdc;
        let amount = Decimal::new(shortfall, 6);
        Some(RampSessionResponse {
            provider: "dynamic_native".to_string(),
            product: "BUY".to_string(),
            wallet_address: response.intent.user_wallet.clone(),
            launch_method: "dynamic_sdk".to_string(),
            client_action: "open_dynamic_onramp".to_string(),
            network: response.intent.network.clone(),
            crypto_currency_code: state.ramp.config().default_crypto_currency.clone(),
            fiat_currency: state.ramp.config().default_fiat_currency.clone(),
            fiat_amount: Some(amount),
            crypto_amount: Some(amount),
            note: "Launch Dynamic's native funding flow for this payment intent shortfall."
                .to_string(),
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(PaymentIntentOnrampResponse {
        intent: response,
        onramp_required,
        ramp_session,
    })))
}

pub async fn get_payment_intent_ptb(
    State(state): State<Arc<AppState>>,
    Path(intent_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<PaymentIntentPtbResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let intent = load_intent_for_wallet(&state, &intent_id, &auth_wallet).await?;
    let response = refresh_intent_response(&state, intent).await?;
    let match_with_odds =
        load_match_for_payment(&state, &response.intent.match_id.to_string()).await?;
    let opponent = validate_payment_target(&match_with_odds, response.intent.opponent_id)?;

    let network_config = state
        .sui
        .config()
        .network(&response.intent.network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;
    let coin_type = network_config.usdc_coin_type.clone();
    let package_id = network_config.package_id.clone();
    let pool_object_id = match_with_odds.match_info.sui_pool_object_id.clone();

    let mut reason = None;
    if response.funding.onramp_required {
        reason = Some("intent_requires_funding".to_string());
    } else if package_id.is_none() {
        reason = Some("staking_package_not_configured".to_string());
    } else if coin_type.is_none() {
        reason = Some("usdc_coin_type_not_configured".to_string());
    } else if pool_object_id.is_none() {
        reason = Some("pool_object_not_configured".to_string());
    }

    let can_build = reason.is_none();
    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        let pool_object_id = pool_object_id.clone().unwrap();
        Some(PaymentMoveCall {
            target: format!("{}::{}::stake", package_id, network_config.staking_module),
            package_id,
            module: network_config.staking_module.clone(),
            function: "stake".to_string(),
            type_arguments: vec![coin_type],
            arguments: vec![
                PaymentPtbArgument {
                    name: "pool".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!(pool_object_id)),
                    source: "matches.sui_pool_object_id".to_string(),
                },
                PaymentPtbArgument {
                    name: "outcome".to_string(),
                    kind: "u8".to_string(),
                    value: Some(json!((opponent.opponent.position + 1) as u8)),
                    source: "match_opponents.position".to_string(),
                },
                PaymentPtbArgument {
                    name: "payment".to_string(),
                    kind: "coin".to_string(),
                    value: Some(json!(response.intent.amount_usdc.to_string())),
                    source: "split exact USDC amount from user's coins".to_string(),
                },
                PaymentPtbArgument {
                    name: "clock".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!("0x6")),
                    source: "Sui Clock object".to_string(),
                },
            ],
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(PaymentIntentPtbResponse {
        intent_id: response.intent.id,
        network: response.intent.network,
        can_build,
        reason,
        coin_type,
        pool_configured: pool_object_id.is_some(),
        pool_object_id,
        amount_usdc: response.intent.amount_usdc,
        reserve_balance_usdc: response.intent.reserve_balance_usdc,
        expected_receipt_type: "StakeReceipt".to_string(),
        steps: vec![
            PaymentPtbStep {
                kind: "reserve_balance_rule".to_string(),
                description: format!(
                    "Leave at least {} micro-USDC in the wallet after staking.",
                    response.intent.reserve_balance_usdc
                ),
            },
            PaymentPtbStep {
                kind: "split_coin".to_string(),
                description: "Split the exact stake amount from the user's USDC coin objects."
                    .to_string(),
            },
            PaymentPtbStep {
                kind: "move_call".to_string(),
                description:
                    "Call tournament_staking::stake; the Move contract locks funds and mints a StakeReceipt."
                        .to_string(),
            },
        ],
        move_call,
    })))
}

async fn notify_payment_intent_created(state: &Arc<AppState>, response: &PaymentIntentResponse) {
    if response.funding.onramp_required {
        notify_user_best_effort(
            state,
            &response.intent.user_wallet,
            "payment_intent_requires_funding",
            notification_payload(
                "Fund wallet to enter tournament",
                &format!(
                    "{} needs more USDC before staking on {}.",
                    response.match_name, response.opponent_name
                ),
                notification_action(
                    "Fund wallet",
                    "open_onramp",
                    "POST",
                    format!(
                        "/api/payments/intents/{}/onramp-session",
                        response.intent.id
                    ),
                    json!({
                        "intent_id": response.intent.id,
                        "shortfall_usdc": response.funding.funding_shortfall_usdc,
                    }),
                ),
                json!({
                    "intent_id": response.intent.id,
                    "match_id": response.intent.match_id,
                    "opponent_id": response.intent.opponent_id,
                    "amount_usdc": response.intent.amount_usdc,
                    "funding_shortfall_usdc": response.funding.funding_shortfall_usdc,
                    "network": response.intent.network,
                }),
            ),
        )
        .await
    } else {
        notify_payment_intent_ready(state, response).await
    }
}

async fn notify_payment_intent_ready(state: &Arc<AppState>, response: &PaymentIntentResponse) {
    notify_user_best_effort(
        state,
        &response.intent.user_wallet,
        "payment_intent_ready_to_stake",
        notification_payload(
            "Stake is ready",
            &format!(
                "Your {} pick on {} can now be submitted.",
                response.opponent_name, response.match_name
            ),
            notification_action(
                "Review stake",
                "open_stake_confirmation",
                "GET",
                format!("/api/payments/intents/{}/ptb", response.intent.id),
                json!({
                    "intent_id": response.intent.id,
                    "match_id": response.intent.match_id,
                    "opponent_id": response.intent.opponent_id,
                }),
            ),
            json!({
                "intent_id": response.intent.id,
                "match_id": response.intent.match_id,
                "opponent_id": response.intent.opponent_id,
                "amount_usdc": response.intent.amount_usdc,
                "network": response.intent.network,
            }),
        ),
    )
    .await
}

async fn load_intent_for_wallet(
    state: &Arc<AppState>,
    intent_id: &str,
    wallet: &str,
) -> Result<PaymentIntentRecord, (StatusCode, Json<ApiResponse<()>>)> {
    let intent_uuid = Uuid::parse_str(intent_id).map_err(|_| bad_request("Invalid intent id"))?;
    let intent = state
        .db
        .get_payment_intent(intent_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Payment intent not found"))?;

    if intent.user_wallet != wallet {
        return Err(unauthorized(
            "wallet in token does not match payment intent",
        ));
    }

    Ok(intent)
}

async fn refresh_intent_response(
    state: &Arc<AppState>,
    intent: PaymentIntentRecord,
) -> Result<PaymentIntentResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let match_with_odds = load_match_for_payment(state, &intent.match_id.to_string()).await?;
    validate_payment_target(&match_with_odds, intent.opponent_id)?;

    let current_balance_usdc =
        wallet_usdc_balance(state, &intent.network, &intent.user_wallet).await?;
    let funding_shortfall_usdc = calculate_shortfall(
        intent.amount_usdc,
        intent.reserve_balance_usdc,
        current_balance_usdc,
    )?;

    let opponent_id = intent.opponent_id;
    let intent = state
        .db
        .update_payment_intent_funding(intent.id, current_balance_usdc, funding_shortfall_usdc)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    build_intent_response(intent, current_balance_usdc, &match_with_odds, opponent_id)
}

async fn load_match_for_payment(
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

fn validate_payment_target(
    match_with_odds: &MatchWithOdds,
    opponent_id: Uuid,
) -> Result<&crate::models::OpponentWithPool, (StatusCode, Json<ApiResponse<()>>)> {
    if match_with_odds.match_info.status != "upcoming"
        && match_with_odds.match_info.status != "live"
    {
        return Err(bad_request(format!(
            "Tournament is not accepting payments (status: {})",
            match_with_odds.match_info.status
        )));
    }

    match_with_odds
        .opponents
        .iter()
        .find(|opponent| opponent.opponent.id == opponent_id)
        .ok_or_else(|| bad_request("Opponent not found for this tournament"))
}

async fn wallet_usdc_balance(
    state: &Arc<AppState>,
    network: &str,
    wallet: &str,
) -> Result<i64, (StatusCode, Json<ApiResponse<()>>)> {
    let balance = state
        .sui
        .usdc_balance_for(network, wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    balance
        .total_balance
        .parse::<i64>()
        .map_err(|_| internal_error("Sui USDC balance did not fit in i64"))
}

fn calculate_shortfall(
    amount_usdc: i64,
    reserve_balance_usdc: i64,
    current_balance_usdc: i64,
) -> Result<i64, (StatusCode, Json<ApiResponse<()>>)> {
    let required = amount_usdc
        .checked_add(reserve_balance_usdc)
        .ok_or_else(|| bad_request("Payment amount plus reserve is too large"))?;
    Ok(required.saturating_sub(current_balance_usdc).max(0))
}

fn build_intent_response(
    intent: PaymentIntentRecord,
    current_balance_usdc: i64,
    match_with_odds: &MatchWithOdds,
    opponent_id: Uuid,
) -> Result<PaymentIntentResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let opponent = validate_payment_target(match_with_odds, opponent_id)?;
    let required_balance_usdc = intent
        .amount_usdc
        .checked_add(intent.reserve_balance_usdc)
        .ok_or_else(|| bad_request("Payment amount plus reserve is too large"))?;
    let funding_shortfall_usdc = required_balance_usdc
        .saturating_sub(current_balance_usdc)
        .max(0);

    let mut rules = Vec::new();
    if intent.reserve_balance_usdc > 0 {
        rules.push(PaymentIntentRule {
            rule_type: "reserve_balance".to_string(),
            amount_usdc: intent.reserve_balance_usdc,
            description: "Preserve this much USDC in the wallet after staking.".to_string(),
        });
    }

    Ok(PaymentIntentResponse {
        intent,
        funding: PaymentIntentFunding {
            current_balance_usdc,
            required_balance_usdc,
            funding_shortfall_usdc,
            onramp_required: funding_shortfall_usdc > 0,
        },
        rules,
        match_name: match_with_odds.match_info.name.clone(),
        opponent_name: opponent.opponent.name.clone(),
        pool_configured: match_with_odds.pool_configured,
        pool_object_id: match_with_odds.match_info.sui_pool_object_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::calculate_shortfall;

    fn shortfall(amount_usdc: i64, reserve_balance_usdc: i64, current_balance_usdc: i64) -> i64 {
        match calculate_shortfall(amount_usdc, reserve_balance_usdc, current_balance_usdc) {
            Ok(value) => value,
            Err(_) => panic!("shortfall calculation failed"),
        }
    }

    #[test]
    fn calculate_shortfall_clamps_to_zero_when_wallet_has_enough_balance() {
        assert_eq!(shortfall(50_000_000, 0, 75_000_000), 0);
    }

    #[test]
    fn calculate_shortfall_includes_reserve_balance() {
        assert_eq!(shortfall(50_000_000, 5_000_000, 20_000_000), 35_000_000);
    }
}
