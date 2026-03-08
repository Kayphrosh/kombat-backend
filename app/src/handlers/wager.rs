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
) -> AppResult<Vec<crate::models::WagerDetailResponse>> {
    let wagers = state.db.list_wagers(&query).await
        .map_err(|e| internal_error(e.to_string()))?;

    // Enrich each wager with participant names/avatars
    let mut enriched = Vec::with_capacity(wagers.len());
    for wager in wagers {
        let initiator_user = state.db.get_user(&wager.initiator).await.ok().flatten();
        let (initiator_name, initiator_avatar) = match initiator_user {
            Some(u) => (u.display_name, u.avatar_url),
            None => (None, None),
        };

        let (challenger_name, challenger_avatar) = if let Some(ref ch) = wager.challenger {
            match state.db.get_user(ch).await.ok().flatten() {
                Some(u) => (u.display_name, u.avatar_url),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        let challenger_option = wager.initiator_option.as_ref().map(|opt| {
            if opt.to_lowercase() == "yes" { "no".to_string() } else { "yes".to_string() }
        });

        enriched.push(crate::models::WagerDetailResponse {
            wager,
            initiator_name,
            initiator_avatar,
            challenger_name,
            challenger_avatar,
            challenger_option,
        });
    }

    Ok(Json(ApiResponse::ok(enriched)))
}

// ─── GET /wagers/:address ─────────────────────────────────────────────────────

pub async fn get_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<crate::models::WagerDetailResponse> {
    let detail = state.db.get_wager_with_users(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;
    Ok(Json(ApiResponse::ok(detail)))
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
    pub stake_usdc: u64,
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
    if req.stake_usdc == 0 {
        return Err(bad_request("Stake must be > 0"));
    }

    let initiator = Pubkey::from_str(&req.initiator)
        .map_err(|_| bad_request("Invalid initiator pubkey"))?;

    // ── Check if the user's WagerRegistry exists on-chain ──────────────────
    let (registry_pda, _) = state.solana.registry_pda(&initiator);
    let registry_account = state.solana.rpc.get_account(&registry_pda).await;

    let mut instructions = Vec::new();
    let wager_id: u64;

    match registry_account {
        Ok(account) => {
            // Registry exists — read wager_count from the account data
            // Layout: 8 (discriminator) + 1 (bump) + 32 (authority) + 8 (wager_count)
            let data = &account.data;
            if data.len() >= 49 {
                let count_bytes: [u8; 8] = data[41..49].try_into()
                    .map_err(|_| internal_error("Failed to read wager_count from registry"))?;
                wager_id = u64::from_le_bytes(count_bytes);
            } else {
                wager_id = 0;
            }
        }
        Err(_) => {
            // Registry doesn't exist — prepend an initialize_registry instruction
            tracing::info!("Registry not found for {}, will auto-initialize", initiator);
            let init_ix = state.solana.ix_initialize_registry(&initiator);
            instructions.push(init_ix);
            wager_id = 0;
        }
    }

    // ── Borsh-serialize the instruction args ──────────────────────────────────
    let resolution_source = match req.resolution_source.to_lowercase().as_str() {
        "oracle" | "oraclefeed" => ResolutionSource::OracleFeed,
        "arbitrator" => ResolutionSource::Arbitrator,
        _ => ResolutionSource::MutualConsent, // default to MutualConsent so both parties can declare winner
    };

    let resolver = Pubkey::from_str(&req.resolver)
        .unwrap_or_else(|_| initiator); // fallback to initiator

    let oracle_feed = req.oracle_feed.as_deref()
        .and_then(|f| Pubkey::from_str(f).ok());

    let args = CreateWagerArgs {
        description: req.description.clone(),
        stake_usdc: req.stake_usdc,
        expiry_ts: req.expiry_ts,
        resolution_source,
        resolver,
        oracle_feed,
        oracle_target: req.oracle_target,
        oracle_initiator_wins_above: req.oracle_initiator_wins_above,
    };

    let args_data = borsh::to_vec(&args)
        .map_err(|e| internal_error(format!("Borsh serialization failed: {}", e)))?;

    // Fetch the USDC mint from on-chain config
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive the initiator's USDC Associated Token Account
    let initiator_token_account = state.solana.get_associated_token_address(&initiator, &usdc_mint);

    let ix = state.solana.ix_create_wager(
        &initiator,
        wager_id,
        &usdc_mint,
        &initiator_token_account,
        args_data,
    );
    instructions.push(ix);

    let tx_b64 = state.solana
        .build_transaction(instructions, &initiator)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // ── Store a pending wager record in the DB immediately ────────────────────
    let (wager_pda, _) = state.solana.wager_pda(&initiator, wager_id);
    let on_chain_address = wager_pda.to_string();

    let wager_record = crate::models::WagerRecord {
        id: uuid::Uuid::new_v4(),
        on_chain_address: on_chain_address.clone(),
        wager_id: wager_id as i64,
        initiator: req.initiator.clone(),
        challenger: req.challenger_address.clone(),
        stake_usdc: req.stake_usdc as i64,
        description: req.description.clone(),
        status: "pending".to_string(),
        resolution_source: req.resolution_source.clone(),
        resolver: req.resolver.clone(),
        expiry_ts: req.expiry_ts,
        created_at: chrono::Utc::now(),
        resolved_at: None,
        winner: None,
        protocol_fee_bps: 100, // 1% default
        oracle_feed: req.oracle_feed.clone(),
        oracle_target: req.oracle_target,
        dispute_opened_at: None,
        dispute_opener: None,
        initiator_option: req.initiator_option.clone(),
        creator_declared_winner: None,
        challenger_declared_winner: None,
    };

    // Fire-and-forget DB insert + push notification
    let db = state.db.clone();
    let notif_tx = state.notif_tx.clone();
    let challenger_addr = req.challenger_address.clone();
    let desc_clone = req.description.clone();
    tokio::spawn(async move {
        // Insert wager record
        if let Err(e) = db.upsert_wager(&wager_record).await {
            tracing::error!("Failed to insert pending wager: {}", e);
        }

        // Notify the challenged user (if specified)
        if let Some(ref challenger) = challenger_addr {
            let payload = serde_json::json!({
                "wager_address": on_chain_address,
                "initiator": wager_record.initiator,
                "description": desc_clone,
                "stake_usdc": wager_record.stake_usdc,
            });

            // In-app notification
            if let Err(e) = db.create_notification(challenger, "wager_challenge", Some(payload.clone())).await {
                tracing::error!("Failed to create challenge notification: {}", e);
            }
            let _ = notif_tx.send((challenger.clone(), payload.clone()));

            // Push notification via Expo
            match db.get_push_tokens(challenger).await {
                Ok(tokens) if !tokens.is_empty() => {
                    let title = "New Kombat Challenge!".to_string();
                    let body = format!("You've been challenged: \"{}\" for {} USDC",
                        desc_clone,
                        wager_record.stake_usdc as f64 / 1_000_000.0
                    );
                    tokio::spawn(async move {
                        if let Err(e) = crate::services::push::send_expo_push(&tokens, &title, &body, Some(payload)).await {
                            tracing::error!("Failed to send push notification: {}", e);
                        }
                    });
                }
                _ => {}
            }
        }
    });

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!(
            "Create wager: '{}' for {} micro-USDC",
            req.description, req.stake_usdc
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

    // Fetch the USDC mint from on-chain config
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive the challenger's USDC Associated Token Account
    let challenger_token_account = state.solana.get_associated_token_address(&challenger, &usdc_mint);

    let ix = state.solana.ix_accept_wager(
        &initiator,
        wager.wager_id as u64,
        &challenger,
        &usdc_mint,
        &challenger_token_account,
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
        // Update status to active
        if let Err(e) = db.update_wager_status(&addr_clone, "active").await {
            tracing::error!("failed to update wager status to active: {}", e);
        }
        // persist notification
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

    // Fetch the USDC mint from on-chain config
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive the initiator's USDC Associated Token Account
    let initiator_token_account = state.solana.get_associated_token_address(&initiator, &usdc_mint);

    let ix = state.solana.ix_cancel_wager(
        &initiator,
        wager.wager_id as u64,
        &usdc_mint,
        &initiator_token_account,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &initiator)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Cancel wager #{}", wager.wager_id),
    })))
}

// ─── POST /wagers/:address/decline ───────────────────────────────────────────
/// Decline a wager challenge — called by the challenged user.

pub async fn decline_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<AcceptWagerRequest>, // reuses { challenger } field
) -> AppResult<TxResponse> {
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let challenger = Pubkey::from_str(&req.challenger)
        .map_err(|_| bad_request("Invalid challenger pubkey"))?;

    let initiator = Pubkey::from_str(&wager.initiator)
        .map_err(|_| internal_error("Stored initiator pubkey invalid"))?;

    // Fetch the USDC mint from on-chain config
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive the initiator's USDC Associated Token Account (they'll receive the refund)
    let initiator_token_account = state.solana.get_associated_token_address(&initiator, &usdc_mint);

    // Build a cancel ix — the challenger declines
    let ix = state.solana.ix_cancel_wager(
        &initiator,
        wager.wager_id as u64,
        &usdc_mint,
        &initiator_token_account,
    );

    let tx_b64 = state.solana
        .build_transaction(vec![ix], &challenger)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Mark as declined in DB immediately
    let db = state.db.clone();
    let addr = address.clone();
    tokio::spawn(async move {
        if let Err(e) = db.update_wager_status(&addr, "declined").await {
            tracing::error!("Failed to update wager status to declined: {}", e);
        }
    });

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Decline wager #{}", wager.wager_id),
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
    
    // Read the treasury and USDC mint from on-chain ProtocolConfig
    let treasury = state.solana.get_treasury().await
        .map_err(|e| internal_error(format!("Failed to read treasury: {}", e)))?;
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive token accounts
    let winner_token_account = state.solana.get_associated_token_address(&winner, &usdc_mint);
    let treasury_token_account = state.solana.get_associated_token_address(&treasury, &usdc_mint);

    // Ensure winner and treasury ATAs exist — prepend create instructions if missing
    let ata_ixs = state.solana
        .ensure_atas_exist(&resolver, &usdc_mint, &[&winner, &treasury])
        .await
        .map_err(|e| internal_error(format!("Failed to check ATAs: {}", e)))?;

    let ix = state.solana.ix_resolve_by_arbitrator(
        &initiator,
        wager.wager_id as u64,
        &usdc_mint,
        &winner_token_account,
        &treasury_token_account,
        &resolver,
    );

    // Prepend ATA creation instructions before the main instruction
    let mut all_ixs = ata_ixs;
    all_ixs.push(ix);

    let tx_b64 = state.solana
        .build_transaction(all_ixs, &resolver)
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

    // Fire-and-forget: save opener's dispute submission + notify other party
    let opener = req.opener.clone();
    let description = req.description.clone();
    let evidence_url = req.evidence_url.clone();
    let db = state.db.clone();
    let addr_clone = address.clone();
    let maybe_other = if opener == wager.initiator {
        wager.challenger.clone()
    } else {
        Some(wager.initiator.clone())
    };

    let notif_tx = state.notif_tx.clone();
    let redis_client = state.redis_client.clone();
    tokio::spawn(async move {
        // Save the opener's dispute submission if they provided a description
        if let Some(desc) = &description {
            if let Err(e) = db.upsert_dispute_submission(
                &addr_clone,
                &opener,
                desc,
                evidence_url.as_deref(),
                None, // declared_winner not set at open time
            ).await {
                tracing::error!("failed to save opener dispute submission: {}", e);
            }
        }

        // Notify the other participant about the dispute
        if let Some(other_wallet) = maybe_other {
            let payload = serde_json::json!({ "wager_address": addr_clone, "opener": opener });
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
        }
    });

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: format!("Open dispute on wager #{}", wager.wager_id),
    })))
}

