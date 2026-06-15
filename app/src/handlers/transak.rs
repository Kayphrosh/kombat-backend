use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;

use crate::{
    models::{
        ApiResponse, TransakQuoteRequest, TransakQuoteResponse, TransakWidgetRequest,
        TransakWidgetResponse,
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

pub async fn get_transak_config(
    State(state): State<Arc<AppState>>,
) -> AppResult<crate::services::transak::TransakConfig> {
    Ok(Json(ApiResponse::ok(state.transak.config().clone())))
}

pub async fn create_transak_widget_url(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<TransakWidgetRequest>,
) -> AppResult<TransakWidgetResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let requested_wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;

    if auth_wallet != requested_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    let product = req
        .product
        .clone()
        .unwrap_or_else(|| "BUY".to_string())
        .to_ascii_uppercase();
    if product != "BUY" {
        return Err(bad_request("Only BUY on-ramp sessions are supported"));
    }

    let widget_url = state
        .transak
        .widget_url(&req)
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(TransakWidgetResponse {
        provider: "transak".to_string(),
        product,
        wallet_address: requested_wallet,
        widget_url,
    })))
}

pub async fn get_transak_quote(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<TransakQuoteRequest>,
) -> AppResult<TransakQuoteResponse> {
    let _wallet = extract_wallet_from_jwt(&headers)?;

    let quote = state
        .transak
        .quote(&req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(quote)))
}
