// app/src/services/delegation.rs
//! Delegated SPL Token authority service.
//!
//! Users sign a one-time `approve` transaction granting the platform signer
//! permission to transfer USDC on their behalf. After that, the backend signs
//! and pays fees for all subsequent transfers — users only need their PIN.
//!
//! The platform signer also acts as fee payer so users never need SOL.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};

/// Maximum delegation amount (500 USDC in micro-USDC).
/// Users approve up to this amount; they can always revoke.
pub const MAX_DELEGATION_USDC: u64 = 500_000_000;

/// SPL Token program ID.
const SPL_TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

pub struct DelegationService {
    /// Platform keypair – acts as delegate and fee payer.
    pub signer: Keypair,
    /// Cached SPL Token program pubkey.
    token_program: Pubkey,
}

impl DelegationService {
    /// Create from a JSON byte-array keypair string (same format as Solana CLI
    /// `~/.config/solana/id.json`, e.g. `[12,34,56,…]`).
    pub fn from_json_keypair(json: &str) -> Result<Self> {
        let bytes: Vec<u8> = serde_json::from_str(json)
            .context("PLATFORM_SIGNER_KEYPAIR must be a JSON byte array")?;
        let signer = Keypair::try_from(bytes.as_slice())
            .map_err(|e| anyhow::anyhow!("Invalid keypair bytes: {e}"))?;

        tracing::info!(
            "Delegation service initialized — platform signer: {}",
            signer.pubkey()
        );

        Ok(Self {
            signer,
            token_program: SPL_TOKEN_PROGRAM.parse().unwrap(),
        })
    }

    /// Public key of the platform signer (the delegate).
    pub fn pubkey(&self) -> Pubkey {
        self.signer.pubkey()
    }

    // ── Build unsigned approve tx (for user to sign once) ──────────────────

    /// Build an unsigned `spl_token::approve` transaction.
    ///
    /// * `user_wallet` – the token account owner (user's wallet pubkey)
    /// * `user_token_account` – the user's USDC ATA
    /// * `amount` – micro-USDC allowance to approve
    /// * `recent_blockhash` – from RPC
    ///
    /// Returns base64-encoded unsigned transaction. The **user** is the fee
    /// payer on this one-time tx so they don't need to trust us with SOL yet.
    pub fn build_approve_tx(
        &self,
        user_wallet: &Pubkey,
        user_token_account: &Pubkey,
        amount: u64,
        recent_blockhash: solana_sdk::hash::Hash,
    ) -> Result<String> {
        let ix = self.approve_ix(user_token_account, user_wallet, amount);

        let message = Message::new_with_blockhash(
            &[ix],
            Some(user_wallet), // user pays fee for the one-time approve
            &recent_blockhash,
        );

        let tx = Transaction::new_unsigned(message);
        let serialized = bincode::serialize(&tx)
            .context("Failed to serialize approve transaction")?;
        Ok(B64.encode(serialized))
    }

    /// Build an unsigned `spl_token::revoke` transaction so the user can
    /// remove the delegation at any time.
    pub fn build_revoke_tx(
        &self,
        user_wallet: &Pubkey,
        user_token_account: &Pubkey,
        recent_blockhash: solana_sdk::hash::Hash,
    ) -> Result<String> {
        let ix = self.revoke_ix(user_token_account, user_wallet);

        let message = Message::new_with_blockhash(
            &[ix],
            Some(user_wallet),
            &recent_blockhash,
        );

        let tx = Transaction::new_unsigned(message);
        let serialized = bincode::serialize(&tx)
            .context("Failed to serialize revoke transaction")?;
        Ok(B64.encode(serialized))
    }

    // ── Execute delegated transfer (backend signs + pays fee) ──────────────

    /// Build a fully-signed `spl_token::transfer` using the platform delegate
    /// authority. The platform signer is both the delegate and fee payer,
    /// so the user doesn't need SOL or to sign anything.
    pub fn build_delegated_transfer(
        &self,
        user_token_account: &Pubkey,  // source (user's USDC ATA)
        dest_token_account: &Pubkey,  // destination (e.g. escrow vault)
        amount: u64,
        recent_blockhash: solana_sdk::hash::Hash,
    ) -> Result<Transaction> {
        let ix = self.transfer_ix(user_token_account, dest_token_account, amount);

        let message = Message::new_with_blockhash(
            &[ix],
            Some(&self.signer.pubkey()), // platform pays the fee
            &recent_blockhash,
        );

        let mut tx = Transaction::new_unsigned(message);
        tx.sign(&[&self.signer], recent_blockhash);

        Ok(tx)
    }

    // ── Raw SPL-Token instruction builders ─────────────────────────────────

    /// `spl_token::instruction::approve`
    /// Layout: tag(1) = 4, amount(8) LE
    fn approve_ix(&self, token_account: &Pubkey, owner: &Pubkey, amount: u64) -> Instruction {
        let mut data = vec![4u8]; // Approve instruction tag
        data.extend_from_slice(&amount.to_le_bytes());

        Instruction {
            program_id: self.token_program,
            accounts: vec![
                AccountMeta::new(*token_account, false),  // source account
                AccountMeta::new_readonly(self.signer.pubkey(), false), // delegate
                AccountMeta::new_readonly(*owner, true),   // owner (signer)
            ],
            data,
        }
    }

    /// `spl_token::instruction::revoke`
    /// Layout: tag(1) = 5
    fn revoke_ix(&self, token_account: &Pubkey, owner: &Pubkey) -> Instruction {
        Instruction {
            program_id: self.token_program,
            accounts: vec![
                AccountMeta::new(*token_account, false), // source account
                AccountMeta::new_readonly(*owner, true), // owner (signer)
            ],
            data: vec![5u8], // Revoke instruction tag
        }
    }

    /// `spl_token::instruction::transfer`
    /// Layout: tag(1) = 3, amount(8) LE
    /// Uses the platform signer as delegate authority.
    fn transfer_ix(
        &self,
        source: &Pubkey,
        destination: &Pubkey,
        amount: u64,
    ) -> Instruction {
        let mut data = vec![3u8]; // Transfer instruction tag
        data.extend_from_slice(&amount.to_le_bytes());

        Instruction {
            program_id: self.token_program,
            accounts: vec![
                AccountMeta::new(*source, false),                      // source
                AccountMeta::new(*destination, false),                 // destination
                AccountMeta::new_readonly(self.signer.pubkey(), true), // delegate (signer)
            ],
            data,
        }
    }

    // ── Payout / Refund (platform signer owns the vault) ───────────────────

    /// Build a fully-signed transfer from the platform signer's own token
    /// account (pool vault) to a user's ATA. Used for payouts and refunds.
    /// The platform signer is the **owner** of the source account.
    pub fn build_payout_transfer(
        &self,
        vault_token_account: &Pubkey,   // source (platform signer's ATA)
        user_token_account: &Pubkey,    // destination (user's ATA)
        amount: u64,
        recent_blockhash: solana_sdk::hash::Hash,
    ) -> Result<Transaction> {
        // Owner transfer: same SPL Transfer instruction but the signer is the owner
        let ix = self.transfer_ix(vault_token_account, user_token_account, amount);

        let message = Message::new_with_blockhash(
            &[ix],
            Some(&self.signer.pubkey()),
            &recent_blockhash,
        );

        let mut tx = Transaction::new_unsigned(message);
        tx.sign(&[&self.signer], recent_blockhash);

        Ok(tx)
    }
}
