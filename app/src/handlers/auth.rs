use axum::{extract::{State, Path}, Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use jsonwebtoken::{EncodingKey, Header};
use crate::services::redis as redis_svc;

use crate::handlers::wager::AppState;

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub wallet: String,
    /// expiry seconds from now (optional, default 900 = 15m)
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub wallet: String,
    /// signature as base64 string or array of bytes
    pub signature: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct NonceResponse {
    pub nonce: String,
    pub expires_at: i64,
}

pub async fn mint_token(
    State(_state): State<Arc<AppState>>,
    // Expect admin header: X-Admin-Token or Authorization: Bearer <token>
    headers: axum::http::HeaderMap,
    Json(req): Json<TokenRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, Json<crate::models::ApiResponse<()>>)> {
    // check admin token
    let admin_token = match std::env::var("AUTH_ADMIN_TOKEN") {
        Ok(v) => v,
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err("server missing AUTH_ADMIN_TOKEN")))),
    };

    let got = headers.get("x-admin-token").and_then(|v| v.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string())));

    if got.as_deref() != Some(admin_token.as_str()) {
        return Err((StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("invalid admin token"))));
    }

    let ttl = req.ttl_seconds.unwrap_or(15 * 60);
    let exp = (chrono::Utc::now() + chrono::Duration::seconds(ttl as i64)).timestamp() as usize;

    #[derive(serde::Serialize)]
    struct Claims<'a> {
        wallet: &'a str,
        exp: usize,
    }

    let claims = Claims { wallet: &req.wallet, exp };

    let secret = match std::env::var("AUTH_JWT_SECRET") {
        Ok(s) => s,
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err("server missing AUTH_JWT_SECRET")))),
    };

    let token = jsonwebtoken::encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(format!("jwt encode error: {}", e)))))?;

    Ok(Json(TokenResponse { token, expires_at: exp as i64 }))
}

/// GET /auth/nonce/:wallet -> create and return a one-time nonce
pub async fn get_nonce(
    State(state): State<Arc<AppState>>,
    Path(wallet): axum::extract::Path<String>,
) -> Result<Json<NonceResponse>, (StatusCode, Json<crate::models::ApiResponse<()>>)> {
    // Rate limiting: prefer Redis-backed limiter when available (cross-instance).
    let max = 5u32;
    let window_secs = 60usize;
    if let Some(redis_client) = &state.redis_client {
        // Use redis INCR with expiry; create a connection manager per request
        let mut conn = redis_client.get_tokio_connection_manager().await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(format!("redis conn error: {}", e)))))?;
        let key = format!("nonce_rl:{}", wallet);
        let cnt = redis_svc::incr_with_expiry(&mut conn, &key, window_secs).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(e.to_string()))))?;
        if cnt > max as u64 {
            if let Some(c) = &state.rate_limit_exceeded {
                c.inc();
            }
            return Err((StatusCode::TOO_MANY_REQUESTS, Json(crate::models::ApiResponse::err("rate limit exceeded"))));
        }
    } else {
        let window = chrono::Duration::seconds(window_secs as i64);
        {
            let mut map = state.nonce_rate.lock().await;
            let now = chrono::Utc::now();
            match map.get_mut(&wallet) {
                Some((count, start)) => {
                    if *start + window > now {
                        if *count >= max {
                                if let Some(c) = &state.rate_limit_exceeded {
                                    c.inc();
                                }
                                return Err((StatusCode::TOO_MANY_REQUESTS, Json(crate::models::ApiResponse::err("rate limit exceeded"))));
                            }
                        *count += 1;
                    } else {
                        *start = now;
                        *count = 1;
                    }
                }
                None => {
                    map.insert(wallet.clone(), (1u32, now));
                }
            }
        }
    }

    let nonce = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(5);
    state.db.insert_nonce(&wallet, &nonce, expires_at).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(e.to_string()))))?;

    Ok(Json(NonceResponse { nonce, expires_at: expires_at.timestamp() }))
}

/// POST /auth/verify { wallet, signature } -> verify signature over nonce and mint JWT
pub async fn verify_signature(
    State(state): State<Arc<AppState>>,
    Json(req): Json<VerifyRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, Json<crate::models::ApiResponse<()>>)> {
    // Query DB for latest unused nonce
    let nonce_rec = state.db.get_latest_unused_nonce(&req.wallet).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(e.to_string()))))?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(crate::models::ApiResponse::err("no valid nonce found"))))?;

    // parse signature
    let sig_bytes: Vec<u8> = if req.signature.is_string() {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(req.signature.as_str().unwrap()).map_err(|_| (StatusCode::BAD_REQUEST, Json(crate::models::ApiResponse::err("invalid base64 signature"))))?
    } else if req.signature.is_array() {
        req.signature.as_array().unwrap().iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect()
    } else {
        return Err((StatusCode::BAD_REQUEST, Json(crate::models::ApiResponse::err("signature must be base64 string or byte array"))));
    };

    // Verify signature using helper (now backed by solana_sdk)
    crate::services::auth::verify_ed25519_signature(&req.wallet, &sig_bytes, nonce_rec.nonce.as_bytes())
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(crate::models::ApiResponse::err("signature verification failed"))))?;

    // mark nonce used
    state.db.consume_nonce(&req.wallet, &nonce_rec.nonce).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(e.to_string()))))?;

    // mint JWT
    let ttl = 15 * 60;
    let exp = (chrono::Utc::now() + chrono::Duration::seconds(ttl as i64)).timestamp() as usize;

    #[derive(serde::Serialize)]
    struct Claims<'a> {
        wallet: &'a str,
        exp: usize,
    }

    let claims = Claims { wallet: &req.wallet, exp };
    let secret = std::env::var("AUTH_JWT_SECRET").map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err("server missing AUTH_JWT_SECRET"))))?;
    let token = jsonwebtoken::encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::models::ApiResponse::err(format!("jwt encode error: {}", e)))))?;

    Ok(Json(TokenResponse { token, expires_at: exp as i64 }))
}
