// programs/wager/src/create_wager.rs
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::{state::*, errors::WagerError};

const MAX_EXPIRY_SECONDS: i64 = 365 * 24 * 60 * 60; // 1 year

// ─── Instruction: Create Wager ────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct CreateWagerArgs {
    pub description: String,
    pub stake_lamports: u64,
    pub expiry_ts: i64,
    pub resolution_source: ResolutionSource,
    pub resolver: Pubkey,
    /// Only required when resolution_source = OracleFeed
    pub oracle_feed: Option<Pubkey>,
    pub oracle_target: Option<i64>,
    pub oracle_initiator_wins_above: Option<bool>,
}

#[derive(Accounts)]
#[instruction(args: CreateWagerArgs)]
pub struct CreateWager<'info> {
    /// Global config — checks protocol is not paused
    #[account(
        seeds  = [b"config"],
        bump   = config.bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    /// Initiator's registry — seeds bind it to signer, proving ownership
    #[account(
        mut,
        seeds  = [b"registry", initiator.key().as_ref()],
        bump   = registry.bump,
        constraint = registry.authority == initiator.key() @ WagerError::UnauthorizedInitiator,
    )]
    pub registry: Account<'info, WagerRegistry>,

    /// The wager PDA — unique per (initiator, wager_id)
    #[account(
        init,
        payer  = initiator,
        space  = Wager::LEN,
        seeds  = [
            b"wager",
            initiator.key().as_ref(),
            registry.wager_count.to_le_bytes().as_ref(),
        ],
        bump,
    )]
    pub wager: Account<'info, Wager>,

    /// Escrow PDA that holds both parties' stakes
    /// CHECK: Validated by seeds; receives lamports directly
    #[account(
        mut,
        seeds  = [b"escrow", wager.key().as_ref()],
        bump,
    )]
    pub escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub initiator: Signer<'info>,


    pub system_program: Program<'info, System>,
}

pub fn handle_create_wager(
    ctx: Context<CreateWager>,
    args: CreateWagerArgs,
) -> Result<()> {
    // ── Validation ────────────────────────────────────────────────────────────
    require!(!ctx.accounts.config.paused, WagerError::ProtocolPaused);
    require!(args.stake_lamports > 0, WagerError::ZeroStake);
    require!(args.description.len() <= 256, WagerError::DescriptionTooLong);

    let clock = Clock::get()?;
    require!(args.expiry_ts > clock.unix_timestamp, WagerError::ExpiryInPast);
    require!(
        args.expiry_ts <= clock.unix_timestamp + MAX_EXPIRY_SECONDS,
        WagerError::ExpiryTooFar
    );

    // ── Populate wager state ───────────────────────────────────────────────────
    let wager_id = ctx.accounts.registry.wager_count;
    let wager    = &mut ctx.accounts.wager;

    wager.bump                      = ctx.bumps.wager;
    wager.wager_id                  = wager_id;
    wager.initiator                 = ctx.accounts.initiator.key();
    wager.challenger                = None;
    wager.stake_lamports            = args.stake_lamports;
    wager.description               = args.description.clone();
    wager.status                    = WagerStatus::Pending;
    wager.resolution_source         = args.resolution_source;
    wager.resolver                  = args.resolver;
    wager.expiry_ts                 = args.expiry_ts;
    wager.created_at                = clock.unix_timestamp;
    wager.resolved_at               = 0;
    wager.winner                    = None;
    wager.protocol_fee_bps          = ctx.accounts.config.default_fee_bps;
    wager.initiator_consent         = false;
    wager.challenger_consent        = false;
    wager.dispute_opened_at         = 0;
    wager.dispute_opener            = None;
    wager.oracle_feed               = args.oracle_feed;
    wager.oracle_target             = args.oracle_target.unwrap_or(0);
    wager.oracle_initiator_wins_above = args.oracle_initiator_wins_above.unwrap_or(true);

    // ── Transfer initiator's stake to escrow ──────────────────────────────────
    let escrow_bump = ctx.bumps.escrow;
    let _ = escrow_bump; // used in accept_wager payout

    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.initiator.to_account_info(),
                to:   ctx.accounts.escrow.to_account_info(),
            },
        ),
        args.stake_lamports,
    )?;

    // ── Increment registry ─────────────────────────────────────────────────────
    ctx.accounts.registry.wager_count = wager_id
        .checked_add(1)
        .ok_or(WagerError::Overflow)?;

    msg!(
        "Wager #{} created by {}. Stake: {} lamports. Expires: {}",
        wager_id,
        ctx.accounts.initiator.key(),
        args.stake_lamports,
        args.expiry_ts,
    );

    Ok(())
}

