// programs/wager/src/create_wager.rs
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::{state::*, errors::WagerError};

const MAX_EXPIRY_SECONDS: i64 = 365 * 24 * 60 * 60; // 1 year

// ─── Instruction: Create Wager ────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct CreateWagerArgs {
    pub description: String,
    pub stake_usdc: u64,
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
pub struct InitializeWager<'info> {
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

    #[account(mut)]
    pub initiator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_wager(
    ctx: Context<InitializeWager>,
    args: CreateWagerArgs,
) -> Result<()> {
    // ── Validation ────────────────────────────────────────────────────────────
    require!(!ctx.accounts.config.paused, WagerError::ProtocolPaused);
    require!(args.stake_usdc > 0, WagerError::ZeroStake);
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
    wager.stake_usdc                = args.stake_usdc;
    wager.description               = args.description.clone();
    wager.status                    = WagerStatus::Initialized;
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

    // ── Increment registry ─────────────────────────────────────────────────────
    ctx.accounts.registry.wager_count = wager_id
        .checked_add(1)
        .ok_or(WagerError::Overflow)?;

    msg!(
        "Wager #{} initialized by {}. Stake: {} micro-USDC. Expires: {}",
        wager_id,
        ctx.accounts.initiator.key(),
        args.stake_usdc,
        args.expiry_ts,
    );

    Ok(())
}

#[derive(Accounts)]
pub struct FundWager<'info> {
    /// Global config — for USDC mint validation
    #[account(
        seeds = [b"config"],
        bump = config.bump,
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
        has_one = initiator @ WagerError::UnauthorizedInitiator,
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

    /// Initiator's USDC token account
    #[account(
        mut,
        constraint = initiator_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = initiator_token_account.owner == initiator.key() @ WagerError::UnauthorizedInitiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub initiator: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_fund_wager(ctx: Context<FundWager>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;
    require!(wager.status == WagerStatus::Initialized, WagerError::NotInitialized);

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.initiator_token_account.to_account_info(),
                to: ctx.accounts.escrow_token_account.to_account_info(),
                authority: ctx.accounts.initiator.to_account_info(),
            },
        ),
        wager.stake_usdc,
    )?;

    wager.status = WagerStatus::Pending;

    msg!(
        "Wager #{} funded by {}. Stake: {} micro-USDC.",
        wager.wager_id,
        ctx.accounts.initiator.key(),
        wager.stake_usdc,
    );

    Ok(())
}

// ─── Instruction: Accept Wager ────────────────────────────────────────────────

#[derive(Accounts)]
pub struct AcceptWager<'info> {
    /// Global config — for USDC mint validation
    #[account(
        seeds = [b"config"],
        bump = config.bump,
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

    /// Challenger's USDC token account
    #[account(
        mut,
        constraint = challenger_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = challenger_token_account.owner == challenger.key() @ WagerError::UnauthorizedChallenger,
    )]
    pub challenger_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = challenger.key() != wager.initiator @ WagerError::SelfChallenge,
    )]
    pub challenger: Signer<'info>,

    pub token_program: Program<'info, Token>,
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

    // ── Transfer challenger's matching USDC stake to escrow ───────────────────
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.challenger_token_account.to_account_info(),
                to: ctx.accounts.escrow_token_account.to_account_info(),
                authority: ctx.accounts.challenger.to_account_info(),
            },
        ),
        wager.stake_usdc,
    )?;

    msg!(
        "Wager #{} accepted by {}. Total escrow: {} micro-USDC.",
        wager.wager_id,
        ctx.accounts.challenger.key(),
        wager.stake_usdc * 2,
    );

    Ok(())
}

// ─── Instruction: Cancel Wager ────────────────────────────────────────────────

#[derive(Accounts)]
pub struct CancelWager<'info> {
    /// Global config — for USDC mint validation
    #[account(
        seeds = [b"config"],
        bump = config.bump,
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
        has_one = initiator @ WagerError::UnauthorizedInitiator,
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

    /// Initiator's USDC token account to receive refund
    #[account(
        mut,
        constraint = initiator_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = initiator_token_account.owner == initiator.key() @ WagerError::UnauthorizedInitiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub initiator: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_cancel_wager(ctx: Context<CancelWager>) -> Result<()> {
    let wager = &mut ctx.accounts.wager;
    require!(wager.status == WagerStatus::Pending, WagerError::NotPending);

    wager.status = WagerStatus::Cancelled;

    // Refund initiator's USDC stake from escrow using PDA signed transfer
    let refund_amt = ctx.accounts.escrow_token_account.amount;
    if refund_amt > 0 {
        let _wager_key = wager.key();
        let bump = wager.bump;
        let initiator_key = wager.initiator;
        let wager_id_bytes = wager.wager_id.to_le_bytes();
        let seeds: &[&[u8]] = &[
            b"wager",
            initiator_key.as_ref(),
            wager_id_bytes.as_ref(),
            &[bump],
        ];

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
            refund_amt,
        )?;
    }

    msg!("Wager #{} cancelled. {} micro-USDC refunded.", wager.wager_id, refund_amt);
    Ok(())
}

// ─── Instruction: Expire Wager ────────────────────────────────────────────────
/// Anyone can call this after expiry to close the PDA and refund the initiator.

#[derive(Accounts)]
pub struct ExpireWager<'info> {
    /// Global config — for USDC mint validation
    #[account(
        seeds = [b"config"],
        bump = config.bump,
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

    /// Initiator's USDC token account to receive refund
    /// CHECK: Receives the refund — must match wager.initiator
    #[account(
        mut,
        constraint = initiator_token_account.mint == usdc_mint.key() @ WagerError::InvalidUsdcMint,
        constraint = initiator_token_account.owner == wager.initiator @ WagerError::UnauthorizedInitiator,
    )]
    pub initiator_token_account: Account<'info, TokenAccount>,

    /// Anyone can crank expiry
    pub crank: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_expire_wager(ctx: Context<ExpireWager>) -> Result<()> {
    let clock = Clock::get()?;
    let wager  = &mut ctx.accounts.wager;

    require!(wager.status == WagerStatus::Pending, WagerError::NotPending);
    require!(clock.unix_timestamp >= wager.expiry_ts, WagerError::WagerNotExpired);

    wager.status = WagerStatus::Expired;

    let refund_amt = ctx.accounts.escrow_token_account.amount;
    if refund_amt > 0 {
        let _wager_key = wager.key();
        let bump = wager.bump;
        let initiator_key = wager.initiator;
        let wager_id_bytes = wager.wager_id.to_le_bytes();
        let seeds: &[&[u8]] = &[
            b"wager",
            initiator_key.as_ref(),
            wager_id_bytes.as_ref(),
            &[bump],
        ];

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
            refund_amt,
        )?;
    }

    msg!("Wager #{} expired. {} micro-USDC refunded.", wager.wager_id, refund_amt);
    Ok(())
}