// ─── POST /wagers/:address/dispute/submit ─────────────────────────────────────
/// Submit or update a dispute form (description, evidence, declared winner).
/// Both participants should call this endpoint.

pub async fn submit_dispute_form(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<DisputeSubmissionRequest>,
) -> AppResult<crate::models::DisputeSubmissionRecord> {
    // Verify wager exists and is in disputed state
    let wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    if wager.status != "disputed" {
        return Err(bad_request("Wager is not in disputed state"));
    }

    // Verify submitter is a participant
    let is_participant = req.submitter == wager.initiator
        || wager.challenger.as_deref() == Some(&req.submitter);
    if !is_participant {
        return Err(bad_request("Only wager participants can submit dispute forms"));
    }

    let record = state.db.upsert_dispute_submission(
        &address,
        &req.submitter,
        &req.description,
        req.evidence_url.as_deref(),
        req.declared_winner.as_deref(),
    ).await.map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(record)))
}

// ─── GET /wagers/:address/dispute ─────────────────────────────────────────────
/// Fetch both parties' dispute submissions for a wager.

pub async fn get_dispute_submissions(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<Vec<crate::models::DisputeSubmissionRecord>> {
    let _wager = state.db.get_wager_by_address(&address).await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(ApiResponse::err("Wager not found"))))?;

    let submissions = state.db.get_dispute_submissions(&address).await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(submissions)))
}

