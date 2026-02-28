// app/src/services/indexer.rs
//! On-chain event indexer — polls the Solana RPC for wager program transactions
//! and syncs state into the PostgreSQL database.

use anyhow::Result;
use std::sync::Arc;
use tokio::time::{Duration, interval};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
};
use crate::services::db::DbService;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;

pub struct IndexerService {
    db: Arc<DbService>,
    rpc_url: String,
    program_id: String,
}

impl IndexerService {
    pub fn new(db: Arc<DbService>, rpc_url: String, program_id: String) -> Self {
        Self { db, rpc_url, program_id }
    }

    /// Start polling loop — call with tokio::spawn
    pub async fn run(&self) {
        let mut ticker = interval(Duration::from_secs(2));
        tracing::info!("Starting Indexer for program: {}", self.program_id);
        
        // Define event discriminators once
        let event_names = [
            "WagerCreated",
            "WagerAccepted", 
            "WagerResolved",
            "WagerCancelled", 
            "WagerDisputed"
        ];
        let discriminators: Vec<[u8; 8]> = event_names.iter().map(|n| event_discriminator(n)).collect();

        // TODO: Persist last_signature in DB so we don't start from scratch or miss events on restart
        let mut last_signature: Option<Signature> = None;

        loop {
            ticker.tick().await;
            if let Err(e) = self.poll_once(&discriminators, &mut last_signature).await {
                tracing::error!("Indexer poll error: {}", e);
            }
        }
    }

    async fn poll_once(&self, discriminators: &[[u8; 8]], last_sig: &mut Option<Signature>) -> Result<()> {
        let rpc = solana_rpc_client::nonblocking::rpc_client::RpcClient::new(self.rpc_url.clone());
        let program_pk = Pubkey::from_str(&self.program_id)?;

        let mut signatures = rpc.get_signatures_for_address(&program_pk).await?;
        
        // Process new signatures (reverse to go oldest -> newest)
        signatures.reverse();
        
        // Simplistic "since last seen" logic
        // In production, use `until` parameter of `get_signatures_for_address` correctly.
        // For now, just process what we find that is newer than last_sig (if present).
        
        if let Some(_last) = last_sig {
             // If we have a last sig, we ask for signatures UNTIL that one (which returns newer ones).
             // get_signatures_for_address_with_config( ..., until: Some(last) )
             // TODO: implement robust pagination
        }

        for sig_info in signatures {
            let signature = Signature::from_str(&sig_info.signature)?;
            
            // Skip failed txs
            if sig_info.err.is_some() {
                *last_sig = Some(signature);
                continue;
            }

            // If we've seen this signature before (simple check if we had state), skip
            // Real implementation needs better state tracking.
            if let Some(last) = last_sig {
                if *last == signature { continue; } 
            }

            // Fetch tx
            let config = solana_rpc_client_api::config::RpcTransactionConfig {
                 encoding: Some(solana_transaction_status::UiTransactionEncoding::Json),
                 commitment: Some(CommitmentConfig::confirmed()),
                 max_supported_transaction_version: Some(0),
            };
            
            match rpc.get_transaction_with_config(&signature, config).await {
                Ok(tx_data) => {
                     if let Some(meta) = tx_data.transaction.meta {
                         if let solana_transaction_status::option_serializer::OptionSerializer::Some(log_messages) = meta.log_messages {
                             if contains_relevant_event(&log_messages, discriminators) {
                                 tracing::info!("Found relevant event in tx: {}", signature);
                                 let valid_wagers = extract_wager_pubkeys(&log_messages, discriminators);
                                 for wager_pk in valid_wagers {
                                     self.update_wager_state(&rpc, wager_pk).await?;
                                 }
                             }
                         }
                     }
                },
                Err(e) => tracing::warn!("Failed to fetch tx {}: {}", signature, e),
            }
            
            *last_sig = Some(signature);
        }
        Ok(())
    }

