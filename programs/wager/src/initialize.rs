// programs/wager/src/initialize.rs
use anchor_lang::prelude::*;
use crate::{state::*, errors::WagerError};

// ─── Initialize Protocol Config ───────────────────────────────────────────────

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer  = admin,
        space  = ProtocolConfig::LEN,
        seeds  = [b"config"],
        bump,
    )]
    pub config: Account<'info, ProtocolConfig>,

    /// Treasury account that receives protocol fees
    /// CHECK: Any account can be a treasury; validated by admin
    pub treasury: UncheckedAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_protocol(
    ctx: Context<InitializeProtocol>,
    default_fee_bps: u16,
    dispute_window_seconds: i64,
) -> Result<()> {
    require!(default_fee_bps <= 1000, WagerError::InvalidFeeBps);
    require!(dispute_window_seconds > 0, WagerError::ExpiryInPast);

    let config = &mut ctx.accounts.config;
    config.bump                  = ctx.bumps.config;
    config.admin                 = ctx.accounts.admin.key();
    config.treasury              = ctx.accounts.treasury.key();
    config.default_fee_bps       = default_fee_bps;
    config.dispute_window_seconds = dispute_window_seconds;
    config.paused                = false;

    msg!("Protocol initialized. Fee: {} bps, Dispute window: {}s",
        default_fee_bps, dispute_window_seconds);
    Ok(())
}

// ─── Initialize User Registry ─────────────────────────────────────────────────

#[derive(Accounts)]
pub struct InitializeRegistry<'info> {
    #[account(
        init,
        payer = authority,
        space = WagerRegistry::LEN,
        seeds = [b"registry", authority.key().as_ref()],
        bump,
    )]
    pub registry: Account<'info, WagerRegistry>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_registry(ctx: Context<InitializeRegistry>) -> Result<()> {
    let registry = &mut ctx.accounts.registry;
    registry.bump         = ctx.bumps.registry;
    registry.authority    = ctx.accounts.authority.key();
    registry.wager_count  = 0;

    msg!("Registry initialized for {}", ctx.accounts.authority.key());
    Ok(())
}

// ─── Admin: Pause / Unpause ────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct SetProtocolPause<'info> {
    #[account(
        mut,
        seeds  = [b"config"],
        bump   = config.bump,
        has_one = admin @ WagerError::UnauthorizedAdmin,
    )]
    pub config: Account<'info, ProtocolConfig>,

    pub admin: Signer<'info>,
}

pub fn handle_set_pause(ctx: Context<SetProtocolPause>, paused: bool) -> Result<()> {
    ctx.accounts.config.paused = paused;
    msg!("Protocol paused: {}", paused);
    Ok(())
}

// ─── Admin: Update Fee ─────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct UpdateFee<'info> {
    #[account(
        mut,
        seeds  = [b"config"],
        bump   = config.bump,
        has_one = admin @ WagerError::UnauthorizedAdmin,
    )]
    pub config: Account<'info, ProtocolConfig>,

    pub admin: Signer<'info>,
}

pub fn handle_update_fee(ctx: Context<UpdateFee>, new_fee_bps: u16) -> Result<()> {
    require!(new_fee_bps <= 1000, WagerError::InvalidFeeBps);
    ctx.accounts.config.default_fee_bps = new_fee_bps;
    msg!("Protocol fee updated to {} bps", new_fee_bps);
    Ok(())
}
