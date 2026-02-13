// app/src/handlers/user.rs
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::{
    handlers::wager::AppState,
    models::{ApiResponse, UpdateProfileRequest, UserRecord},
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