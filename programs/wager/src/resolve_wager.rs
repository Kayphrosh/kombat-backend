// programs/wager/src/resolve_wager.rs
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::{state::*, errors::WagerError};

// ─── Shared payout helper ─────────────────────────────────────────────────────
/// Uses PDA-signed SPL token transfer to move USDC from escrow to winner and treasury.

fn payout_winner<'info>(
    wager: &Account<'info, Wager>,
    escrow_token_account: &Account<'info, TokenAccount>,
    winner_token_account: &Account<'info, TokenAccount>,
    treasury_token_account: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let total_pot = escrow_token_account.amount;
    let fee_amount = (total_pot as u128)
        .checked_mul(wager.protocol_fee_bps as u128)
        .ok_or(WagerError::Overflow)?
        .checked_div(10_000)
        .ok_or(WagerError::Overflow)? as u64;

    let winner_payout = total_pot
        .checked_sub(fee_amount)
        .ok_or(WagerError::Overflow)?;

    // Build PDA signer seeds for the wager account (which owns the escrow ATA)
    let initiator_key = wager.initiator;
    let wager_id_bytes = wager.wager_id.to_le_bytes();
    let bump = wager.bump;
    let seeds: &[&[u8]] = &[
        b"wager",
        initiator_key.as_ref(),
        wager_id_bytes.as_ref(),
        &[bump],
    ];

    // Transfer winner's payout
    if winner_payout > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                Transfer {
                    from: escrow_token_account.to_account_info(),
                    to: winner_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            winner_payout,
        )?;
    }

    // Transfer protocol fee to treasury
    if fee_amount > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                Transfer {
                    from: escrow_token_account.to_account_info(),
                    to: treasury_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            fee_amount,
        )?;
    }

    msg!(
        "Payout: {} micro-USDC to winner, {} micro-USDC fee to treasury",
        winner_payout,
        fee_amount
    );

    Ok(())
}

// ─── Instruction: Resolve by Arbitrator ───────────────────────────────────────

#[derive(Accounts)]
pub struct ResolveByArbitrator<'info> {
    #[account(
        seeds  = [b"config"],
        bump   = config.bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [
            b"wager",
            wager.initiator.as_ref(),
            wager.wager_id.to_le_bytes().as_ref(),
        ],
        bump = wager.bump,
        constraint = wager.resolution_source == ResolutionSource::Arbitrator
            @ WagerError::UnauthorizedResolver,
        has_one = resolver @ WagerError::UnauthorizedResolver,
    )]
    pub wager: Account<'info, Wager>,

    /// USDC mint account
    #[account(
        constraint = usdc_mint.key() == config.usdc_mint @ WagerError::InvalidUsdcMint
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    /// Escrow token account (PDA-owned ATA) that holds USDC stakes
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = wager,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// The declared winner's USDC token account
    #[account(
        mut,
        constraint = winner_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub winner_token_account: Account<'info, TokenAccount>,

    /// Treasury's USDC token account for protocol fees
    #[account(
        mut,
        constraint = treasury_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    /// The arbitrator — must match wager.resolver
    pub resolver: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_resolve_by_arbitrator(ctx: Context<ResolveByArbitrator>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;
    let winner_key = ctx.accounts.winner_token_account.owner;

    // ── Lifecycle guard ────────────────────────────────────────────────────────
    require!(
        wager.status == WagerStatus::Active || wager.status == WagerStatus::Disputed,
        WagerError::NotActive
    );

    // ── Validate winner is a participant ──────────────────────────────────────
    let is_initiator  = winner_key == wager.initiator;
    let is_challenger = Some(winner_key) == wager.challenger;
    require!(is_initiator || is_challenger, WagerError::InvalidWinner);

    // ── Mark resolved ──────────────────────────────────────────────────────────
    let clock = Clock::get()?;
    wager.status      = WagerStatus::Resolved;
    wager.winner      = Some(winner_key);
    wager.resolved_at = clock.unix_timestamp;

    msg!(
        "Wager #{} resolved by arbitrator. Winner: {}",
        wager.wager_id,
        winner_key
    );

    payout_winner(
        wager,
        &ctx.accounts.escrow_token_account,
        &ctx.accounts.winner_token_account,
        &ctx.accounts.treasury_token_account,
        &ctx.accounts.token_program,
    )
}

