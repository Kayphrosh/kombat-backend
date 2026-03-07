// app/src/handlers/delegation.rs
//! Handlers for SPL-Token delegation (approve / revoke / status).
//!
//! Flow:
//! 1. Frontend calls GET /api/delegation/approve-tx?wallet=X to get an unsigned approve tx
//! 2. User signs it in-wallet (one-time)
//! 3. All subsequent stakes go through the backend using the delegate authority

use axum::{
    extract::{Query, State},
    http::{StatusCode, HeaderMap},
    Json,
};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;

use crate::{
    models::ApiResponse,
    handlers::wager::AppState,
    services::auth::verify_jwt_get_wallet,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}
fn unauthorized(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::UNAUTHORIZED, Json(ApiResponse::err(msg)))
}
fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::err(msg)))
}

// ─── Query / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DelegationQuery {
    pub wallet: String,
    /// Optional: custom allowance in micro-USDC. Defaults to MAX_DELEGATION_USDC.
    pub amount: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ApproveResponse {
    /// Base64-encoded unsigned transaction for the user to sign.
    pub transaction: String,
    /// Platform signer public key (the delegate).
    pub delegate: String,
    /// Approved allowance in micro-USDC.
    pub amount: u64,
}

#[derive(Debug, Serialize)]
pub struct RevokeResponse {
    /// Base64-encoded unsigned transaction for the user to sign.
    pub transaction: String,
}

#[derive(Debug, Serialize)]
pub struct DelegationStatus {
    /// Whether delegation service is enabled on the backend.
    pub enabled: bool,
    /// Platform signer public key (the delegate).
    pub delegate: String,
    /// Current delegated amount remaining (micro-USDC), if on-chain lookup succeeds.
    pub delegated_amount: Option<u64>,
    /// The user's USDC token account address.
    pub token_account: Option<String>,
}

fn extract_wallet(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ApiResponse<()>>)> {
    let secret = std::env::var("AUTH_JWT_SECRET")
        .map_err(|_| internal_error("server missing AUTH_JWT_SECRET"))?;
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing Authorization header"))?;
    verify_jwt_get_wallet(token, &secret)
        .map_err(|e| unauthorized(format!("invalid token: {}", e)))
}

// ─── GET /api/delegation/approve-tx ──────────────────────────────────────────

/// Returns an unsigned `spl_token::approve` transaction for the user to sign
/// once. After signing, the platform signer can transfer USDC on their behalf.
pub async fn get_approve_tx(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DelegationQuery>,
) -> AppResult<ApproveResponse> {
    let auth_wallet = extract_wallet(&headers)?;
    if auth_wallet != query.wallet {
        return Err(unauthorized("wallet mismatch"));
    }

    let delegation = state.delegation.as_ref()
        .ok_or_else(|| bad_request("Delegation not enabled on this server"))?;

    let wallet_pubkey = Pubkey::from_str(&query.wallet)
        .map_err(|_| bad_request("Invalid wallet address"))?;

    // Derive the user's USDC ATA
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to get USDC mint: {}", e)))?;
    let user_ata = state.solana.get_associated_token_address(&wallet_pubkey, &usdc_mint);

    let amount = query.amount
        .unwrap_or(crate::services::delegation::MAX_DELEGATION_USDC)
        .min(crate::services::delegation::MAX_DELEGATION_USDC);

    let blockhash = state.solana.rpc.get_latest_blockhash().await
        .map_err(|e| internal_error(format!("RPC error: {}", e)))?;

    let tx_b64 = delegation.build_approve_tx(&wallet_pubkey, &user_ata, amount, blockhash)
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(ApproveResponse {
        transaction: tx_b64,
        delegate: delegation.pubkey().to_string(),
        amount,
    })))
}

// ─── GET /api/delegation/revoke-tx ───────────────────────────────────────────

/// Returns an unsigned `spl_token::revoke` transaction for the user to sign.
pub async fn get_revoke_tx(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DelegationQuery>,
) -> AppResult<RevokeResponse> {
    let auth_wallet = extract_wallet(&headers)?;
    if auth_wallet != query.wallet {
        return Err(unauthorized("wallet mismatch"));
    }

    let delegation = state.delegation.as_ref()
        .ok_or_else(|| bad_request("Delegation not enabled on this server"))?;

    let wallet_pubkey = Pubkey::from_str(&query.wallet)
        .map_err(|_| bad_request("Invalid wallet address"))?;

    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to get USDC mint: {}", e)))?;
    let user_ata = state.solana.get_associated_token_address(&wallet_pubkey, &usdc_mint);

    let blockhash = state.solana.rpc.get_latest_blockhash().await
        .map_err(|e| internal_error(format!("RPC error: {}", e)))?;

    let tx_b64 = delegation.build_revoke_tx(&wallet_pubkey, &user_ata, blockhash)
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(RevokeResponse {
        transaction: tx_b64,
    })))
}

// ─── GET /api/delegation/status ──────────────────────────────────────────────

/// Check delegation status for a wallet — whether delegation is enabled,
/// and how much allowance remains.
pub async fn get_delegation_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DelegationQuery>,
) -> AppResult<DelegationStatus> {
    let delegation = match state.delegation.as_ref() {
        Some(d) => d,
        None => return Ok(Json(ApiResponse::ok(DelegationStatus {
            enabled: false,
            delegate: String::new(),
            delegated_amount: None,
            token_account: None,
        }))),
    };

    let wallet_pubkey = Pubkey::from_str(&query.wallet)
        .map_err(|_| bad_request("Invalid wallet address"))?;

    // Derive user ATA
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to get USDC mint: {}", e)))?;
    let user_ata = state.solana.get_associated_token_address(&wallet_pubkey, &usdc_mint);

    // Try to read account on-chain to check delegated amount
    let delegated_amount = match state.solana.rpc.get_account(&user_ata).await {
        Ok(account) => {
            // SPL Token account layout: ...64 bytes... delegate(32) @ offset 76,
            // then delegated_amount(u64) @ offset 121
            // Actually: offset 72 = delegate option (4 bytes), 76 = delegate pubkey (32),
            // 108 = state (1), 109 = is_native option (4+8), 121 = delegated_amount (8)
            if account.data.len() >= 129 {
                let delegate_option = u32::from_le_bytes(
                    account.data[72..76].try_into().unwrap_or([0; 4])
                );
                if delegate_option == 1 {
                    // Check if delegate matches our platform signer
                    let delegate_bytes: [u8; 32] = account.data[76..108]
                        .try_into().unwrap_or([0; 32]);
                    let on_chain_delegate = Pubkey::new_from_array(delegate_bytes);

                    if on_chain_delegate == delegation.pubkey() {
                        let amount_bytes: [u8; 8] = account.data[121..129]
                            .try_into().unwrap_or([0; 8]);
                        Some(u64::from_le_bytes(amount_bytes))
                    } else {
                        Some(0) // Delegated to someone else
                    }
                } else {
                    Some(0) // No delegation set
                }
            } else {
                None
            }
        }
        Err(_) => None, // Account doesn't exist or RPC error
    };

    Ok(Json(ApiResponse::ok(DelegationStatus {
        enabled: true,
        delegate: delegation.pubkey().to_string(),
        delegated_amount,
        token_account: Some(user_ata.to_string()),
    })))
}