    async fn update_wager_state(&self, rpc: &solana_rpc_client::nonblocking::rpc_client::RpcClient, wager_pk: Pubkey) -> Result<()> {
        tracing::info!("Updating wager state for: {}", wager_pk);
        let account = rpc.get_account(&wager_pk).await?;
        if account.data.len() < 8 { return Ok(()); }
        
        // Skip 8-byte discriminator
        let mut data_slice = &account.data[8..];
        let state: WagerAccount = BorshDeserialize::deserialize(&mut data_slice)?;

        let on_chain_address = wager_pk.to_string();
        let new_status = format!("{:?}", state.status).to_lowercase();
        let winner_wallet = state.winner.map(|k| k.to_string());

        // Check existing DB record to avoid double-counting wins/losses
        let existing = self.db.get_wager_by_address(&on_chain_address).await?;
        let was_already_resolved = existing.as_ref().map_or(false, |w| w.status == "resolved");

        let record = crate::models::WagerRecord {
            id: uuid::Uuid::new_v4(),
            on_chain_address: on_chain_address.clone(),
            wager_id: state.wager_id as i64,
            initiator: state.initiator.to_string(),
            challenger: state.challenger.map(|k| k.to_string()),
            stake_lamports: state.stake_lamports as i64,
            description: state.description.clone(),
            status: new_status.clone(),
            resolution_source: format!("{:?}", state.resolution_source).to_lowercase(),
            resolver: state.resolver.to_string(),
            expiry_ts: state.expiry_ts,
            created_at: chrono::DateTime::from_timestamp(state.created_at, 0).unwrap_or_default().into(),
            resolved_at: if state.resolved_at > 0 { Some(chrono::DateTime::from_timestamp(state.resolved_at, 0).unwrap_or_default().into()) } else { None },
            winner: winner_wallet.clone(),
            protocol_fee_bps: state.protocol_fee_bps as i16,
            oracle_feed: state.oracle_feed.map(|k| k.to_string()),
            oracle_target: Some(state.oracle_target),
            dispute_opened_at: if state.dispute_opened_at > 0 { Some(chrono::DateTime::from_timestamp(state.dispute_opened_at, 0).unwrap_or_default().into()) } else { None },
            dispute_opener: state.dispute_opener.map(|k| k.to_string()),
            initiator_option: None, // Not stored on-chain, only in backend DB
            creator_declared_winner: None,
            challenger_declared_winner: None,
        };
        
        self.db.upsert_wager(&record).await?;
        tracing::info!("Upserted wager #{} ({:?})", state.wager_id, state.status);

        // If the wager just became resolved (wasn't already), update user win/loss stats
        if new_status == "resolved" && !was_already_resolved {
            if let Some(ref winner) = winner_wallet {
                let initiator = state.initiator.to_string();
                let challenger = state.challenger.map(|k| k.to_string());
                let loser = if *winner == initiator {
                    challenger.as_deref()
                } else {
                    Some(initiator.as_str())
                };
                self.db.record_wager_result(winner, loser).await?;
            }
        }

        Ok(())
    }
}

// ─── Mirror On-Chain Types (Borsh) ──────────────────────────────────────────

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct WagerAccount {
    pub bump: u8,
    pub wager_id: u64,
    pub initiator: Pubkey,
    pub challenger: Option<Pubkey>,
    pub stake_lamports: u64,
    pub description: String,
    pub status: WagerStatus,
    pub resolution_source: ResolutionSource,
    pub resolver: Pubkey,
    pub expiry_ts: i64,
    pub created_at: i64,
    pub resolved_at: i64,
    pub winner: Option<Pubkey>,
    pub protocol_fee_bps: u16,
    pub initiator_consent: bool,
    pub challenger_consent: bool,
    pub dispute_opened_at: i64,
    pub dispute_opener: Option<Pubkey>,
    pub oracle_feed: Option<Pubkey>,
    pub oracle_target: i64,
    pub oracle_initiator_wins_above: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum WagerStatus {
    Pending,
    Active,
    Resolved,
    Cancelled,
    Disputed,
    Expired,
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum ResolutionSource {
    Arbitrator,
    OracleFeed,
    MutualConsent,
}

// ─── Helper functions ────────────────────────────────────────────────────────

fn event_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(format!("event:{}", name).as_bytes());
    let result = hasher.finalize();
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&result[..8]);
    disc
}

fn contains_relevant_event(logs: &[String], targets: &[[u8; 8]]) -> bool {
    logs.iter().any(|log| {
        if log.starts_with("Program data: ") {
             if let Ok(data) = B64.decode(&log["Program data: ".len()..]) {
                 if data.len() >= 8 {
                     targets.contains(&data[0..8].try_into().unwrap())
                 } else { false }
             } else { false }
        } else { false }
    })
}

fn extract_wager_pubkeys(logs: &[String], targets: &[[u8; 8]]) -> Vec<Pubkey> {
    let mut keys = Vec::new();
    for log in logs {
         if log.starts_with("Program data: ") {
             if let Ok(data) = B64.decode(&log["Program data: ".len()..]) {
                 if data.len() >= 40 { // 8 discriminator + 32 Pubkey
                     let disc: [u8; 8] = data[0..8].try_into().unwrap();
                     if targets.contains(&disc) {
                         let pk_bytes: [u8; 32] = data[8..40].try_into().unwrap();
                         keys.push(Pubkey::new_from_array(pk_bytes));
                     }
                 }
             }
        }
    }
    keys
}