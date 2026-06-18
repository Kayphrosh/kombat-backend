use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    models::{
        ApiResponse, HomeSummaryResponse, NotificationSettings, StakeListQuery,
        UpdateNotificationSettings, UpdateProfileRequest, UserRecord, UserSearchQuery, UserStats,
    },
    services::{auth::verify_jwt_get_wallet, sui::SuiService},
    state::AppState,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::err(msg)),
    )
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
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

fn require_wallet(
    headers: &HeaderMap,
    wallet_address: &str,
) -> Result<String, (StatusCode, Json<ApiResponse<()>>)> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let requested_wallet = SuiService::normalize_address(wallet_address)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;

    if auth_wallet != requested_wallet {
        return Err(unauthorized("wallet in token does not match request"));
    }

    Ok(requested_wallet)
}

fn is_history_status(status: &str) -> bool {
    matches!(status, "resolved" | "cancelled" | "declined" | "expired")
}

#[derive(Debug, Deserialize)]
pub struct CheckEmailQuery {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct CheckEmailResponse {
    pub email: String,
    pub available: bool,
}

// ─── GET /users/:wallet ───────────────────────────────────────────────────────

pub async fn get_user_profile(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<UserRecord> {
    let user: Option<UserRecord> = state
        .db
        .get_user(&wallet_address)
        .await
        .map_err(|e: anyhow::Error| internal_error(e.to_string()))?;

    let user = user.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::err("User not found")),
        )
    })?;
    Ok(Json(ApiResponse::ok(user)))
}

// ─── GET /api/users/search ────────────────────────────────────────────────────

pub async fn search_users(
    State(state): State<Arc<AppState>>,
    Query(query_params): Query<UserSearchQuery>,
) -> AppResult<Vec<UserRecord>> {
    let query = query_params
        .query
        .or(query_params.q)
        .or(query_params.username)
        .or(query_params.display_name)
        .unwrap_or_default();

    if query.trim().is_empty() {
        return Ok(Json(ApiResponse::ok(vec![])));
    }

    let limit = query_params.limit.unwrap_or(20).min(50);

    let users = state
        .db
        .search_users(&query, limit)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(users)))
}

// ─── GET /check-email?email=... ───────────────────────────────────────────────

pub async fn check_email(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CheckEmailQuery>,
) -> AppResult<CheckEmailResponse> {
    let email = query.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(bad_request("Invalid email"));
    }

    let exists = state
        .db
        .email_exists(&email)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(CheckEmailResponse {
        email,
        available: !exists,
    })))
}

// ─── POST /users/:wallet ──────────────────────────────────────────────────────

pub async fn update_user_profile(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateProfileRequest>,
) -> AppResult<UserRecord> {
    let wallet_address = require_wallet(&headers, &wallet_address)?;
    let user: UserRecord = state
        .db
        .upsert_user(&wallet_address, &req)
        .await
        .map_err(|e: anyhow::Error| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(user)))
}

// ─── DELETE /users/:wallet ────────────────────────────────────────────────────

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    headers: HeaderMap,
) -> AppResult<()> {
    let wallet_address = require_wallet(&headers, &wallet_address)?;
    state
        .db
        .delete_user(&wallet_address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(())))
}

// ─── GET /users/:wallet/stats ─────────────────────────────────────────────────

pub async fn get_user_stats(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<UserStats> {
    let stats = state
        .db
        .get_user_stats(&wallet_address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(stats)))
}

// ─── GET /home/:wallet ────────────────────────────────────────────────────────

pub async fn get_home_summary(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<HomeSummaryResponse> {
    let stats = state
        .db
        .get_user_stats(&wallet_address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let wagers = state
        .db
        .list_my_wagers(&wallet_address, Some(100), Some(0))
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let active_stakes = state
        .db
        .get_user_stakes(
            &wallet_address,
            &StakeListQuery {
                status: Some("active".to_string()),
                match_id: None,
                limit: Some(20),
                offset: Some(0),
            },
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let (history_kombats, live_kombats): (Vec<_>, Vec<_>) = wagers
        .into_iter()
        .partition(|wager| is_history_status(&wager.wager.status));

    Ok(Json(ApiResponse::ok(HomeSummaryResponse {
        stats,
        live_kombats,
        history_kombats,
        active_stakes,
    })))
}

// ─── GET /users/:wallet/notification-settings ─────────────────────────────────

pub async fn get_notification_settings(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    headers: HeaderMap,
) -> AppResult<NotificationSettings> {
    let wallet_address = require_wallet(&headers, &wallet_address)?;
    let settings = state
        .db
        .get_notification_settings(&wallet_address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(settings)))
}

// ─── PUT /users/:wallet/notification-settings ─────────────────────────────────

pub async fn update_notification_settings(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateNotificationSettings>,
) -> AppResult<NotificationSettings> {
    let wallet_address = require_wallet(&headers, &wallet_address)?;
    let settings = state
        .db
        .upsert_notification_settings(&wallet_address, &req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(settings)))
}

// ─── POST /users/:wallet/push-token ───────────────────────────────────────────

pub async fn register_push_token(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    headers: HeaderMap,
    Json(req): Json<crate::models::RegisterPushTokenRequest>,
) -> AppResult<()> {
    let wallet_address = require_wallet(&headers, &wallet_address)?;
    state
        .db
        .upsert_push_token(&wallet_address, &req.expo_token)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(())))
}
