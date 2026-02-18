// app/src/handlers/user.rs
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::{
    handlers::wager::AppState,
    models::{ApiResponse, UpdateProfileRequest, UserRecord, UserStats, NotificationSettings, UpdateNotificationSettings},
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::err(msg)))
}

// ─── GET /users/:wallet ───────────────────────────────────────────────────────

pub async fn get_user_profile(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<UserRecord> {
    let user: Option<UserRecord> = state.db.get_user(&wallet_address).await
        .map_err(|e: anyhow::Error| internal_error(e.to_string()))?;

    let user = user.ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("User not found"))))?;
    Ok(Json(ApiResponse::ok(user)))
}

// ─── POST /users/:wallet ──────────────────────────────────────────────────────

pub async fn update_user_profile(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    Json(req): Json<UpdateProfileRequest>,
) -> AppResult<UserRecord> {
    let user: UserRecord = state.db.upsert_user(&wallet_address, &req).await
        .map_err(|e: anyhow::Error| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(user)))
}

// ─── DELETE /users/:wallet ────────────────────────────────────────────────────

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<()> {
    state.db.delete_user(&wallet_address).await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(())))
}

// ─── GET /users/:wallet/stats ─────────────────────────────────────────────────

pub async fn get_user_stats(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<UserStats> {
    let stats = state.db.get_user_stats(&wallet_address).await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(stats)))
}

// ─── GET /users/:wallet/notification-settings ─────────────────────────────────

pub async fn get_notification_settings(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
) -> AppResult<NotificationSettings> {
    let settings = state.db.get_notification_settings(&wallet_address).await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(settings)))
}

// ─── PUT /users/:wallet/notification-settings ─────────────────────────────────

pub async fn update_notification_settings(
    State(state): State<Arc<AppState>>,
    Path(wallet_address): Path<String>,
    Json(req): Json<UpdateNotificationSettings>,
) -> AppResult<NotificationSettings> {
    let settings = state.db.upsert_notification_settings(&wallet_address, &req).await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(settings)))
}