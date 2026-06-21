//! P2P (1-v-1) wager endpoints.
//!
//! The on-chain wager object is created and settled client-side with the user's
//! wallet. This backend **indexes** wagers and owns the off-chain social layer:
//! accepting a challenge, declaring winners (auto-resolves when both agree),
//! disputes, and win/loss stats.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use std::sync::Arc;

use crate::{
    models::{
        AcceptWagerRequest, ApiResponse, CancelWagerRequest, ConsentRequest, CreateWagerRequest,
        CreateWalrusArtifactRequest, DeclineWagerRequest, DisputeSubmissionRecord,
        DisputeSubmissionRequest, MineWagersQuery, PaymentMoveCall, PaymentPtbArgument,
        PaymentPtbStep, UpdateWagerStatusRequest, WagerCreatePtbRequest, WagerDetailResponse,
        WagerListQuery, WagerPtbResponse, WagerRecord, WagerResolvePtbQuery, WalrusArtifactRecord,
    },
    services::sui::SuiService,
    state::AppState,
};
use serde_json::{json, Value as JsonValue};

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

/// Allowed wager lifecycle statuses the client may set directly.
const SETTABLE_STATUSES: &[&str] = &["open", "active", "cancelled", "declined", "expired"];

fn normalize(addr: &str) -> Option<String> {
    SuiService::normalize_address(addr)
}

// ─── POST /api/wagers ──────────────────────────────────────────────────────────

/// Index a newly created on-chain wager.
pub async fn create_wager(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWagerRequest>,
) -> AppResult<WagerDetailResponse> {
    if req.on_chain_address.trim().is_empty() {
        return Err(bad_request("on_chain_address is required"));
    }
    if req.description.trim().is_empty() {
        return Err(bad_request("description is required"));
    }
    let initiator =
        normalize(&req.initiator).ok_or_else(|| bad_request("Invalid initiator address"))?;
    let challenger = match req.challenger_address.as_ref() {
        Some(c) => Some(normalize(c).ok_or_else(|| bad_request("Invalid challenger address"))?),
        None => None,
    };

    // Default new wagers to mutual-consent resolution so both parties can
    // declare a winner. Only fall back to the original when explicitly set to
    // a non-empty value.
    let resolution_source = {
        let v = req.resolution_source.trim();
        if v.is_empty() {
            "mutual_consent".to_string()
        } else {
            v.to_string()
        }
    };

    let record = WagerRecord {
        id: uuid::Uuid::new_v4(),
        on_chain_address: req.on_chain_address.clone(),
        wager_id: req.wager_id,
        initiator,
        challenger,
        stake_usdc: req.stake_usdc as i64,
        description: req.description.clone(),
        status: "open".to_string(),
        resolution_source: resolution_source.clone(),
        resolver: req.resolver.clone(),
        expiry_ts: req.expiry_ts,
        created_at: Utc::now(),
        resolved_at: None,
        winner: None,
        protocol_fee_bps: req.protocol_fee_bps.unwrap_or(0),
        oracle_feed: req.oracle_feed.clone(),
        oracle_target: req.oracle_target,
        dispute_opened_at: None,
        dispute_opener: None,
        initiator_option: req.initiator_option.clone(),
        creator_declared_winner: None,
        challenger_declared_winner: None,
        resolution_error: None,
        resolution_attempted_at: None,
    };

    state
        .db
        .upsert_wager(&record)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    // Durably store wager terms/agreement on Walrus (best-effort).
    if let Some(terms) = req.terms.as_ref() {
        store_wager_artifact(
            &state,
            &record.on_chain_address,
            "wager_terms",
            terms,
            json!({ "kind": "terms" }),
        )
        .await;
    }

    fetch_detail(&state, &record.on_chain_address).await
}

// ─── GET /api/wagers ───────────────────────────────────────────────────────────

pub async fn list_wagers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WagerListQuery>,
) -> AppResult<Vec<WagerDetailResponse>> {
    let wagers = state
        .db
        .list_wagers_enriched(&query, None)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(wagers)))
}

// ─── GET /api/wagers/mine ──────────────────────────────────────────────────────

