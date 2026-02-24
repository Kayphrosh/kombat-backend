// app/src/handlers/wager.rs
//! Axum route handlers — build unsigned transactions and return them to
//! the client for wallet signing.  The API is intentionally stateless w.r.t.
//! keys: it never holds a private key.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::{
    models::*,
    services::{DbService, SolanaService},
};
use redis::AsyncCommands;
use serde_json;
use tracing;

// ─── Shared app state ─────────────────────────────────────────────────────────

pub struct AppState {
    pub db:     Arc<DbService>,
    pub solana: Arc<SolanaService>,
    pub notif_tx: Arc<broadcast::Sender<(String, serde_json::Value)>>,
    // Simple in-memory rate limiter for auth nonces: map wallet -> (count, window_start)
    pub nonce_rate: Arc<tokio::sync::Mutex<std::collections::HashMap<String, (u32, chrono::DateTime<chrono::Utc>)>>>,
    // Optional Redis client for cross-instance rate limiting / pubsub
    pub redis_client: Option<Arc<redis::Client>>,
    // Prometheus metrics (optional)
    pub rate_limit_exceeded: Option<std::sync::Arc<prometheus::IntCounter>>,
    pub rate_limit_requests: Option<std::sync::Arc<prometheus::IntCounter>>,
    // Dynamic SDK service for JWT verification (optional — only if DYNAMIC_ENVIRONMENT_ID is set)
    pub dynamic_service: Option<Arc<crate::services::DynamicService>>,
    // Upload service (optional — only if UPLOAD_DIR is set)
    pub upload_service: Option<Arc<crate::services::UploadService>>,
}

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::err(msg)))
}

// ─── GET /wagers ──────────────────────────────────────────────────────────────

pub async fn list_wagers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WagerListQuery>,
) -> AppResult<Vec<WagerRecord>> {
    let wagers = state.db.list_wagers(&query).await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(wagers)))
}

// ─── GET /wagers/:address ─────────────────────────────────────────────────────

pub async fn get_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<WagerRecord> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;
    Ok(Json(ApiResponse::ok(wager)))
}

// ─── Anchor Instruction Structs ────────────────────────────────────────────────

#[derive(borsh::BorshSerialize)]
pub enum ResolutionSource {
    Arbitrator,
    OracleFeed,
    MutualConsent,
}

#[derive(borsh::BorshSerialize)]
pub struct CreateWagerArgs {
    pub description: String,
    pub stake_lamports: u64,
    pub expiry_ts: i64,
    pub resolution_source: ResolutionSource,
    pub resolver: Pubkey,
    pub oracle_feed: Option<Pubkey>,
    pub oracle_target: Option<i64>,
    pub oracle_initiator_wins_above: Option<bool>,
}

// ─── POST /wagers ─────────────────────────────────────────────────────────────
/// Returns an unsigned transaction the client must sign with the initiator's wallet.

pub async fn create_wager(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWagerRequest>,
) -> AppResult<TxResponse> {
    // ── Validate ──────────────────────────────────────────────────────────────
    if req.description.len() > 256 {
        return Err(bad_request("Description exceeds 256 characters"));
    }
    if req.stake_lamports == 0 {
        return Err(bad_request("Stake must be > 0"));
    }

    let initiator = Pubkey::from_str(&req.initiator)
        .map_err(|_| bad_request("Invalid initiator pubkey"))?;

    // ── Look up current wager_id from registry ─────────────────────────────
    // In production: fetch the on-chain WagerRegistry account via RPC.
    // For scaffold, default to 0.
    let wager_id: u64 = 0;

    // ── Borsh-serialize the instruction args ──────────────────────────────────
    let resolution_source = match req.resolution_source.to_lowercase().as_str() {
        "oracle" | "oraclefeed" => ResolutionSource::OracleFeed,
        "mutual" | "mutualconsent" => ResolutionSource::MutualConsent,
        _ => ResolutionSource::Arbitrator, // defaults to Arbitrator (manual)
    };

    let resolver = Pubkey::from_str(&req.resolver)
        .unwrap_or_else(|_| initiator); // fallback to initiator or config treasury if parsing fails

    let oracle_feed = req.oracle_feed.as_deref()
        .and_then(|f| Pubkey::from_str(f).ok());

    let args = CreateWagerArgs {
        description: req.description.clone(),
        stake_lamports: req.stake_lamports,
        expiry_ts: req.expiry_ts,
        resolution_source,
        resolver,
        oracle_feed,
        oracle_target: req.oracle_target,
        oracle_initiator_wins_above: req.oracle_initiator_wins_above,
    };

    let args_data = borsh::to_vec(&args)
        .map_err(|e| internal_error(format!("Borsh serialization failed: {}", e)))?;

    let ix = state.solana.ix_create_wager(&initiator, wager_id, args_data);

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &initiator)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!(
            "Create wager: '{}' for {} lamports",
            req.description, req.stake_lamports
        ),
    })))
}

