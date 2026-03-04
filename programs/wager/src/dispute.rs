// programs/wager/src/dispute.rs
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::{state::*, errors::WagerError};

// ─── Instruction: Open Dispute ────────────────────────────────────────────────
/// A participant may open a dispute on an Active wager.
/// Once disputed, only the designated resolver/arbitrator can settle it.

#[derive(Accounts)]
pub struct OpenDispute<'info> {
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
    )]
    pub wager: Account<'info, Wager>,

    /// Must be initiator or challenger
    pub participant: Signer<'info>,
}

pub fn handle_open_dispute(ctx: Context<OpenDispute>) -> Result<()> {
    let wager       = &mut ctx.accounts.wager;
    let participant = ctx.accounts.participant.key();

    // ── Lifecycle guard ────────────────────────────────────────────────────────
    require!(wager.status == WagerStatus::Active, WagerError::NotActive);
    require!(wager.dispute_opened_at == 0, WagerError::AlreadyDisputed);

    // ── Ensure caller is a participant ─────────────────────────────────────────
    let is_initiator  = participant == wager.initiator;
    let is_challenger = Some(participant) == wager.challenger;
    require!(is_initiator || is_challenger, WagerError::NotAParticipant);

    let clock = Clock::get()?;

    // ── Dispute window must still be open ──────────────────────────────────────
    let dispute_deadline = wager.expiry_ts
        .checked_add(ctx.accounts.config.dispute_window_seconds)
        .ok_or(WagerError::Overflow)?;
    require!(
        clock.unix_timestamp <= dispute_deadline,
        WagerError::DisputeWindowClosed
    );

    // ── Transition to Disputed ────────────────────────────────────────────────
    wager.status             = WagerStatus::Disputed;
    wager.dispute_opened_at  = clock.unix_timestamp;
    wager.dispute_opener     = Some(participant);

    msg!(
        "Dispute opened on wager #{} by {} at {}",
        wager.wager_id,
        participant,
        clock.unix_timestamp
    );

    Ok(())
}

// ─── Instruction: Settle Dispute (Admin override) ─────────────────────────────
/// The protocol admin can settle any disputed wager as a last resort,
/// overriding the normal arbitrator flow.

#[derive(Accounts)]
pub struct SettleDispute<'info> {
    #[account(
        seeds  = [b"config"],
        bump   = config.bump,
        has_one = admin @ WagerError::UnauthorizedAdmin,
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

    /// The winner's USDC token account declared by admin
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

    pub admin: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_settle_dispute(ctx: Context<SettleDispute>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;
    let winner_key = ctx.accounts.winner_token_account.owner;

    // ── Must be in disputed state ────────────────────────────────────────────
    require!(wager.status == WagerStatus::Disputed, WagerError::NotDisputed);

    // ── Winner must be a participant ─────────────────────────────────────────
    let is_initiator  = winner_key == wager.initiator;
    let is_challenger = Some(winner_key) == wager.challenger;
    require!(is_initiator || is_challenger, WagerError::InvalidWinner);

    let clock = Clock::get()?;
    wager.status      = WagerStatus::Resolved;
    wager.winner      = Some(winner_key);
    wager.resolved_at = clock.unix_timestamp;

    msg!(
        "Dispute on wager #{} settled by admin. Winner: {}",
        wager.wager_id,
        winner_key
    );

    // ── Payout using PDA-signed SPL token transfer ─────────────────────────
    let total_pot = ctx.accounts.escrow_token_account.amount;
    let fee_amount = (total_pot as u128)
        .checked_mul(wager.protocol_fee_bps as u128)
        .ok_or(WagerError::Overflow)?
        .checked_div(10_000)
        .ok_or(WagerError::Overflow)? as u64;
    let winner_payout = total_pot.checked_sub(fee_amount).ok_or(WagerError::Overflow)?;

    let initiator_key = wager.initiator;
    let wager_id_bytes = wager.wager_id.to_le_bytes();
    let bump = wager.bump;
    let seeds: &[&[u8]] = &[
        b"wager",
        initiator_key.as_ref(),
        wager_id_bytes.as_ref(),
        &[bump],
    ];

    if winner_payout > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.winner_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            winner_payout,
        )?;
    }

    if fee_amount > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.treasury_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            fee_amount,
        )?;
    }

    msg!(
        "Settled payout: {} micro-USDC to winner, {} fee to treasury",
        winner_payout,
        fee_amount
    );

    Ok(())
}

// ─── Instruction: Close Expired Dispute ───────────────────────────────────────
/// If a dispute sits unresolved past the dispute window + grace period,
/// refund both parties their original stakes (minus a small protocol fee).

#[derive(Accounts)]
pub struct CloseExpiredDispute<'info> {
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

    /// Initiator's USDC token account for refund
    #[account(
        mut,
        constraint = initiator_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = initiator_token_account.owner == wager.initiator @ WagerError::UnauthorizedInitiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// Challenger's USDC token account for refund
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

    /// Anyone can crank expiry
    pub crank: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

/// Grace period after dispute_window before stale disputes can be force-closed.
const STALE_DISPUTE_GRACE_SECONDS: i64 = 7 * 24 * 60 * 60; // 7 days

pub fn handle_close_expired_dispute(ctx: Context<CloseExpiredDispute>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;

    require!(wager.status == WagerStatus::Disputed, WagerError::NotDisputed);

    let clock = Clock::get()?;
    let stale_threshold = wager
        .dispute_opened_at
        .checked_add(STALE_DISPUTE_GRACE_SECONDS)
        .ok_or(WagerError::Overflow)?;

    require!(
        clock.unix_timestamp > stale_threshold,
        WagerError::DisputeWindowClosed
    );

    require!(
        ctx.accounts.challenger_token_account.owner == wager.challenger.ok_or(WagerError::NotActive)?,
        WagerError::InvalidWinner
    );

    wager.status = WagerStatus::Expired;

    // ── Refund both parties equally minus protocol fee, using PDA-signed transfers ───
    let total_escrow = ctx.accounts.escrow_token_account.amount;
    let fee_amount = (total_escrow as u128)
        .checked_mul(wager.protocol_fee_bps as u128)
        .ok_or(WagerError::Overflow)?
        .checked_div(10_000)
        .ok_or(WagerError::Overflow)? as u64;

    let refundable = total_escrow.checked_sub(fee_amount).ok_or(WagerError::Overflow)?;
    let each_refund = refundable / 2;

    let initiator_key = wager.initiator;
    let wager_id_bytes = wager.wager_id.to_le_bytes();
    let bump = wager.bump;
    let seeds: &[&[u8]] = &[
        b"wager",
        initiator_key.as_ref(),
        wager_id_bytes.as_ref(),
        &[bump],
    ];

    // Refund initiator
    if each_refund > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.initiator_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            each_refund,
        )?;
    }

    // Refund challenger
    if each_refund > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.challenger_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            each_refund,
        )?;
    }

    // Protocol fee to treasury
    if fee_amount > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.treasury_token_account.to_account_info(),
                    authority: wager.to_account_info(),
                },
                &[seeds],
            ),
            fee_amount,
        )?;
    }

    msg!(
        "Stale dispute on wager #{} force-closed. Each party refunded {} micro-USDC.",
        wager.wager_id,
        each_refund
    );

    Ok(())
}
