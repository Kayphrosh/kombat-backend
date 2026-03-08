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

/// SPL Token program ID
pub const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// SPL Associated Token Account program ID
pub const SPL_ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

pub struct SolanaService {
    pub rpc: RpcClient,
    pub program_id: Pubkey,
    pub token_program_id: Pubkey,
    pub associated_token_program_id: Pubkey,
}

impl SolanaService {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc: RpcClient::new(rpc_url.to_string()),
            program_id: Pubkey::from_str(WAGER_PROGRAM_ID).unwrap(),
            token_program_id: Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).unwrap(),
            associated_token_program_id: Pubkey::from_str(SPL_ASSOCIATED_TOKEN_PROGRAM_ID).unwrap(),
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

    /// Derive the Associated Token Account address for a given wallet and mint.
    pub fn get_associated_token_address(&self, wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
        let seeds = &[
            wallet.as_ref(),
            self.token_program_id.as_ref(),
            mint.as_ref(),
        ];
        let (ata, _) = Pubkey::find_program_address(seeds, &self.associated_token_program_id);
        ata
    }

    /// Read the USDC mint pubkey from the on-chain ProtocolConfig PDA.
    /// Layout: 8 (disc) + 1 (bump) + 32 (admin) + 32 (treasury) + 2 (fee_bps) + 8 (dispute_window) + 1 (paused) + 32 (usdc_mint)
    pub async fn get_usdc_mint(&self) -> Result<Pubkey> {
        let (config_pda, _) = self.config_pda();
        let account = self.rpc.get_account(&config_pda).await
            .context("Failed to read ProtocolConfig account")?;
        let data = &account.data;
        // usdc_mint offset: 8 + 1 + 32 + 32 + 2 + 8 + 1 = 84
        if data.len() < 116 {
            anyhow::bail!("ProtocolConfig account data too short (expected at least 116 bytes, got {})", data.len());
        }
        let usdc_mint_bytes: [u8; 32] = data[84..116].try_into()
            .map_err(|_| anyhow::anyhow!("Failed to parse usdc_mint pubkey"))?;
        Ok(Pubkey::new_from_array(usdc_mint_bytes))
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
    /// 
    /// # Arguments
    /// * `initiator` - The initiator's wallet pubkey
    /// * `wager_id` - The wager ID from the registry
    /// * `usdc_mint` - The USDC mint address from on-chain config
    /// * `initiator_token_account` - The initiator's USDC ATA
    /// * `args_data` - Borsh-serialized CreateWagerArgs
    pub fn ix_create_wager(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        usdc_mint: &Pubkey,
        initiator_token_account: &Pubkey,
        args_data: Vec<u8>,
    ) -> Instruction {
        let (config, _)   = self.config_pda();
        let (registry, _) = self.registry_pda(initiator);
        let (wager, _)    = self.wager_pda(initiator, wager_id);
        
        // Escrow token account is the ATA owned by the wager PDA
        let escrow_token_account = self.get_associated_token_address(&wager, usdc_mint);

        let mut data = anchor_discriminator("create_wager");
        data.extend_from_slice(&args_data);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config (readonly)
                AccountMeta::new_readonly(config, false),
                // 2. registry (mutable)
                AccountMeta::new(registry, false),
                // 3. wager (init, mutable)
                AccountMeta::new(wager, false),
                // 4. usdc_mint (readonly)
                AccountMeta::new_readonly(*usdc_mint, false),
                // 5. escrow_token_account (init, mutable)
                AccountMeta::new(escrow_token_account, false),
                // 6. initiator_token_account (mutable)
                AccountMeta::new(*initiator_token_account, false),
                // 7. initiator (signer, mutable)
                AccountMeta::new(*initiator, true),
                // 8. system_program
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
                // 9. token_program
                AccountMeta::new_readonly(self.token_program_id, false),
                // 10. associated_token_program
                AccountMeta::new_readonly(self.associated_token_program_id, false),
            ],
            data,
        }
    }

    /// Build the `accept_wager` instruction.
    /// 
    /// # Arguments
    /// * `initiator` - The original initiator's wallet pubkey (for PDA derivation)
    /// * `wager_id` - The wager ID
    /// * `challenger` - The challenger's wallet pubkey
    /// * `usdc_mint` - The USDC mint address from on-chain config
    /// * `challenger_token_account` - The challenger's USDC ATA
    pub fn ix_accept_wager(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        challenger: &Pubkey,
        usdc_mint: &Pubkey,
        challenger_token_account: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        
        // Escrow token account is the ATA owned by the wager PDA
        let escrow_token_account = self.get_associated_token_address(&wager, usdc_mint);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config (readonly)
                AccountMeta::new_readonly(config, false),
                // 2. wager (mutable)
                AccountMeta::new(wager, false),
                // 3. usdc_mint (readonly)
                AccountMeta::new_readonly(*usdc_mint, false),
                // 4. escrow_token_account (mutable)
                AccountMeta::new(escrow_token_account, false),
                // 5. challenger_token_account (mutable)
                AccountMeta::new(*challenger_token_account, false),
                // 6. challenger (signer, mutable)
                AccountMeta::new(*challenger, true),
                // 7. token_program
                AccountMeta::new_readonly(self.token_program_id, false),
            ],
            data: anchor_discriminator("accept_wager"),
        }
    }

    /// Build the `resolve_by_arbitrator` instruction.
    /// 
    /// # Arguments
    /// * `initiator` - The original initiator's wallet pubkey (for PDA derivation)
    /// * `wager_id` - The wager ID
    /// * `usdc_mint` - The USDC mint address from on-chain config
    /// * `winner_token_account` - The winner's USDC ATA
    /// * `treasury_token_account` - The treasury's USDC ATA for protocol fees
    /// * `resolver` - The arbitrator who is resolving the wager
    pub fn ix_resolve_by_arbitrator(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        usdc_mint: &Pubkey,
        winner_token_account: &Pubkey,
        treasury_token_account: &Pubkey,
        resolver: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        
        // Escrow token account is the ATA owned by the wager PDA
        let escrow_token_account = self.get_associated_token_address(&wager, usdc_mint);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config (readonly)
                AccountMeta::new_readonly(config, false),
                // 2. wager (mutable)
                AccountMeta::new(wager, false),
                // 3. usdc_mint (readonly)
                AccountMeta::new_readonly(*usdc_mint, false),
                // 4. escrow_token_account (mutable)
                AccountMeta::new(escrow_token_account, false),
                // 5. winner_token_account (mutable)
                AccountMeta::new(*winner_token_account, false),
                // 6. treasury_token_account (mutable)
                AccountMeta::new(*treasury_token_account, false),
                // 7. resolver (signer)
                AccountMeta::new_readonly(*resolver, true),
                // 8. token_program
                AccountMeta::new_readonly(self.token_program_id, false),
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
    /// 
    /// # Arguments
    /// * `initiator` - The initiator's wallet pubkey
    /// * `wager_id` - The wager ID
    /// * `usdc_mint` - The USDC mint address from on-chain config
    /// * `initiator_token_account` - The initiator's USDC ATA to receive refund
    pub fn ix_cancel_wager(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        usdc_mint: &Pubkey,
        initiator_token_account: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        
        // Escrow token account is the ATA owned by the wager PDA
        let escrow_token_account = self.get_associated_token_address(&wager, usdc_mint);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config (readonly)
                AccountMeta::new_readonly(config, false),
                // 2. wager (mutable)
                AccountMeta::new(wager, false),
                // 3. usdc_mint (readonly)
                AccountMeta::new_readonly(*usdc_mint, false),
                // 4. escrow_token_account (mutable)
                AccountMeta::new(escrow_token_account, false),
                // 5. initiator_token_account (mutable) - receives refund
                AccountMeta::new(*initiator_token_account, false),
                // 6. initiator (signer, mutable)
                AccountMeta::new(*initiator, true),
                // 7. token_program
                AccountMeta::new_readonly(self.token_program_id, false),
            ],
            data: anchor_discriminator("cancel_wager"),
        }
    }

    /// Build the `consent_resolve` instruction (mutual consent winner declaration).
    /// 
    /// # Arguments
    /// * `initiator` - The original initiator's wallet pubkey (for PDA derivation)
    /// * `wager_id` - The wager ID
    /// * `usdc_mint` - The USDC mint address from on-chain config
    /// * `participant` - The participant giving consent
    /// * `winner_token_account` - The declared winner's USDC ATA
    /// * `treasury_token_account` - The treasury's USDC ATA for protocol fees
    pub fn ix_consent_resolve(
        &self,
        initiator: &Pubkey,
        wager_id: u64,
        usdc_mint: &Pubkey,
        participant: &Pubkey,
        winner_token_account: &Pubkey,
        treasury_token_account: &Pubkey,
    ) -> Instruction {
        let (config, _) = self.config_pda();
        let (wager, _)  = self.wager_pda(initiator, wager_id);
        
        // Escrow token account is the ATA owned by the wager PDA
        let escrow_token_account = self.get_associated_token_address(&wager, usdc_mint);

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                // 1. config (readonly)
                AccountMeta::new_readonly(config, false),
                // 2. wager (mutable)
                AccountMeta::new(wager, false),
                // 3. usdc_mint (readonly)
                AccountMeta::new_readonly(*usdc_mint, false),
                // 4. escrow_token_account (mutable)
                AccountMeta::new(escrow_token_account, false),
                // 5. participant (signer)
                AccountMeta::new_readonly(*participant, true),
                // 6. winner_token_account (mutable)
                AccountMeta::new(*winner_token_account, false),
                // 7. treasury_token_account (mutable)
                AccountMeta::new(*treasury_token_account, false),
                // 8. token_program
                AccountMeta::new_readonly(self.token_program_id, false),
            ],
            data: anchor_discriminator("consent_resolve"),
        }
    }

    /// Check whether an account exists on-chain (i.e. has been initialized).
    pub async fn account_exists(&self, address: &Pubkey) -> Result<bool> {
        match self.rpc.get_account(address).await {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("AccountNotFound") || msg.contains("could not find account") {
                    Ok(false)
                } else {
                    Err(anyhow::anyhow!("RPC error checking account {}: {}", address, msg))
                }
            }
        }
    }

    /// Build a `create_associated_token_account_idempotent` instruction.
    /// This is a no-op if the ATA already exists, so it's safe to always include.
    pub fn ix_create_ata_idempotent(
        &self,
        payer: &Pubkey,
        wallet: &Pubkey,
        mint: &Pubkey,
    ) -> Instruction {
        let ata = self.get_associated_token_address(wallet, mint);
        // CreateIdempotent instruction tag is index 1 in the ATA program
        Instruction {
            program_id: self.associated_token_program_id,
            accounts: vec![
                AccountMeta::new(*payer, true),                             // funding account
                AccountMeta::new(ata, false),                               // associated token account
                AccountMeta::new_readonly(*wallet, false),                  // wallet owner
                AccountMeta::new_readonly(*mint, false),                    // token mint
                AccountMeta::new_readonly(solana_sdk::system_program::id(), false), // system program
                AccountMeta::new_readonly(self.token_program_id, false),    // token program
            ],
            data: vec![1], // 1 = CreateIdempotent
        }
    }

    /// Build `create_associated_token_account_idempotent` instructions for any
    /// ATAs in the provided list that don't yet exist on-chain.
    /// Returns the instructions to prepend to the transaction.
    pub async fn ensure_atas_exist(
        &self,
        payer: &Pubkey,
        mint: &Pubkey,
        owners: &[&Pubkey],
    ) -> Result<Vec<Instruction>> {
        let mut ixs = Vec::new();
        for owner in owners {
            let ata = self.get_associated_token_address(owner, mint);
            if !self.account_exists(&ata).await? {
                tracing::info!("ATA {} for owner {} does not exist — will create", ata, owner);
                ixs.push(self.ix_create_ata_idempotent(payer, owner, mint));
            }
        }
        Ok(ixs)
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