// ─── POST /wagers/:address/accept ─────────────────────────────────────────────

pub async fn accept_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<AcceptWagerRequest>,
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let initiator  = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Stored initiator pubkey is invalid"))?;
    let challenger = Pubkey::from_str(&req.challenger)
        .map_err(|_| bad_request("Invalid challenger pubkey"))?;

    let ix = state.solana.ix_accept_wager(
        &initiator,
        wager.wager_id as u64,
        &challenger,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &challenger)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Try to persist a notification for the initiator (fire-and-forget)
    let initiator_wallet = wager.initiator.clone();
    let addr_clone = address.clone();
    let challenger_clone = req.challenger.clone();
    let db = state.db.clone();
    let notif_tx = state.notif_tx.clone();
    let redis_client = state.redis_client.clone();
    tokio::spawn(async move {
        let payload = serde_json::json!({ "wager_address": addr_clone, "challenger": challenger_clone });
        // persist
        if let Err(e) = db.create_notification(&initiator_wallet, "wager_accepted", Some(payload.clone())).await {
            tracing::error!("failed to create notification: {}", e);
        }
        // publish to realtime channel (ignore send error)
        let _ = notif_tx.send((initiator_wallet.clone(), payload.clone()));
        // publish to Redis pub/sub if available
        if let Some(rc) = &redis_client {
            if let Ok(mut conn) = rc.get_tokio_connection().await {
                let msg = serde_json::json!({ "to": initiator_wallet.clone(), "payload": payload }).to_string();
                let _ : Result<i64, _> = conn.publish("notifications", msg).await;
            }
        }
    });

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Accept wager #{}", wager.wager_id),
    })))
}

// ─── POST /wagers/:address/cancel ────────────────────────────────────────────

pub async fn cancel_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let initiator = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Stored initiator pubkey invalid"))?;

    let ix = state.solana.ix_cancel_wager(&initiator, wager.wager_id as u64);

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &initiator)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Cancel wager #{}", wager.wager_id),
    })))
}

// ─── POST /wagers/:address/resolve ────────────────────────────────────────────