pub async fn list_my_wagers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MineWagersQuery>,
) -> AppResult<Vec<WagerDetailResponse>> {
    let wallet = normalize(&query.wallet).ok_or_else(|| bad_request("Invalid wallet address"))?;
    let wagers = state
        .db
        .list_my_wagers(&wallet, query.limit, query.offset)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(wagers)))
}

// ─── GET /api/wagers/:address ──────────────────────────────────────────────────

pub async fn get_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<WagerDetailResponse> {
    fetch_detail(&state, &address).await
}

// ─── POST /api/wagers/:address/accept ──────────────────────────────────────────

/// Challenger accepts an open wager.
pub async fn accept_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<AcceptWagerRequest>,
) -> AppResult<WagerDetailResponse> {
    let challenger =
        normalize(&req.challenger).ok_or_else(|| bad_request("Invalid challenger address"))?;

    let mut wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    if wager.initiator == challenger {
        return Err(bad_request("Initiator cannot accept their own wager"));
    }
    if let Some(named_challenger) = wager.challenger.as_deref() {
        if named_challenger != challenger {
            return Err(bad_request(
                "Only the named challenger can accept this wager",
            ));
        }
    }
    if wager.status != "open" {
        return Err(bad_request(format!(
            "Wager is {} and cannot be accepted",
            wager.status
        )));
    }

    wager.challenger = Some(challenger);
    wager.status = "active".to_string();

    state
        .db
        .upsert_wager(&wager)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    fetch_detail(&state, &address).await
}

// ─── POST /api/wagers/:address/cancel ──────────────────────────────────────────

/// Initiator records a signed on-chain cancel/refund for an open wager.
pub async fn cancel_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<CancelWagerRequest>,
) -> AppResult<WagerDetailResponse> {
    let initiator =
        normalize(&req.initiator).ok_or_else(|| bad_request("Invalid initiator address"))?;

    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    if wager.initiator != initiator {
        return Err(bad_request("Only the initiator can cancel this wager"));
    }
    if wager.status != "open" {
        return Err(bad_request(format!(
            "Wager is {} and cannot be cancelled",
            wager.status
        )));
    }

    state
        .db
        .update_wager_status(&address, "cancelled")
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    fetch_detail(&state, &address).await
}

// ─── POST /api/wagers/:address/decline ─────────────────────────────────────────

/// Named challenger declines the social invite. The initiator must still cancel
/// on-chain to reclaim escrowed funds.
pub async fn decline_wager(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<DeclineWagerRequest>,
) -> AppResult<WagerDetailResponse> {
    let challenger =
        normalize(&req.challenger).ok_or_else(|| bad_request("Invalid challenger address"))?;

    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    if wager.challenger.as_deref() != Some(challenger.as_str()) {
        return Err(bad_request(
            "Only the named challenger can decline this wager",
        ));
    }
    if wager.status != "open" {
        return Err(bad_request(format!(
            "Wager is {} and cannot be declined",
            wager.status
        )));
    }

    state
        .db
        .update_wager_status(&address, "declined")
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    fetch_detail(&state, &address).await
}

// ─── POST /api/wagers/:address/status ──────────────────────────────────────────

pub async fn update_wager_status(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<UpdateWagerStatusRequest>,
) -> AppResult<WagerDetailResponse> {
    if !SETTABLE_STATUSES.contains(&req.status.as_str()) {
        return Err(bad_request(format!(
            "status must be one of: {}",
            SETTABLE_STATUSES.join(", ")
        )));
    }
    state
        .db
        .update_wager_status(&address, &req.status)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    fetch_detail(&state, &address).await
}

// ─── POST /api/wagers/:address/declare-winner ──────────────────────────────────

#[derive(serde::Serialize)]
pub struct DeclareWinnerResponse {
    /// Set when both participants agreed and the wager auto-resolved (off-chain).
    pub resolved_winner: Option<String>,
    /// Set when the backend also resolved the wager on-chain and paid the winner.
    pub onchain_resolve_tx: Option<String>,
    /// Set when backend attempted on-chain resolution but the resolver signer could not complete it.
    pub onchain_resolve_error: Option<String>,
    pub wager: WagerDetailResponse,
}