// ─── Instruction: Mutual Consent Resolution ───────────────────────────────────
/// Either participant calls this to register their consent.
/// When both have consented, the winner is paid out automatically.

#[derive(Accounts)]
pub struct ConsentResolve<'info> {
    #[account(
        seeds  = [b"config"],
        bump   = config.bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [
            b"wager",
            wager.initiator.as_ref(),
            wager.wager_id.to_le_bytes().as_ref(),
        ],
        bump = wager.bump,
        constraint = wager.resolution_source == ResolutionSource::MutualConsent
            @ WagerError::UnauthorizedResolver,
    )]
    pub wager: Account<'info, Wager>,

    /// USDC mint account
    #[account(
        constraint = usdc_mint.key() == config.usdc_mint @ WagerError::InvalidUsdcMint
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    /// Escrow token account (PDA-owned ATA) that holds USDC stakes
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = wager,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// The participant giving consent
    pub participant: Signer<'info>,

    /// The declared winner's USDC token account
    #[account(
        mut,
        constraint = winner_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub winner_token_account: Account<'info, TokenAccount>,

    /// Treasury's USDC token account for protocol fees
    #[account(
        mut,
        constraint = treasury_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_consent_resolve(ctx: Context<ConsentResolve>) -> Result<()> {
    let wager       = &mut ctx.accounts.wager;
    let participant = ctx.accounts.participant.key();
    let winner_key  = ctx.accounts.winner_token_account.owner;

    require!(wager.status == WagerStatus::Active, WagerError::NotActive);

    // ── Validate winner ────────────────────────────────────────────────────────
    let is_initiator  = winner_key == wager.initiator;
    let is_challenger = Some(winner_key) == wager.challenger;
    require!(is_initiator || is_challenger, WagerError::InvalidWinner);

    // ── Register consent ───────────────────────────────────────────────────────
    if participant == wager.initiator {
        wager.initiator_consent = true;
    } else if Some(participant) == wager.challenger {
        wager.challenger_consent = true;
    } else {
        return err!(WagerError::NotAParticipant);
    }

    msg!(
        "Participant {} consented. Initiator: {}, Challenger: {}",
        participant,
        wager.initiator_consent,
        wager.challenger_consent,
    );

    // ── Both consented — pay out ───────────────────────────────────────────────
    if wager.initiator_consent && wager.challenger_consent {
        let clock = Clock::get()?;
        wager.status      = WagerStatus::Resolved;
        wager.winner      = Some(winner_key);
        wager.resolved_at = clock.unix_timestamp;

        msg!("Mutual consent reached. Paying out winner: {}", winner_key);

        payout_winner(
            wager,
            &ctx.accounts.escrow_token_account,
            &ctx.accounts.winner_token_account,
            &ctx.accounts.treasury_token_account,
            &ctx.accounts.token_program,
        )?;
    }

    Ok(())
}

// ─── Instruction: Resolve by Oracle Feed ──────────────────────────────────────
/// Reads a Switchboard/Pyth-compatible on-chain price account and
/// determines the winner based on oracle_target and oracle_initiator_wins_above.

