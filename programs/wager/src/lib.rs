// programs/wager/src/lib.rs
use anchor_lang::prelude::*;

pub mod errors;
pub mod state;

// Instruction modules — declared directly at crate root so the #[program]
// macro can resolve every Context<T> type without module-qualified paths.
pub mod initialize;
pub mod create_wager;
pub mod resolve_wager;
pub mod dispute;

// Re-export everything so `use crate::*` inside #[program] picks it all up.
use initialize::*;
use create_wager::*;
use resolve_wager::*;
use dispute::*;

// Replace with your deployed program ID after `anchor deploy`
declare_id!("Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK");

/// ─────────────────────────────────────────────────────────────────────────────
///  W A G E R   P R O G R A M
/// ─────────────────────────────────────────────────────────────────────────────
///
///  A head-to-head P2P wagering program on Solana.
///
///  Account hierarchy
///  ─────────────────
///  ProtocolConfig  [PDA: "config"]
///    └─ WagerRegistry  [PDA: "registry", authority]
///         └─ Wager         [PDA: "wager", initiator, wager_id]
///              └─ Escrow   [PDA: "escrow", wager]
///
///  Lifecycle
///  ─────────
///  Pending ──accept──► Active ──resolve──► Resolved
///     │                  │
///     │                  └──dispute──► Disputed ──settle──► Resolved
///     │                                                └──expire──► Expired
///     └──cancel──► Cancelled
///     └──expire──► Expired
///
#[program]
pub mod wager {
    use super::*;

    // ── Admin / Protocol ──────────────────────────────────────────────────────

    /// One-time protocol initialisation. Sets fee BPS and dispute window.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        default_fee_bps: u16,
        dispute_window_seconds: i64,
    ) -> Result<()> {
        handle_initialize_protocol(ctx, default_fee_bps, dispute_window_seconds)
    }

    /// Create a per-user registry to track sequential wager IDs.
    pub fn initialize_registry(ctx: Context<InitializeRegistry>) -> Result<()> {
        handle_initialize_registry(ctx)
    }

    /// Admin: pause or unpause the protocol.
    pub fn set_protocol_pause(ctx: Context<SetProtocolPause>, paused: bool) -> Result<()> {
        handle_set_pause(ctx, paused)
    }

    /// Admin: update the protocol fee.
    pub fn update_fee(ctx: Context<UpdateFee>, new_fee_bps: u16) -> Result<()> {
        handle_update_fee(ctx, new_fee_bps)
    }

    /// Admin: migrate config to add USDC mint (one-time migration)
    pub fn migrate_config(ctx: Context<MigrateConfig>) -> Result<()> {
        handle_migrate_config(ctx)
    }

    // ── Wager Lifecycle ───────────────────────────────────────────────────────

    /// Create a new P2P wager and escrow the initiator's stake.
    pub fn create_wager(ctx: Context<CreateWager>, args: CreateWagerArgs) -> Result<()> {
        handle_create_wager(ctx, args)
    }

    /// Accept an open wager as the challenger and escrow the matching stake.
    pub fn accept_wager(ctx: Context<AcceptWager>) -> Result<()> {
        handle_accept_wager(ctx)
    }

    /// Cancel a pending wager (initiator only) and refund the stake.
    pub fn cancel_wager(ctx: Context<CancelWager>) -> Result<()> {
        handle_cancel_wager(ctx)
    }

    /// Expire a wager that was never accepted and refund the initiator.
    pub fn expire_wager(ctx: Context<ExpireWager>) -> Result<()> {
        handle_expire_wager(ctx)
    }

    // ── Resolution ────────────────────────────────────────────────────────────

    /// Resolve a wager using the designated arbitrator.
    pub fn resolve_by_arbitrator(ctx: Context<ResolveByArbitrator>) -> Result<()> {
        handle_resolve_by_arbitrator(ctx)
    }

    /// Register participant consent for mutual-consent resolution.
    /// Auto-pays when both parties have consented.
    pub fn consent_resolve(ctx: Context<ConsentResolve>) -> Result<()> {
        handle_consent_resolve(ctx)
    }

    /// Resolve a wager using a Switchboard/Pyth on-chain oracle feed.
    pub fn resolve_by_oracle(ctx: Context<ResolveByOracle>) -> Result<()> {
        handle_resolve_by_oracle(ctx)
    }

    // ── Disputes ──────────────────────────────────────────────────────────────

    /// Open a dispute on an active wager.
    pub fn open_dispute(ctx: Context<OpenDispute>) -> Result<()> {
        handle_open_dispute(ctx)
    }

    /// Admin: settle a disputed wager and declare a winner.
    pub fn settle_dispute(ctx: Context<SettleDispute>) -> Result<()> {
        handle_settle_dispute(ctx)
    }

    /// Close a stale dispute that the arbitrator failed to resolve within the grace period.
    pub fn close_expired_dispute(ctx: Context<CloseExpiredDispute>) -> Result<()> {
        handle_close_expired_dispute(ctx)
    }
}
