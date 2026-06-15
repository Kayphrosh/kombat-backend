use axum::{extract::State, http::StatusCode, Json};
use jsonwebtoken::{EncodingKey, Header};
use std::sync::Arc;

use crate::state::AppState;

/// POST /api/auth/verify — verify Dynamic SDK JWT and return app-level JWT
pub async fn verify_dynamic(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::models::DynamicAuthRequest>,
) -> Result<
    Json<crate::models::DynamicAuthResponse>,
    (StatusCode, Json<crate::models::ApiResponse<()>>),
> {
    let dynamic_svc = state.dynamic_service.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::models::ApiResponse::err(
                "Dynamic service not configured",
            )),
        )
    })?;

    // Verify the Dynamic SDK token
    let claims = dynamic_svc
        .verify_token(&req.dynamic_token)
        .await
        .map_err(|e| {
            (
                StatusCode::UNAUTHORIZED,
                Json(crate::models::ApiResponse::err(format!(
                    "Dynamic token verification failed: {}",
                    e
                ))),
            )
        })?;

    // Extract wallet address from the token claims
    let wallet = claims.wallet_address().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(crate::models::ApiResponse::err(
                "No wallet address in Dynamic token",
            )),
        )
    })?;

    // Upsert the user (create if first login)
    // Only set display_name from email for NEW users who don't have one yet
    let existing_user = state.db.get_user(&wallet).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::models::ApiResponse::err(format!("DB error: {}", e))),
        )
    })?;

    let display_name = match &existing_user {
        Some(u) if u.display_name.is_some() => None, // keep existing name
        _ => claims.email_address(),                 // new user or no name set
    };

    let profile_req = crate::models::UpdateProfileRequest {
        email: claims.email_address(),
        display_name,
        avatar_url: None,
    };
    let user = state
        .db
        .upsert_user(&wallet, &profile_req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(crate::models::ApiResponse::err(format!(
                    "Failed to upsert user: {}",
                    e
                ))),
            )
        })?;

    // Mint an app-level JWT
    let ttl = 15 * 60;
    let exp = (chrono::Utc::now() + chrono::Duration::seconds(ttl as i64)).timestamp() as usize;

    #[derive(serde::Serialize)]
    struct Claims<'a> {
        wallet: &'a str,
        exp: usize,
    }

    let jwt_claims = Claims {
        wallet: &wallet,
        exp,
    };
    let secret = std::env::var("AUTH_JWT_SECRET").map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::models::ApiResponse::err(
                "server missing AUTH_JWT_SECRET",
            )),
        )
    })?;
    let token = jsonwebtoken::encode(
        &Header::default(),
        &jwt_claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::models::ApiResponse::err(format!(
                "jwt encode error: {}",
                e
            ))),
        )
    })?;

    Ok(Json(crate::models::DynamicAuthResponse {
        user,
        access_token: token,
    }))
}