pub async fn resolve_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<ResolveWagerRequest>,
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let initiator = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Invalid initiator pubkey"))?;
    let winner   = Pubkey::from_str(&req.winner)
        .map_err(|_| bad_request("Invalid winner pubkey"))?;
    let resolver = Pubkey::from_str(&req.caller)
        .map_err(|_| bad_request("Invalid caller pubkey"))?;
    // In production, look up treasury from config PDA
    let treasury = resolver;

    let ix = state.solana.ix_resolve_by_arbitrator(
        &initiator,
        wager.wager_id as u64,
        &winner,
        &resolver,
        &treasury,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &resolver)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Notify participants about the resolution
    let db = state.db.clone();
    let addr_clone = address.clone();
    let winner_clone = req.winner.clone();
    let initiator_wallet = wager.initiator.clone();
    let challenger_wallet = wager.challenger.clone();

    let payload = serde_json::json!({ "wager_address": addr_clone, "winner": winner_clone });
    // notify initiator and publish
    {
        let db = db.clone();
        let payload = payload.clone();
        let notif_tx = state.notif_tx.clone();
        let redis_client = state.redis_client.clone();
        let initiator_wallet = initiator_wallet.clone();
        tokio::spawn(async move {
            if let Err(e) = db.create_notification(&initiator_wallet, "wager_resolved", Some(payload.clone())).await {
                tracing::error!("failed to create resolve notification (initiator): {}", e);
            }
            let _ = notif_tx.send((initiator_wallet.clone(), payload.clone()));
            if let Some(rc) = &redis_client {
                if let Ok(mut conn) = rc.get_tokio_connection().await {
                    let msg = serde_json::json!({ "to": initiator_wallet.clone(), "payload": payload.clone() }).to_string();
                    let _ : Result<i64, _> = conn.publish("notifications", msg).await;
                }
            }
        });
    }
    // notify challenger if present
    if let Some(ch) = challenger_wallet {
        let db = db.clone();
        let payload = serde_json::json!({ "wager_address": address.clone(), "winner": req.winner.clone() });
        let notif_tx = state.notif_tx.clone();
        let redis_client = state.redis_client.clone();
        tokio::spawn(async move {
            if let Err(e) = db.create_notification(&ch, "wager_resolved", Some(payload.clone())).await {
                tracing::error!("failed to create resolve notification (challenger): {}", e);
            }
            let _ = notif_tx.send((ch.clone(), payload.clone()));
            if let Some(rc) = &redis_client {
                if let Ok(mut conn) = rc.get_tokio_connection().await {
                    let msg = serde_json::json!({ "to": ch.clone(), "payload": payload.clone() }).to_string();
                    let _ : Result<i64, _> = conn.publish("notifications", msg).await;
                }
            }
        });
    }

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Resolve wager #{} — winner: {}", wager.wager_id, req.winner),
    })))
}

// ─── POST /wagers/:address/dispute ────────────────────────────────────────────

pub async fn dispute_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<DisputeRequest>,
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let initiator   = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Invalid initiator pubkey"))?;
    let participant = Pubkey::from_str(&req.opener)
        .map_err(|_| bad_request("Invalid opener pubkey"))?;

    let ix = state.solana.ix_open_dispute(
        &initiator,
        wager.wager_id as u64,
        &participant,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &participant)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Notify the other participant about the dispute (if present)
    let opener = req.opener.clone();
    let db = state.db.clone();
    let addr_clone = address.clone();
    let maybe_other = if opener == wager.initiator {
        wager.challenger.clone()
    } else {
        Some(wager.initiator.clone())
    };

    if let Some(other_wallet) = maybe_other {
        let payload = serde_json::json!({ "wager_address": addr_clone, "opener": opener });
        let notif_tx = state.notif_tx.clone();
        let redis_client = state.redis_client.clone();
        tokio::spawn(async move {
            if let Err(e) = db.create_notification(&other_wallet, "wager_disputed", Some(payload.clone())).await {
                tracing::error!("failed to create dispute notification: {}", e);
            }
            let _ = notif_tx.send((other_wallet.clone(), payload.clone()));
            if let Some(rc) = &redis_client {
                if let Ok(mut conn) = rc.get_tokio_connection().await {
                    let msg = serde_json::json!({ "to": other_wallet.clone(), "payload": payload }).to_string();
                    let _ : Result<i64, _> = conn.publish("notifications", msg).await;
                }
            }
        });
    }

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Open dispute on wager #{}", wager.wager_id),
    })))
}

// ─── POST /wagers/:address/consent ────────────────────────────────────────────

pub async fn consent_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<ConsentRequest>,
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let initiator = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Invalid initiator pubkey"))?;
    let participant = Pubkey::from_str(&req.participant)
        .map_err(|_| bad_request("Invalid participant pubkey"))?;
    let declared_winner = Pubkey::from_str(&req.declared_winner)
        .map_err(|_| bad_request("Invalid declared_winner pubkey"))?;

    let ix = state.solana.ix_consent_resolve(
        &initiator,
        wager.wager_id as u64,
        &participant,
        &declared_winner,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &participant)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Consent resolve wager #{} — declared winner: {}", wager.wager_id, req.declared_winner),
    })))
}