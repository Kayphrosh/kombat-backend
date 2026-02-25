// app/src/services/solana.rs
//! Builds and serializes unsigned Solana transactions for the client to sign.
//! The API never holds private keys — all transactions are returned as
//! base64-encoded `VersionedTransaction` messages for wallet signing.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    transaction::Transaction,
};
use std::str::FromStr;

/// The deployed Wager program ID
pub const WAGER_PROGRAM_ID: &str = "Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK";

pub struct SolanaService {
    pub rpc: RpcClient,
    pub program_id: Pubkey,
}

impl SolanaService {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc: RpcClient::new(rpc_url.to_string()),
            program_id: Pubkey::from_str(WAGER_PROGRAM_ID).unwrap(),
        }
    }

    // ── PDA Derivation ────────────────────────────────────────────────────────

    pub fn registry_pda(&self, authority: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"registry", authority.as_ref()],
            &self.program_id,
        )
    }

    pub fn wager_pda(&self, initiator: &Pubkey, wager_id: u64) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"wager", initiator.as_ref(), &wager_id.to_le_bytes()],
            &self.program_id,
        )
    }

    pub fn escrow_pda(&self, wager: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"escrow", wager.as_ref()],
            &self.program_id,
        )
    }

    pub fn config_pda(&self) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"config"], &self.program_id)
    }

    // ── Transaction Serialization ─────────────────────────────────────────────

    /// Serialize a list of instructions into an unsigned base64 transaction.
    pub async fn build_transaction(
        &self,
        instructions: Vec<Instruction>,
        payer: &Pubkey,
    ) -> Result<String> {
        let result: Result<solana_sdk::hash::Hash, _> = self.rpc.get_latest_blockhash().await;
        let recent_blockhash = result.map_err(|e| anyhow::anyhow!("Failed to fetch latest blockhash: {}", e))?;

        let message = Message::new_with_blockhash(
            &instructions,
            Some(payer),
            &recent_blockhash,
        );

        let tx = Transaction::new_unsigned(message);
        let serialized = bincode::serialize(&tx)
            .context("Failed to serialize transaction")?;

        Ok(B64.encode(serialized))
    }

    // ── Instruction Builders ──────────────────────────────────────────────────

    /// Build the `initialize_registry` instruction for a new user.
    pub fn ix_initialize_registry(&self, authority: &Pubkey) -> Instruction {
        let (registry, _) = self.registry_pda(authority);

        // Anchor discriminator: sha256("global:initialize_registry")[..8]
        let data = anchor_discriminator("initialize_registry");
        // No additional args

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // Must match: InitializeRegistry { registry, authority, system_program }
                AccountMeta::new(registry, false),
                AccountMeta::new(*authority, true),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data,
        }
    }

    /// Build the `create_wager` instruction.
    pub fn ix_create_wager(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        args_data: Vec<u8>,  // borsh-serialized CreateWagerArgs
    ) -> Instruction {
        let (config, _)   = self.config_pda();
        let (registry, _) = self.registry_pda(initiator);
        let (wager, _)    = self.wager_pda(initiator, wager_id);
        let (escrow, _)   = self.escrow_pda(&wager);

        let mut data = anchor_discriminator("create_wager");
        data.extend_from_slice(&args_data);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config
                AccountMeta::new_readonly(config, false),
                // 2. registry
                AccountMeta::new(registry, false),
                // 3. wager
                AccountMeta::new(wager, false),
                // 4. escrow
                AccountMeta::new(escrow, false),
                // 5. initiator
                AccountMeta::new(*initiator, true),
                // 6. system_program
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data,
        }
    }

    /// Build the `accept_wager` instruction.
    pub fn ix_accept_wager(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        challenger: &Pubkey,
    ) -> Instruction {
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        let (escrow, _) = self.escrow_pda(&wager);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(wager, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(*challenger, true),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data: anchor_discriminator("accept_wager"),
        }
    }

    /// Build the `resolve_by_arbitrator` instruction.
    pub fn ix_resolve_by_arbitrator(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        winner: &Pubkey,
        resolver: &Pubkey,
        treasury: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        let (escrow, _) = self.escrow_pda(&wager);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(wager, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(*winner, false),
                AccountMeta::new(*treasury, false),
                AccountMeta::new_readonly(*resolver, true),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data: anchor_discriminator("resolve_by_arbitrator"),
        }
    }

    /// Build the `open_dispute` instruction.
    pub fn ix_open_dispute(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        participant: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(wager, false),
                AccountMeta::new_readonly(*participant, true),
            ],
            data: anchor_discriminator("open_dispute"),
        }
    }

    /// Build the `cancel_wager` instruction.
    pub fn ix_cancel_wager(&self, initiator: &Pubkey, wager_id: u64) -> Instruction {
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        let (escrow, _) = self.escrow_pda(&wager);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(wager, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(*initiator, true),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data: anchor_discriminator("cancel_wager"),
        }
    }

    /// Build the `consent_resolve` instruction (mutual consent winner declaration).
    pub fn ix_consent_resolve(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        participant: &Pubkey,
        declared_winner: &Pubkey,
        treasury: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        let (escrow, _) = self.escrow_pda(&wager);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // Must match: ConsentResolve { config, wager, escrow, participant, winner, treasury, system_program }
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(wager, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new_readonly(*participant, true),
                AccountMeta::new(*declared_winner, false),
                AccountMeta::new(*treasury, false),
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            ],
            data: anchor_discriminator("consent_resolve"),
        }
    }

    /// Read the treasury pubkey from the on-chain ProtocolConfig PDA.
    /// Layout: 8 (discriminator) + 1 (bump) + 32 (admin) + 32 (treasury)
    pub async fn get_treasury(&self) -> Result<Pubkey> {
        let (config_pda, _) = self.config_pda();
        let account = self.rpc.get_account(&config_pda).await
            .context("Failed to read ProtocolConfig account")?;
        let data = &account.data;
        // 8 (disc) + 1 (bump) + 32 (admin) = offset 41, then 32 bytes of treasury
        if data.len() < 73 {
            anyhow::bail!("ProtocolConfig account data too short");
        }
        let treasury_bytes: [u8; 32] = data[41..73].try_into()
            .map_err(|_| anyhow::anyhow!("Failed to parse treasury pubkey"))?;
        Ok(Pubkey::new_from_array(treasury_bytes))
    }
}

// ─── Anchor discriminator helper ─────────────────────────────────────────────────
/// Produces the 8-byte Anchor instruction discriminator: sha256("global:<name>")[..8]
fn anchor_discriminator(name: &str) -> Vec<u8> {
    use sha2::{Sha256, Digest};

    let preimage = format!("global:{}", name);
    let hash = Sha256::digest(preimage.as_bytes());
    hash[..8].to_vec()
}