use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;

use crate::{
    models::{
        ApiResponse, RampProviderQuery, RampProvidersResponse, RampSessionRequest,
        RampSessionResponse,
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

pub async fn list_ramp_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RampProviderQuery>,
) -> AppResult<RampProvidersResponse> {
    let config = state.ramp.config();

    Ok(Json(ApiResponse::ok(RampProvidersResponse {
        primary_provider: config.primary_provider.clone(),
        default_network: config.default_network.clone(),
        default_crypto_currency: config.default_crypto_currency.clone(),
        default_fiat_currency: config.default_fiat_currency.clone(),
        partner_fee_bps: config.partner_fee_bps,
        country: query.country.map(|country| country.to_ascii_uppercase()),
        providers: state.ramp.providers(state.transak.config().enabled),
    })))
}

pub async fn create_dynamic_ramp_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RampSessionRequest>,
) -> AppResult<RampSessionResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let requested_wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;

    if auth_wallet != requested_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    let config = state.ramp.config();
    let product = req
        .product
        .clone()
        .unwrap_or_else(|| "BUY".to_string())
        .to_ascii_uppercase();
    if product != "BUY" {
        return Err(bad_request("Only BUY on-ramp sessions are supported"));
    }

    if !config.dynamic_onramp_enabled {
        return Err(bad_request("Dynamic onramp is disabled"));
    }

    Ok(Json(ApiResponse::ok(RampSessionResponse {
        provider: "dynamic_native".to_string(),
        product: product.clone(),
        wallet_address: requested_wallet,
        launch_method: "dynamic_sdk".to_string(),
        client_action: "open_dynamic_onramp".to_string(),
        network: req
            .network
            .unwrap_or_else(|| config.default_network.clone())
            .to_ascii_lowercase(),
        crypto_currency_code: req
            .crypto_currency_code
            .unwrap_or_else(|| config.default_crypto_currency.clone())
            .to_ascii_uppercase(),
        fiat_currency: req
            .fiat_currency
            .unwrap_or_else(|| config.default_fiat_currency.clone())
            .to_ascii_uppercase(),
        fiat_amount: req.fiat_amount,
        crypto_amount: req.crypto_amount,
        note: "Launch Dynamic's native funding flow in the frontend; provider availability is configured in the Dynamic dashboard.".to_string(),
    })))
}