/// A participant declares the winner. When both sides declare the same wallet,
/// the wager auto-resolves and win/loss stats are recorded.
pub async fn declare_winner(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<ConsentRequest>,
) -> AppResult<DeclareWinnerResponse> {
    let participant =
        normalize(&req.participant).ok_or_else(|| bad_request("Invalid participant address"))?;
    let declared_winner = normalize(&req.declared_winner)
        .ok_or_else(|| bad_request("Invalid declared_winner address"))?;

    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    let is_initiator = wager.initiator == participant;
    let is_challenger = wager.challenger.as_deref() == Some(participant.as_str());
    if !is_initiator && !is_challenger {
        return Err(bad_request("participant is not part of this wager"));
    }
    // declared_winner must be one of the two participants
    if declared_winner != wager.initiator
        && Some(declared_winner.as_str()) != wager.challenger.as_deref()
    {
        return Err(bad_request(
            "declared_winner must be a participant of the wager",
        ));
    }

    let resolved_winner = state
        .db
        .set_declared_winner(&address, is_initiator, &declared_winner)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    // When both sides agreed, attempt the on-chain payout (best-effort). This
    // requires the wager's on-chain `resolver` to be the platform signer and
    // that signer to hold SUI for gas; failures are logged, not fatal.
    let (onchain_resolve_tx, onchain_resolve_error) = if let Some(ref winner) = resolved_winner {
        let network = state.sui.config().active_network.clone();
        match state
            .sui
            .resolve_wager_on_chain(&network, &address, winner)
            .await
        {
            Ok(digest) => {
                tracing::info!(wager = %address, %digest, "Wager resolved on-chain");
                if let Err(e) = state.db.mark_wager_resolution_attempt(&address, None).await {
                    tracing::warn!(wager = %address, "Failed to record resolution success: {}", e);
                }
                (Some(digest), None)
            }
            Err(e) => {
                let message = e.to_string();
                tracing::warn!(wager = %address, "On-chain wager resolve failed: {}", message);
                if let Err(db_error) = state
                    .db
                    .mark_wager_resolution_attempt(&address, Some(&message))
                    .await
                {
                    tracing::warn!(wager = %address, "Failed to record resolution error: {}", db_error);
                }
                (None, Some(message))
            }
        }
    } else {
        (None, None)
    };

    let detail = fetch_detail_raw(&state, &address).await?;
    Ok(Json(ApiResponse::ok(DeclareWinnerResponse {
        resolved_winner,
        onchain_resolve_tx,
        onchain_resolve_error,
        wager: detail,
    })))
}

// ─── Disputes ──────────────────────────────────────────────────────────────────

/// POST /api/wagers/:address/disputes — submit/replace a participant's dispute.
pub async fn submit_dispute(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Json(req): Json<DisputeSubmissionRequest>,
) -> AppResult<DisputeSubmissionRecord> {
    let submitter =
        normalize(&req.submitter).ok_or_else(|| bad_request("Invalid submitter address"))?;
    if req.description.trim().is_empty() {
        return Err(bad_request("description is required"));
    }
    let declared_winner = match req.declared_winner.as_ref() {
        Some(w) => {
            Some(normalize(w).ok_or_else(|| bad_request("Invalid declared_winner address"))?)
        }
        None => None,
    };

    // If structured evidence is supplied, store it durably on Walrus and use the
    // resulting aggregator URL as the evidence_url (falls back to any provided URL).
    let evidence_url = match req.evidence.as_ref() {
        Some(evidence) => store_wager_artifact(
            &state,
            &address,
            "wager_evidence",
            evidence,
            json!({ "kind": "evidence", "submitter": submitter }),
        )
        .await
        .or_else(|| req.evidence_url.clone()),
        None => req.evidence_url.clone(),
    };

    let record = state
        .db
        .upsert_dispute_submission(
            &address,
            &submitter,
            &req.description,
            evidence_url.as_deref(),
            declared_winner.as_deref(),
        )
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    // Mark the wager as disputed so it surfaces in review flows.
    let _ = state.db.update_wager_status(&address, "disputed").await;

    Ok(Json(ApiResponse::ok(record)))
}

