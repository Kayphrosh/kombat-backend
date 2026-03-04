// programs/wager/src/initialize.rs
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token};
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

    /// USDC SPL Token mint
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
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
    config.usdc_mint             = ctx.accounts.usdc_mint.key();
    config.default_fee_bps       = default_fee_bps;
    config.dispute_window_seconds = dispute_window_seconds;
    config.paused                = false;

    msg!("Protocol initialized. Fee: {} bps, Dispute window: {}s, USDC mint: {}",
        default_fee_bps, dispute_window_seconds, config.usdc_mint);
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

// ─── Admin: Migrate Config (one-time USDC migration) ──────────────────────────

#[derive(Accounts)]
pub struct MigrateConfig<'info> {
    /// CHECK: We manually validate and reallocate this account
    #[account(
        mut,
        seeds = [b"config"],
        bump,
    )]
    pub config: AccountInfo<'info>,

    /// USDC SPL Token mint
    pub usdc_mint: Account<'info, token::Mint>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_migrate_config(ctx: Context<MigrateConfig>) -> Result<()> {
    let config_info = &ctx.accounts.config;
    let admin = &ctx.accounts.admin;
    let usdc_mint_key = ctx.accounts.usdc_mint.key();
    
    // Check owner is our program
    require!(
        config_info.owner == ctx.program_id,
        WagerError::UnauthorizedAdmin
    );
    
    let current_len = config_info.data_len();
    let new_len = ProtocolConfig::LEN;
    
    msg!("Current config length: {}, new length: {}", current_len, new_len);
    
    // Verify admin from the account data (admin is at offset 8+1=9, after discriminator and bump)
    let data = config_info.try_borrow_data()?;
    let stored_admin = Pubkey::try_from(&data[9..41]).map_err(|_| WagerError::UnauthorizedAdmin)?;
    require!(stored_admin == admin.key(), WagerError::UnauthorizedAdmin);
    drop(data);
    
    // Reallocate if needed
    if current_len < new_len {
        let rent = Rent::get()?;
        let new_minimum_balance = rent.minimum_balance(new_len);
        let current_balance = config_info.lamports();
        
        if current_balance < new_minimum_balance {
            let diff = new_minimum_balance - current_balance;
            // Transfer lamports from admin to config
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: admin.to_account_info(),
                    to: config_info.clone(),
                },
            );
            anchor_lang::system_program::transfer(cpi_ctx, diff)?;
        }
        
        config_info.realloc(new_len, false)?;
        msg!("Reallocated config from {} to {} bytes", current_len, new_len);
    }
    
    // Write the usdc_mint (at the end of the struct: offset 8+1+32+32+2+8+1 = 84)
    let mut data = config_info.try_borrow_mut_data()?;
    let usdc_mint_offset = 84; // After: discrim(8) + bump(1) + admin(32) + treasury(32) + fee_bps(2) + dispute_window(8) + paused(1)
    
    // Check if usdc_mint is already set
    let current_mint = Pubkey::try_from(&data[usdc_mint_offset..usdc_mint_offset+32]).unwrap_or_default();
    
    if current_mint == Pubkey::default() {
        data[usdc_mint_offset..usdc_mint_offset+32].copy_from_slice(usdc_mint_key.as_ref());
        msg!("Config migrated. USDC mint set to: {}", usdc_mint_key);
    } else {
        msg!("Config already migrated. USDC mint: {}", current_mint);
    }
    
    Ok(())
}
