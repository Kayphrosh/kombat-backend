use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    models::{
        ApiResponse, OrganizerApplyRequest, OrganizerKycSessionRequest,
        OrganizerKycSessionResponse, OrganizerProfileRecord, ReviewOrganizerRequest,
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
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;

    verify_jwt_get_wallet(token, &secret).map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

fn verify_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let admin_token = std::env::var("AUTH_ADMIN_TOKEN")
        .map_err(|_| internal_error("server missing AUTH_ADMIN_TOKEN"))?;
    let got = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer ").map(ToString::to_string))
        });

    if got.as_deref() != Some(admin_token.as_str()) {
        return Err(unauthorized("invalid admin token"));
    }
    Ok(())
}

fn has_admin_token(headers: &HeaderMap) -> bool {
    let Ok(admin_token) = std::env::var("AUTH_ADMIN_TOKEN") else {
        return false;
    };
    let got = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer ").map(ToString::to_string))
        });

    got.as_deref() == Some(admin_token.as_str())
}

pub async fn apply_organizer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut req): Json<OrganizerApplyRequest>,
) -> AppResult<OrganizerProfileRecord> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    if auth_wallet != wallet {
        return Err(unauthorized("wallet in token does not match organizer"));
    }
    if req.organization_name.trim().is_empty() {
        return Err(bad_request("organization_name is required"));
    }
    req.wallet_address = wallet;

    let organizer = state
        .db
        .upsert_organizer_profile(&req)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(organizer)))
}

pub async fn get_organizer(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    headers: HeaderMap,
) -> AppResult<OrganizerProfileRecord> {
    let wallet = SuiService::normalize_address(&wallet)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    if !has_admin_token(&headers) {
        let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
            .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
        if auth_wallet != wallet {
            return Err(unauthorized("wallet in token does not match organizer"));
        }
    }

    let organizer = state
        .db
        .get_organizer_profile(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Organizer profile not found"))?;

    Ok(Json(ApiResponse::ok(organizer)))
}

pub async fn create_organizer_kyc_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<OrganizerKycSessionRequest>,
) -> AppResult<OrganizerKycSessionResponse> {
    let auth_wallet = SuiService::normalize_address(&extract_wallet_from_jwt(&headers)?)
        .ok_or_else(|| unauthorized("token wallet is not a valid Sui address"))?;
    let wallet = SuiService::normalize_address(&req.wallet_address)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    if auth_wallet != wallet {
        return Err(unauthorized("wallet in token does not match organizer"));
    }

    let existing = state
        .db
        .get_organizer_profile(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| bad_request("Apply as an organizer before starting KYC"))?;

    let provider = req.provider.unwrap_or_else(|| {
        std::env::var("KYC_PROVIDER").unwrap_or_else(|_| "manual_review".to_string())
    });
    let reference_id = format!("org_kyc_{}", Uuid::new_v4());
    let session_url = req.return_url.or(existing.kyc_session_url);
    let organizer = state
        .db
        .create_organizer_kyc_session(&wallet, &provider, &reference_id, session_url.as_deref())
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(OrganizerKycSessionResponse {
        organizer,
        provider,
        reference_id,
        session_url,
        status: "pending".to_string(),
    })))
}

pub async fn review_organizer(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ReviewOrganizerRequest>,
) -> AppResult<OrganizerProfileRecord> {
    verify_admin(&headers)?;
    let wallet = SuiService::normalize_address(&wallet)
        .ok_or_else(|| bad_request("Invalid organizer wallet address"))?;
    let organizer = state
        .db
        .review_organizer_profile(&wallet, &req)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    Ok(Json(ApiResponse::ok(organizer)))
}