/// GET /api/wagers/:address/disputes — list dispute submissions for a wager.
pub async fn list_disputes(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<Vec<DisputeSubmissionRecord>> {
    let rows = state
        .db
        .get_dispute_submissions(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(rows)))
}

// ─── GET /api/wagers/:address/artifacts ────────────────────────────────────────

/// List Walrus artifacts (terms, evidence) tied to a wager.
pub async fn list_wager_artifacts(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<Vec<WalrusArtifactRecord>> {
    let rows = state
        .db
        .list_walrus_artifacts_for_wager(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(Json(ApiResponse::ok(rows)))
}

// ─── PTB builders (backend describes the on-chain transaction) ──────────────────

/// Resolve the configured wager package + coin type for a network, or a reason
/// string explaining why a PTB cannot be built yet.
fn wager_build_context(
    state: &AppState,
    network: &str,
) -> Result<(String, String, String), (Option<String>, String)> {
    let cfg = match state.sui.config().network(network) {
        Some(c) => c,
        None => return Err((Some("unsupported_network".into()), network.to_string())),
    };
    let net = cfg.network.clone();
    let Some(package_id) = cfg.wager_package_id.clone() else {
        return Err((Some("wager_package_not_configured".into()), net));
    };
    let Some(coin_type) = cfg.usdc_coin_type.clone() else {
        return Err((Some("usdc_coin_type_not_configured".into()), net));
    };
    Ok((package_id, coin_type, cfg.wager_module.clone()))
}

/// POST /api/wagers/create-ptb — transaction to create a wager on-chain.
pub async fn create_wager_ptb(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WagerCreatePtbRequest>,
) -> AppResult<WagerPtbResponse> {
    let network = req
        .network
        .clone()
        .unwrap_or_else(|| state.sui.config().active_network.clone());

    let (package_id, coin_type, wager_module, can_build, reason) =
        match wager_build_context(&state, &network) {
            Ok((p, c, m)) => (Some(p), Some(c), Some(m), true, None),
            Err((reason, _)) => (None, None, None, false, reason),
        };

    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        let wager_module = wager_module.unwrap();
        Some(PaymentMoveCall {
            target: format!("{}::{}::create_wager", package_id, wager_module),
            package_id: package_id.clone(),
            module: wager_module,
            function: "create_wager".to_string(),
            type_arguments: vec![coin_type.clone()],
            arguments: vec![
                PaymentPtbArgument {
                    name: "stake".to_string(),
                    kind: "coin".to_string(),
                    value: Some(json!(req.stake_usdc.to_string())),
                    source: "split exact USDC stake from initiator's coins".to_string(),
                },
                PaymentPtbArgument {
                    name: "description".to_string(),
                    kind: "string".to_string(),
                    value: Some(json!(req.description)),
                    source: "request".to_string(),
                },
                PaymentPtbArgument {
                    name: "initiator_option".to_string(),
                    kind: "string_option".to_string(),
                    value: req.initiator_option.clone().map(|o| json!(o)),
                    source: "request (which side the initiator backs)".to_string(),
                },
                PaymentPtbArgument {
                    name: "challenger".to_string(),
                    kind: "address".to_string(),
                    // @0x0 = open to any challenger
                    value: Some(json!(req
                        .challenger_address
                        .clone()
                        .unwrap_or_else(|| "0x0".to_string()))),
                    source: "request (0x0 = open to anyone)".to_string(),
                },
                PaymentPtbArgument {
                    name: "expiry_ms".to_string(),
                    kind: "u64".to_string(),
                    value: Some(json!(req.expiry_ts.to_string())),
                    source: "request (unix milliseconds)".to_string(),
                },
                PaymentPtbArgument {
                    name: "resolver".to_string(),
                    kind: "address".to_string(),
                    value: Some(json!(req.resolver)),
                    source: "request".to_string(),
                },
                PaymentPtbArgument {
                    name: "clock".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!("0x6")),
                    source: "Sui Clock object".to_string(),
                },
            ],
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(WagerPtbResponse {
        wager_address: None,
        network,
        can_build,
        reason,
        coin_type,
        package_id,
        expected_object_type: "Wager".to_string(),
        steps: vec![
            PaymentPtbStep {
                kind: "split_coin".to_string(),
                description: "Split the exact stake amount from the initiator's USDC coins."
                    .to_string(),
            },
            PaymentPtbStep {
                kind: "move_call".to_string(),
                description: "Call wager::create_wager; locks the initiator's stake and shares a Wager object."
                    .to_string(),
            },
            PaymentPtbStep {
                kind: "index".to_string(),
                description: format!(
                    "After execution by initiator {}, POST the resulting object address to /api/wagers to index it.",
                    req.initiator
                ),
            },
        ],
        move_call,
    })))
}

/// GET /api/wagers/:address/accept-ptb — transaction for a challenger to accept.
pub async fn accept_wager_ptb(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<WagerPtbResponse> {
    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    let network = state.sui.config().active_network.clone();
    let (package_id, coin_type, wager_module, can_build, reason) =
        match wager_build_context(&state, &network) {
            Ok((p, c, m)) => (Some(p), Some(c), Some(m), true, None),
            Err((reason, _)) => (None, None, None, false, reason),
        };

    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        let wager_module = wager_module.unwrap();
        Some(PaymentMoveCall {
            target: format!("{}::{}::accept_wager", package_id, wager_module),
            package_id: package_id.clone(),
            module: wager_module,
            function: "accept_wager".to_string(),
            type_arguments: vec![coin_type.clone()],
            arguments: vec![
                PaymentPtbArgument {
                    name: "wager".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!(address)),
                    source: "wagers.on_chain_address".to_string(),
                },
                PaymentPtbArgument {
                    name: "payment".to_string(),
                    kind: "coin".to_string(),
                    value: Some(json!(wager.stake_usdc.to_string())),
                    source: "split matching USDC stake from challenger's coins".to_string(),
                },
                PaymentPtbArgument {
                    name: "clock".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!("0x6")),
                    source: "Sui Clock object".to_string(),
                },
            ],
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(WagerPtbResponse {
        wager_address: Some(address),
        network,
        can_build,
        reason,
        coin_type,
        package_id,
        expected_object_type: "Wager".to_string(),
        steps: vec![
            PaymentPtbStep {
                kind: "split_coin".to_string(),
                description: "Split the matching stake from the challenger's USDC coins."
                    .to_string(),
            },
            PaymentPtbStep {
                kind: "move_call".to_string(),
                description: "Call wager::accept_wager to lock the challenger's stake.".to_string(),
            },
        ],
        move_call,
    })))
}