// ─── POST /wagers/:address/declare-winner ─────────────────────────────────────
/// Smart routing based on resolution_source AND who is calling:
///   - Arbitrator wagers: only the resolver can call → resolve_by_arbitrator
///   - MutualConsent wagers: either participant → consent_resolve (auto-pays when both agree)

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

    // Read the treasury and USDC mint from on-chain ProtocolConfig
    let treasury = state.solana.get_treasury().await
        .map_err(|e| internal_error(format!("Failed to read treasury: {}", e)))?;
    let usdc_mint = state.solana.get_usdc_mint().await
        .map_err(|e| internal_error(format!("Failed to fetch USDC mint from config: {}", e)))?;
    
    // Derive token accounts
    let winner_token_account = state.solana.get_associated_token_address(&declared_winner, &usdc_mint);
    let treasury_token_account = state.solana.get_associated_token_address(&treasury, &usdc_mint);

    // Ensure winner and treasury ATAs exist — prepend create instructions if missing
    let ata_ixs = state.solana
        .ensure_atas_exist(&participant, &usdc_mint, &[&declared_winner, &treasury])
        .await
        .map_err(|e| internal_error(format!("Failed to check ATAs: {}", e)))?;

    let resolution = wager.resolution_source.to_lowercase();
    let resolver_str = wager.resolver.clone();
    let caller_is_resolver = req.participant == resolver_str;

    let (ix, desc) = if resolution == "arbitrator" || resolution == "manual" {
        // Arbitrator wager — only the designated resolver can resolve
        if !caller_is_resolver {
            return Err(bad_request(format!(
                "Only the resolver ({}) can declare the winner for arbitrator wagers. \
                 You ({}) are not the resolver.",
                resolver_str, req.participant
            )));
        }
        let ix = state.solana.ix_resolve_by_arbitrator(
            &initiator,
            wager.wager_id as u64,
            &usdc_mint,
            &winner_token_account,
            &treasury_token_account,
            &participant,  // participant IS the resolver
        );
        (ix, format!("Resolve wager #{} — winner: {}", wager.wager_id, req.declared_winner))
    } else {
        // MutualConsent — either participant can consent, auto-pays when both agree
        let ix = state.solana.ix_consent_resolve(
            &initiator,
            wager.wager_id as u64,
            &usdc_mint,
            &participant,
            &winner_token_account,
            &treasury_token_account,
        );
        (ix, format!("Consent resolve wager #{} — declared winner: {}", wager.wager_id, req.declared_winner))
    };

    // Prepend ATA creation instructions before the main instruction
    let mut all_ixs = ata_ixs;
    all_ixs.push(ix);

    let tx_b64 = state.solana
        .build_transaction(all_ixs, &participant)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    // Fire-and-forget: track who declared which winner in the DB
    let db = state.db.clone();
    let addr = address.clone();
    let is_initiator = req.participant == wager.initiator;
    let winner_str = req.declared_winner.clone();
    tokio::spawn(async move {
        match db.set_declared_winner(&addr, is_initiator, &winner_str).await {
            Ok(Some(agreed_winner)) => {
                tracing::info!("Both parties agreed — wager {} resolved, winner: {}", addr, agreed_winner);
            }
            Ok(None) => {
                tracing::info!("Declared winner recorded for wager {}", addr);
            }
            Err(e) => {
                tracing::error!("Failed to store declared winner: {}", e);
            }
        }
    });

    Ok(Json(ApiResponse::ok(TxResponse {
        transaction_b64: tx_b64,
        description: desc,
    })))
}