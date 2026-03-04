// programs/wager/src/errors.rs
use anchor_lang::prelude::*;

#[error_code]
pub enum WagerError {
    // ── Lifecycle errors ──────────────────────────────────────────────────────
    #[msg("Wager is not in Pending status")]
    NotPending,

    #[msg("Wager is not in Active status")]
    NotActive,

    #[msg("Wager has already been resolved")]
    AlreadyResolved,

    #[msg("Wager has expired")]
    WagerExpired,

    #[msg("Wager has not yet expired")]
    WagerNotExpired,

    #[msg("Wager was cancelled")]
    WagerCancelled,

    // ── Authorization errors ──────────────────────────────────────────────────
    #[msg("Only the initiator can perform this action")]
    UnauthorizedInitiator,

    #[msg("Only the challenger can perform this action")]
    UnauthorizedChallenger,

    #[msg("Only the designated resolver may resolve this wager")]
    UnauthorizedResolver,

    #[msg("Only the protocol admin can call this instruction")]
    UnauthorizedAdmin,

    #[msg("Caller is not a participant in this wager")]
    NotAParticipant,

    #[msg("Initiator cannot challenge their own wager")]
    SelfChallenge,

    // ── Stake / fund errors ───────────────────────────────────────────────────
    #[msg("Stake amount must be greater than zero")]
    ZeroStake,

    #[msg("Incorrect stake amount provided")]
    IncorrectStake,

    #[msg("Insufficient lamports for stake + fees")]
    InsufficientFunds,

    #[msg("Invalid USDC mint address")]
    InvalidUsdcMint,

    // ── Resolution errors ─────────────────────────────────────────────────────
    #[msg("Winner must be the initiator or challenger")]
    InvalidWinner,

    #[msg("Both participants must consent before mutual resolution")]
    ConsentNotReached,

    #[msg("Oracle feed account does not match stored feed")]
    OracleFeedMismatch,

    #[msg("Oracle price data is stale")]
    StaleOraclePrice,

    #[msg("Oracle price value is out of expected range")]
    OraclePriceInvalid,

    // ── Dispute errors ────────────────────────────────────────────────────────
    #[msg("Wager is already under dispute")]
    AlreadyDisputed,

    #[msg("Dispute window has not opened yet; wager is still pending/active")]
    DisputeWindowNotOpen,

    #[msg("Dispute window has already closed")]
    DisputeWindowClosed,

    #[msg("Wager is not in a disputed state")]
    NotDisputed,

    // ── Input validation errors ───────────────────────────────────────────────
    #[msg("Description exceeds maximum length of 256 bytes")]
    DescriptionTooLong,

    #[msg("Expiry timestamp must be in the future")]
    ExpiryInPast,

    #[msg("Expiry timestamp is too far in the future (max 1 year)")]
    ExpiryTooFar,

    #[msg("Protocol is currently paused")]
    ProtocolPaused,

    #[msg("Invalid fee basis points; must be <= 1000 (10%)")]
    InvalidFeeBps,

    #[msg("Arithmetic overflow")]
    Overflow,
}