/// GET /api/wagers/:address/cancel-ptb — transaction for an initiator refund.
pub async fn cancel_wager_ptb(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> AppResult<WagerPtbResponse> {
    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    if wager.status != "open" {
        return Err(bad_request(format!(
            "Wager is {} and cannot be cancelled",
            wager.status
        )));
    }

    let network = state.sui.config().active_network.clone();
    let (package_id, coin_type, wager_module, can_build, reason) =
        match wager_build_context(&state, &network) {
            Ok((p, c, m)) => (Some(p), Some(c), Some(m), true, None),
            Err((reason, _)) => (None, None, None, false, reason),
        };

    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        let wager_module = wager_module.unwrap();
        Some(PaymentMoveCall {
            target: format!("{}::{}::cancel_wager", package_id, wager_module),
            package_id: package_id.clone(),
            module: wager_module,
            function: "cancel_wager".to_string(),
            type_arguments: vec![coin_type.clone()],
            arguments: vec![
                PaymentPtbArgument {
                    name: "wager".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!(address)),
                    source: "wagers.on_chain_address".to_string(),
                },
                PaymentPtbArgument {
                    name: "clock".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!("0x6")),
                    source: "Sui Clock object".to_string(),
                },
            ],
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(WagerPtbResponse {
        wager_address: Some(address),
        network,
        can_build,
        reason,
        coin_type,
        package_id,
        expected_object_type: "Wager".to_string(),
        steps: vec![PaymentPtbStep {
            kind: "move_call".to_string(),
            description: "Call wager::cancel_wager; sender must be the initiator and receives the escrowed stake."
                .to_string(),
        }],
        move_call,
    })))
}

