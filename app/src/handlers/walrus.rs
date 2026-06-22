use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;

use crate::{
    models::{
        ApiResponse, CreateWalrusArtifactRequest, ListWalrusArtifactsQuery, WalrusArtifactRecord,
        WalrusArtifactResponse, WalrusBlobUrlResponse, WalrusConfigResponse,
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

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::NOT_FOUND, Json(ApiResponse::err(msg)))
}

pub async fn list_walrus_artifacts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListWalrusArtifactsQuery>,
) -> AppResult<Vec<WalrusArtifactRecord>> {
    let limit = params.limit.unwrap_or(20).min(100) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    let artifacts = state
        .db
        .list_walrus_artifacts(
            params.artifact_type.as_deref(),
            params.owner_wallet.as_deref(),
            limit,
            offset,
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(artifacts)))
}

pub async fn get_walrus_config(
    State(state): State<Arc<AppState>>,
) -> AppResult<WalrusConfigResponse> {
    let config = state.walrus.config();
    Ok(Json(ApiResponse::ok(WalrusConfigResponse {
        enabled: config.enabled,
        configured: config.configured(),
        network: config.network.clone(),
        aggregator_url: config.aggregator_url.clone(),
        max_upload_bytes: config.max_upload_bytes,
    })))
}

pub async fn create_walrus_artifact(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut req): Json<CreateWalrusArtifactRequest>,
) -> AppResult<WalrusArtifactResponse> {
    if req.artifact_type.trim().is_empty() {
        return Err(bad_request("artifact_type is required"));
    }
    if req.artifact_type.len() > 40 {
        return Err(bad_request("artifact_type must be 40 characters or fewer"));
    }

    let auth = verify_artifact_auth(&headers)?;
    if let Some(owner_wallet) = req.owner_wallet.as_ref() {
        let normalized = SuiService::normalize_address(owner_wallet)
            .ok_or_else(|| bad_request("Invalid owner_wallet address"))?;
        if matches!(auth, ArtifactAuth::Wallet(ref wallet) if wallet != &normalized) {
            return Err(unauthorized("wallet in token does not match owner_wallet"));
        }
        req.owner_wallet = Some(normalized);
    } else if matches!(auth, ArtifactAuth::Wallet(_)) {
        return Err(bad_request(
            "owner_wallet is required for wallet-authenticated uploads",
        ));
    }

    let stored = state
        .walrus
        .store_json(&req.manifest)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    let artifact = state
        .db
        .create_walrus_artifact(&req, &stored)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(WalrusArtifactResponse {
        blob_url: artifact
            .aggregator_url
            .clone()
            .or_else(|| state.walrus.blob_url(&artifact.blob_id)),
        artifact,
    })))
}

pub async fn get_walrus_artifact(
    State(state): State<Arc<AppState>>,
    Path(artifact_id): Path<String>,
) -> AppResult<WalrusArtifactRecord> {
    let artifact_uuid =
        uuid::Uuid::parse_str(&artifact_id).map_err(|_| bad_request("Invalid artifact id"))?;
    let artifact = state
        .db
        .get_walrus_artifact(artifact_uuid)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Walrus artifact not found"))?;

    Ok(Json(ApiResponse::ok(artifact)))
}

pub async fn get_walrus_blob_url(
    State(state): State<Arc<AppState>>,
    Path(blob_id): Path<String>,
) -> AppResult<WalrusBlobUrlResponse> {
    if blob_id.trim().is_empty() {
        return Err(bad_request("blob_id is required"));
    }
    let url = state
        .walrus
        .blob_url(&blob_id)
        .ok_or_else(|| internal_error("WALRUS_AGGREGATOR_URL is not configured"))?;

    Ok(Json(ApiResponse::ok(WalrusBlobUrlResponse {
        blob_id,
        url,
    })))
}

enum ArtifactAuth {
    Wallet(String),
    AdminOrAgent,
}

fn verify_artifact_auth(
    headers: &HeaderMap,
) -> Result<ArtifactAuth, (StatusCode, Json<ApiResponse<()>>)> {
    if token_matches(headers, "x-admin-token", "AUTH_ADMIN_TOKEN")
        || token_matches(headers, "x-agent-token", "AGENT_API_TOKEN")
    {
        return Ok(ArtifactAuth::AdminOrAgent);
    }

    let secret = std::env::var("AUTH_JWT_SECRET")
        .map_err(|_| internal_error("server missing AUTH_JWT_SECRET"))?;
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;

    verify_jwt_get_wallet(token, &secret)
        .map(ArtifactAuth::Wallet)
        .map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

fn token_matches(headers: &HeaderMap, header_name: &str, env_name: &str) -> bool {
    let Ok(expected) = std::env::var(env_name) else {
        return false;
    };
    headers
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == expected)
        .unwrap_or(false)
        || headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(|value| value == expected)
            .unwrap_or(false)
}