#[derive(Accounts)]
pub struct ResolveByOracle<'info> {
    #[account(
        seeds  = [b"config"],
        bump   = config.bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [
            b"wager",
            wager.initiator.as_ref(),
            wager.wager_id.to_le_bytes().as_ref(),
        ],
        bump = wager.bump,
        constraint = wager.resolution_source == ResolutionSource::OracleFeed
            @ WagerError::UnauthorizedResolver,
    )]
    pub wager: Account<'info, Wager>,

    /// USDC mint account
    #[account(
        constraint = usdc_mint.key() == config.usdc_mint @ WagerError::InvalidUsdcMint
    )]
    pub usdc_mint: Account<'info, token::Mint>,

    /// Escrow token account (PDA-owned ATA) that holds USDC stakes
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = wager,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// The oracle price/result feed (Switchboard or Pyth aggregator)
    /// CHECK: Validated against wager.oracle_feed
    pub oracle_feed: UncheckedAccount<'info>,

    /// Initiator's USDC token account for possible payout
    #[account(
        mut,
        constraint = initiator_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = initiator_token_account.owner == wager.initiator @ WagerError::UnauthorizedInitiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// Challenger's USDC token account for possible payout
    #[account(
        mut,
        constraint = challenger_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub challenger_token_account: Account<'info, TokenAccount>,

    /// Treasury's USDC token account for protocol fees
    #[account(
        mut,
        constraint = treasury_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,

    /// Anyone can crank the oracle resolution once the condition is met
    pub crank: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_resolve_by_oracle(ctx: Context<ResolveByOracle>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;

    require!(wager.status == WagerStatus::Active, WagerError::NotActive);

    // ── Validate oracle feed account ───────────────────────────────────────────
    let expected_feed = wager.oracle_feed.ok_or(WagerError::OracleFeedMismatch)?;
    require!(
        ctx.accounts.oracle_feed.key() == expected_feed,
        WagerError::OracleFeedMismatch
    );

    // ── Read oracle value ──────────────────────────────────────────────────────
    // Generic raw-byte parsing — works with Switchboard V2 AggregatorAccountData.
    // For Pyth, swap to pyth_sdk_solana::load_price_feed_from_account_info.
    let oracle_value = read_oracle_value(&ctx.accounts.oracle_feed)?;

    // ── Determine winner ───────────────────────────────────────────────────────
    let initiator_wins = if wager.oracle_initiator_wins_above {
        oracle_value >= wager.oracle_target
    } else {
        oracle_value < wager.oracle_target
    };

    let (winner_token_account, winner_key) = if initiator_wins {
        (&ctx.accounts.initiator_token_account, wager.initiator)
    } else {
        let challenger_key = wager.challenger.ok_or(WagerError::NotActive)?;
        require!(
            ctx.accounts.challenger_token_account.owner == challenger_key,
            WagerError::InvalidWinner
        );
        (&ctx.accounts.challenger_token_account, challenger_key)
    };

    // ── Mark resolved ──────────────────────────────────────────────────────────
    let clock = Clock::get()?;
    wager.status      = WagerStatus::Resolved;
    wager.winner      = Some(winner_key);
    wager.resolved_at = clock.unix_timestamp;

    msg!(
        "Wager #{} resolved by oracle. Value: {}, Target: {}, Winner: {}",
        wager.wager_id,
        oracle_value,
        wager.oracle_target,
        winner_key,
    );

    payout_winner(
        wager,
        &ctx.accounts.escrow_token_account,
        winner_token_account,
        &ctx.accounts.treasury_token_account,
        &ctx.accounts.token_program,
    )
}

/// Parse the i64 result from a Switchboard-V2 aggregator account.
/// Reads `result.mantissa` at byte offset 65 in the raw account data.
fn read_oracle_value(feed: &UncheckedAccount) -> Result<i64> {
    let data = feed.try_borrow_data().map_err(|_| WagerError::OraclePriceInvalid)?;
    require!(data.len() >= 73, WagerError::OraclePriceInvalid);

    // Switchboard V2: SwitchboardDecimal { mantissa: i128 (offset 65), scale: u32 }
    let mantissa = i128::from_le_bytes(
        data[65..73].try_into().map_err(|_| WagerError::OraclePriceInvalid)?
    );

    // Truncate to i64 for comparison (precision sufficient for most use-cases)
    let value = i64::try_from(mantissa).map_err(|_| WagerError::OraclePriceInvalid)?;
    Ok(value)
}