/// GET /api/wagers/:address/resolve-ptb?winner=0x.. — transaction to resolve.
pub async fn resolve_wager_ptb(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
    Query(query): Query<WagerResolvePtbQuery>,
) -> AppResult<WagerPtbResponse> {
    let winner = normalize(&query.winner).ok_or_else(|| bad_request("Invalid winner address"))?;

    let wager = state
        .db
        .get_wager_by_address(&address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))?;

    if winner != wager.initiator && Some(winner.as_str()) != wager.challenger.as_deref() {
        return Err(bad_request("winner must be a participant of the wager"));
    }

    let network = state.sui.config().active_network.clone();
    let (package_id, coin_type, wager_module, can_build, reason) =
        match wager_build_context(&state, &network) {
            Ok((p, c, m)) => (Some(p), Some(c), Some(m), true, None),
            Err((reason, _)) => (None, None, None, false, reason),
        };

    let move_call = if can_build {
        let package_id = package_id.clone().unwrap();
        let coin_type = coin_type.clone().unwrap();
        let wager_module = wager_module.unwrap();
        Some(PaymentMoveCall {
            target: format!("{}::{}::resolve_wager", package_id, wager_module),
            package_id: package_id.clone(),
            module: wager_module,
            function: "resolve_wager".to_string(),
            type_arguments: vec![coin_type.clone()],
            arguments: vec![
                PaymentPtbArgument {
                    name: "wager".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!(address)),
                    source: "wagers.on_chain_address".to_string(),
                },
                PaymentPtbArgument {
                    name: "winner".to_string(),
                    kind: "address".to_string(),
                    value: Some(json!(winner)),
                    source: "query".to_string(),
                },
                PaymentPtbArgument {
                    name: "clock".to_string(),
                    kind: "shared_object".to_string(),
                    value: Some(json!("0x6")),
                    source: "Sui Clock object".to_string(),
                },
            ],
        })
    } else {
        None
    };

    Ok(Json(ApiResponse::ok(WagerPtbResponse {
        wager_address: Some(address),
        network,
        can_build,
        reason,
        coin_type,
        package_id,
        expected_object_type: "Wager".to_string(),
        steps: vec![PaymentPtbStep {
            kind: "move_call".to_string(),
            description: "Call wager::resolve_wager; pays the winner and closes the wager. Sender must be the resolver."
                .to_string(),
        }],
        move_call,
    })))
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Upload a JSON payload to Walrus and index it as an artifact tied to a wager.
/// Returns the aggregator URL when the upload succeeds. Best-effort: if Walrus
/// is unconfigured or the upload fails, returns None and the caller proceeds.
async fn store_wager_artifact(
    state: &AppState,
    wager_address: &str,
    artifact_type: &str,
    payload: &JsonValue,
    extra_metadata: JsonValue,
) -> Option<String> {
    if !state.walrus.config().configured() {
        return None;
    }
    let stored = match state.walrus.store_json(payload).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Walrus upload failed for wager {}: {}", wager_address, e);
            return None;
        }
    };
    let mut metadata = json!({ "wager_address": wager_address });
    if let (Some(obj), Some(extra)) = (metadata.as_object_mut(), extra_metadata.as_object()) {
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
    }
    let req = CreateWalrusArtifactRequest {
        artifact_type: artifact_type.to_string(),
        owner_wallet: None,
        match_id: None,
        outcome_proposal_id: None,
        content_type: Some("application/json".to_string()),
        manifest: payload.clone(),
        metadata: Some(metadata),
    };
    let url = stored.aggregator_url.clone();
    if let Err(e) = state.db.create_walrus_artifact(&req, &stored).await {
        tracing::warn!("Failed to index wager artifact: {}", e);
    }
    url
}

async fn fetch_detail_raw(
    state: &AppState,
    address: &str,
) -> Result<WagerDetailResponse, (StatusCode, Json<ApiResponse<()>>)> {
    state
        .db
        .get_wager_with_users(address)
        .await
        .map_err(|e| internal_error(e.to_string()))?
        .ok_or_else(|| not_found("Wager not found"))
}

async fn fetch_detail(state: &AppState, address: &str) -> AppResult<WagerDetailResponse> {
    let detail = fetch_detail_raw(state, address).await?;
    Ok(Json(ApiResponse::ok(detail)))
}