// ─── Instruction: Accept Wager ────────────────────────────────────────────────

#[derive(Accounts)]
pub struct AcceptWager<'info> {
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

    /// Escrow that holds the initiator's stake
    /// CHECK: Validated by seeds
    #[account(
        mut,
        seeds = [b"escrow", wager.key().as_ref()],
        bump,
    )]
    pub escrow: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = challenger.key() != wager.initiator @ WagerError::SelfChallenge,
    )]
    pub challenger: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_accept_wager(ctx: Context<AcceptWager>) -> Result<()> {
    let clock = Clock::get()?;
    let wager  = &mut ctx.accounts.wager;

    // ── Lifecycle guard ────────────────────────────────────────────────────────
    require!(wager.status == WagerStatus::Pending, WagerError::NotPending);
    require!(clock.unix_timestamp < wager.expiry_ts, WagerError::WagerExpired);

    // ── Lock in challenger ─────────────────────────────────────────────────────
    wager.challenger = Some(ctx.accounts.challenger.key());
    wager.status     = WagerStatus::Active;

    // ── Transfer challenger's matching stake to escrow ────────────────────────
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.challenger.to_account_info(),
                to:   ctx.accounts.escrow.to_account_info(),
            },
        ),
        wager.stake_lamports,
    )?;

    msg!(
        "Wager #{} accepted by {}. Total escrow: {} lamports.",
        wager.wager_id,
        ctx.accounts.challenger.key(),
        wager.stake_lamports * 2,
    );

    Ok(())
}

// ─── Instruction: Cancel Wager ────────────────────────────────────────────────

#[derive(Accounts)]
pub struct CancelWager<'info> {
    #[account(
        mut,
        seeds = [
            b"wager",
            wager.initiator.as_ref(),
            wager.wager_id.to_le_bytes().as_ref(),
        ],
        bump = wager.bump,
        has_one = initiator @ WagerError::UnauthorizedInitiator,
    )]
    pub wager: Account<'info, Wager>,

    /// CHECK: Validated by seeds
    #[account(
        mut,
        seeds = [b"escrow", wager.key().as_ref()],
        bump,
    )]
    pub escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub initiator: Signer<'info>,
}

pub fn handle_cancel_wager(ctx: Context<CancelWager>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;
    require!(wager.status == WagerStatus::Pending, WagerError::NotPending);

    wager.status = WagerStatus::Cancelled;

    // ── Refund initiator's stake from escrow ──────────────────────────────────
    let escrow     = &ctx.accounts.escrow;
    let initiator  = &ctx.accounts.initiator;
    let refund_amt = escrow.lamports();

    **escrow.try_borrow_mut_lamports()? -= refund_amt;
    **initiator.try_borrow_mut_lamports()? += refund_amt;

    msg!("Wager #{} cancelled. {} lamports refunded.", wager.wager_id, refund_amt);
    Ok(())
}

// ─── Instruction: Expire Wager ────────────────────────────────────────────────
/// Anyone can call this after expiry to close the PDA and refund the initiator.

#[derive(Accounts)]
pub struct ExpireWager<'info> {
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

    /// CHECK: Validated by seeds
    #[account(
        mut,
        seeds = [b"escrow", wager.key().as_ref()],
        bump,
    )]
    pub escrow: UncheckedAccount<'info>,

    /// CHECK: Receives the refund — must match wager.initiator
    #[account(
        mut,
        constraint = initiator.key() == wager.initiator,
    )]
    pub initiator: UncheckedAccount<'info>,
}

pub fn handle_expire_wager(ctx: Context<ExpireWager>) -> Result<()> {
    let clock = Clock::get()?;
    let wager  = &mut ctx.accounts.wager;

    require!(wager.status == WagerStatus::Pending, WagerError::NotPending);
    require!(clock.unix_timestamp >= wager.expiry_ts, WagerError::WagerNotExpired);

    wager.status = WagerStatus::Expired;

    let escrow     = &ctx.accounts.escrow;
    let initiator  = &ctx.accounts.initiator;
    let refund_amt = escrow.lamports();

    **escrow.try_borrow_mut_lamports()? -= refund_amt;
    **initiator.try_borrow_mut_lamports()? += refund_amt;

    msg!("Wager #{} expired. {} lamports refunded.", wager.wager_id, refund_amt);
    Ok(())
}
