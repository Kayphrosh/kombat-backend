// programs/wager/src/state.rs
use anchor_lang::prelude::*;

/// ─── Wager Status ────────────────────────────────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum WagerStatus {
    /// Created by initiator, awaiting challenger
    Pending,
    /// Challenger has deposited — both sides locked in
    Active,
    /// Oracle/resolver has declared a winner
    Resolved,
    /// Cancelled before challenger joined
    Cancelled,
    /// Under dispute review
    Disputed,
    /// Expired (deadline passed with no acceptance)
    Expired,
}

impl Default for WagerStatus {
    fn default() -> Self {
        WagerStatus::Pending
    }
}

/// ─── Resolution Source ───────────────────────────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum ResolutionSource {
    /// A mutually agreed trusted third-party arbitrator
    Arbitrator,
    /// Switchboard / Pyth on-chain price feed
    OracleFeed,
    /// Mutual agreement between both parties
    MutualConsent,
}

impl Default for ResolutionSource {
    fn default() -> Self {
        ResolutionSource::Arbitrator
    }
}

/// ─── Wager Account (PDA) ─────────────────────────────────────────────────────
///
/// Seeds: [b"wager", initiator.key().as_ref(), wager_id.to_le_bytes().as_ref()]
///
#[account]
#[derive(Default)]
pub struct Wager {
    /// Bump seed for this PDA
    pub bump: u8,

    /// Unique numeric ID (incrementing per initiator)
    pub wager_id: u64,

    /// The party who opened the wager
    pub initiator: Pubkey,

    /// The party who accepted the wager (None until accepted)
    pub challenger: Option<Pubkey>,

    /// SOL stake (in lamports) each side must put in
    pub stake_lamports: u64,

    /// Human-readable description (max 256 bytes)
    pub description: String,

    /// Current lifecycle status
    pub status: WagerStatus,

    /// How this wager will be resolved
    pub resolution_source: ResolutionSource,

    /// Arbitrator / oracle pubkey (depends on resolution_source)
    pub resolver: Pubkey,

    /// Unix timestamp — wager expires if not accepted before this
    pub expiry_ts: i64,

    /// Unix timestamp when the wager was created
    pub created_at: i64,

    /// Unix timestamp when it was resolved (0 if not yet)
    pub resolved_at: i64,

    /// The winner — set on resolution
    pub winner: Option<Pubkey>,

    /// Fee taken by protocol (basis points, e.g. 100 = 1%)
    pub protocol_fee_bps: u16,

    /// Whether initiator has consented to mutual resolve
    pub initiator_consent: bool,

    /// Whether challenger has consented to mutual resolve
    pub challenger_consent: bool,

    /// Dispute timestamp (0 if no active dispute)
    pub dispute_opened_at: i64,

    /// Party that opened the dispute
    pub dispute_opener: Option<Pubkey>,

    /// Oracle feed pubkey (used only when resolution_source = OracleFeed)
    pub oracle_feed: Option<Pubkey>,

    /// The target oracle value to determine the winner
    pub oracle_target: i64,

    /// Whether oracle target means "above" (true) or "below" (false) wins for initiator
    pub oracle_initiator_wins_above: bool,
}

impl Wager {
    /// Account size — must be large enough for all fields
    pub const LEN: usize = 8  // discriminator
        + 1   // bump
        + 8   // wager_id
        + 32  // initiator
        + 1 + 32  // challenger (Option<Pubkey>)
        + 8   // stake_lamports
        + 4 + 256 // description (String: 4-byte length prefix + max 256 bytes)
        + 2   // status (enum)
        + 2   // resolution_source (enum)
        + 32  // resolver
        + 8   // expiry_ts
        + 8   // created_at
        + 8   // resolved_at
        + 1 + 32  // winner (Option<Pubkey>)
        + 2   // protocol_fee_bps
        + 1   // initiator_consent
        + 1   // challenger_consent
        + 8   // dispute_opened_at
        + 1 + 32  // dispute_opener (Option<Pubkey>)
        + 1 + 32  // oracle_feed (Option<Pubkey>)
        + 8   // oracle_target
        + 1   // oracle_initiator_wins_above
        + 64; // padding


}

/// ─── Initiator Registry (PDA) ────────────────────────────────────────────────
///
/// Seeds: [b"registry", authority.key().as_ref()]
/// Tracks how many wagers a user has created for sequential IDs.
///
#[account]
pub struct WagerRegistry {
    pub bump: u8,
    pub authority: Pubkey,
    pub wager_count: u64,
}

impl WagerRegistry {
    pub const LEN: usize = 8 + 1 + 32 + 8;
}

/// ─── Protocol Config (PDA) ───────────────────────────────────────────────────
///
/// Seeds: [b"config"]
/// Global protocol settings controlled by the admin.
///
#[account]
pub struct ProtocolConfig {
    pub bump: u8,
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub default_fee_bps: u16,
    pub dispute_window_seconds: i64,
    pub paused: bool,
}

impl ProtocolConfig {
    pub const LEN: usize = 8 + 1 + 32 + 32 + 2 + 8 + 1;
}
