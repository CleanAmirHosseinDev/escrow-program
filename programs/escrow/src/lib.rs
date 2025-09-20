//! A sophisticated escrow program that includes an arbiter, cancellable
//! functionality, and on-chain state tracking via events.
//!
//! This program enhances the basic escrow concept by introducing:
//! - An `arbiter` who can resolve disputes.
//! - A `cancel` function for the initializer.
//! - Explicit on-chain `EscrowStatus` for clear state management.
//! - Events for all state transitions, allowing for easy off-chain monitoring.
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use anchor_lang::solana_program::clock::Clock;

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod escrow {
    use super::*;

    /// Initializes a new escrow agreement.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The context of accounts for the instruction.
    /// * `amount` - The amount of tokens to be held in escrow.
    /// * `timeout` - The duration (in seconds) after which the escrow can be refunded.
    pub fn initialize(ctx: Context<Initialize>, amount: u64, timeout: i64) -> Result<()> {
        require!(amount > 0, EscrowError::InvalidAmount);
        let initializer = &ctx.accounts.initializer;
        let recipient = &ctx.accounts.recipient;
        require!(
            initializer.key() != recipient.key(),
            EscrowError::InvalidRecipient
        );

        let escrow_state = &mut ctx.accounts.escrow_state;
        escrow_state.initializer = *initializer.key;
        escrow_state.recipient = *recipient.key;
        escrow_state.arbiter = *ctx.accounts.arbiter.key;
        escrow_state.amount = amount;
        escrow_state.timeout = Clock::get()?
            .unix_timestamp
            .checked_add(timeout)
            .ok_or(EscrowError::Overflow)?;
        escrow_state.status = EscrowStatus::Initialized;
        escrow_state.vault_bump = ctx.bumps.vault;
        escrow_state.escrow_bump = ctx.bumps.escrow_state;

        // Transfer tokens from initializer to the vault.
        let cpi_accounts = Transfer {
            from: ctx
                .accounts
                .initializer_deposit_token_account
                .to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: initializer.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, amount)?;

        emit!(EscrowInitialized {
            escrow: escrow_state.key(),
            initializer: *initializer.key,
            recipient: *recipient.key,
            arbiter: *ctx.accounts.arbiter.key,
            amount,
        });

        Ok(())
    }

    /// Allows the recipient to withdraw tokens from the escrow.
    pub fn withdraw(ctx: Context<Withdraw>) -> Result<()> {
        let escrow_state = &mut ctx.accounts.escrow_state;
        let recipient = &ctx.accounts.recipient;

        require!(
            escrow_state.status == EscrowStatus::Initialized,
            EscrowError::InvalidState
        );
        require!(
            Clock::get()?.unix_timestamp < escrow_state.timeout,
            EscrowError::TimeoutExpired
        );

        // Transfer tokens from the vault to the recipient.
        let escrow_key = escrow_state.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault".as_ref(),
            escrow_key.as_ref(),
            &[escrow_state.vault_bump],
        ]];
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx
                .accounts
                .recipient_deposit_token_account
                .to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx =
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, escrow_state.amount)?;

        escrow_state.status = EscrowStatus::Withdrawn;

        emit!(EscrowWithdrawn {
            escrow: escrow_state.key(),
            recipient: *recipient.key,
            amount: escrow_state.amount,
        });

        Ok(())
    }

    /// Allows the initializer to get a refund after the timeout has expired.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let escrow_state = &mut ctx.accounts.escrow_state;
        let initializer = &ctx.accounts.initializer;

        require!(
            escrow_state.status == EscrowStatus::Initialized,
            EscrowError::InvalidState
        );
        require!(
            Clock::get()?.unix_timestamp >= escrow_state.timeout,
            EscrowError::RefundNotAllowed
        );

        // Transfer tokens from the vault back to the initializer.
        let escrow_key = escrow_state.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault".as_ref(),
            escrow_key.as_ref(),
            &[escrow_state.vault_bump],
        ]];
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx
                .accounts
                .initializer_refund_token_account
                .to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx =
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, escrow_state.amount)?;

        escrow_state.status = EscrowStatus::Refunded;

        emit!(EscrowRefunded {
            escrow: escrow_state.key(),
            initializer: *initializer.key,
            amount: escrow_state.amount,
        });

        Ok(())
    }

    /// Allows the initializer to cancel the escrow before timeout.
    pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
        let escrow_state = &mut ctx.accounts.escrow_state;
        let initializer = &ctx.accounts.initializer;

        require!(
            escrow_state.status == EscrowStatus::Initialized,
            EscrowError::InvalidState
        );
        require!(
            Clock::get()?.unix_timestamp < escrow_state.timeout,
            EscrowError::CancelNotAllowed
        );

        // Transfer tokens from the vault back to the initializer.
        let escrow_key = escrow_state.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault".as_ref(),
            escrow_key.as_ref(),
            &[escrow_state.vault_bump],
        ]];
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx
                .accounts
                .initializer_refund_token_account
                .to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx =
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, escrow_state.amount)?;

        escrow_state.status = EscrowStatus::Cancelled;

        emit!(EscrowCancelled {
            escrow: escrow_state.key(),
            initializer: *initializer.key,
        });

        Ok(())
    }

    /// Allows the arbiter to resolve the dispute and release funds.
    pub fn resolve_by_arbiter(ctx: Context<ResolveByArbiter>, release_to_recipient: bool) -> Result<()> {
        let escrow_state = &mut ctx.accounts.escrow_state;

        require!(
            escrow_state.status == EscrowStatus::Initialized,
            EscrowError::InvalidState
        );

        let escrow_key = escrow_state.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault".as_ref(),
            escrow_key.as_ref(),
            &[escrow_state.vault_bump],
        ]];

        if release_to_recipient {
            // Transfer to recipient
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.recipient_deposit_token_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx =
                CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, escrow_state.amount)?;
            escrow_state.status = EscrowStatus::Withdrawn;
        } else {
            // Refund to initializer
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.initializer_refund_token_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx =
                CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, escrow_state.amount)?;
            escrow_state.status = EscrowStatus::Refunded;
        }

        emit!(EscrowResolved {
            escrow: escrow_state.key(),
            arbiter: *ctx.accounts.arbiter.key,
            release_to_recipient,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Cancel<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,
    #[account(mut)]
    pub initializer_refund_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = escrow_state.initializer == initializer.key() @ EscrowError::InvalidInitializer,
        seeds = [b"escrow", escrow_state.initializer.as_ref(), escrow_state.recipient.as_ref()],
        bump = escrow_state.escrow_bump,
    )]
    pub escrow_state: Account<'info, Escrow>,
    #[account(
        mut,
        seeds = [b"vault", escrow_state.key().as_ref()],
        bump = escrow_state.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ResolveByArbiter<'info> {
    #[account(mut)]
    pub arbiter: Signer<'info>,
    #[account(
        mut,
        constraint = escrow_state.arbiter == arbiter.key() @ EscrowError::InvalidArbiter,
        seeds = [b"escrow", escrow_state.initializer.as_ref(), escrow_state.recipient.as_ref()],
        bump = escrow_state.escrow_bump,
    )]
    pub escrow_state: Account<'info, Escrow>,
    #[account(
        mut,
        seeds = [b"vault", escrow_state.key().as_ref()],
        bump = escrow_state.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub recipient_deposit_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub initializer_refund_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}


#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,
    /// CHECK: The recipient is validated in the instruction logic.
    pub recipient: AccountInfo<'info>,
    /// CHECK: The arbiter is validated in the instruction logic.
    pub arbiter: AccountInfo<'info>,
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = initializer_deposit_token_account.amount > 0,
        constraint = initializer_deposit_token_account.owner == initializer.key()
    )]
    pub initializer_deposit_token_account: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = initializer,
        space = 8 + Escrow::LEN,
        seeds = [b"escrow", initializer.key().as_ref(), recipient.key().as_ref()],
        bump
    )]
    pub escrow_state: Account<'info, Escrow>,
    #[account(
        init,
        payer = initializer,
        seeds = [b"vault", escrow_state.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault
    )]
    pub vault: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    #[account(mut)]
    pub recipient_deposit_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = escrow_state.recipient == recipient.key() @ EscrowError::InvalidRecipient,
        seeds = [b"escrow", escrow_state.initializer.as_ref(), escrow_state.recipient.as_ref()],
        bump = escrow_state.escrow_bump,
    )]
    pub escrow_state: Account<'info, Escrow>,
    #[account(
        mut,
        seeds = [b"vault", escrow_state.key().as_ref()],
        bump = escrow_state.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,
    #[account(mut)]
    pub initializer_refund_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = escrow_state.initializer == initializer.key() @ EscrowError::InvalidInitializer,
        seeds = [b"escrow", escrow_state.initializer.as_ref(), escrow_state.recipient.as_ref()],
        bump = escrow_state.escrow_bump,
    )]
    pub escrow_state: Account<'info, Escrow>,
    #[account(
        mut,
        seeds = [b"vault", escrow_state.key().as_ref()],
        bump = escrow_state.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[account]
#[derive(Default)]
pub struct Escrow {
    pub initializer: Pubkey,
    pub recipient: Pubkey,
    pub arbiter: Pubkey,
    pub amount: u64,
    pub timeout: i64,
    pub status: EscrowStatus,
    pub vault_bump: u8,
    pub escrow_bump: u8,
}

impl Escrow {
    pub const LEN: usize = 32 + 32 + 32 + 8 + 8 + 1 + 1 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum EscrowStatus {
    Initialized,
    Withdrawn,
    Refunded,
    Cancelled,
}

impl Default for EscrowStatus {
    fn default() -> Self {
        Self::Initialized
    }
}

#[error_code]
pub enum EscrowError {
    #[msg("The amount must be greater than zero.")]
    InvalidAmount,
    #[msg("The recipient is not valid for this escrow.")]
    InvalidRecipient,
    #[msg("The initializer is not valid for this escrow.")]
    InvalidInitializer,
    #[msg("The arbiter is not valid for this escrow.")]
    InvalidArbiter,
    #[msg("The timeout has expired, withdrawal is no longer possible.")]
    TimeoutExpired,
    #[msg("The timeout has not yet expired, refund is not allowed.")]
    RefundNotAllowed,
    #[msg("The escrow cannot be cancelled, timeout has been reached.")]
    CancelNotAllowed,
    #[msg("The escrow is not in the correct state for this action.")]
    InvalidState,
    #[msg("Overflow when calculating timeout.")]
    Overflow,
    #[msg("Invalid bump seed.")]
    InvalidBump,
}

#[event]
pub struct EscrowInitialized {
    pub escrow: Pubkey,
    pub initializer: Pubkey,
    pub recipient: Pubkey,
    pub arbiter: Pubkey,
    pub amount: u64,
}

#[event]
pub struct EscrowWithdrawn {
    pub escrow: Pubkey,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct EscrowRefunded {
    pub escrow: Pubkey,
    pub initializer: Pubkey,
    pub amount: u64,
}

#[event]
pub struct EscrowCancelled {
    pub escrow: Pubkey,
    pub initializer: Pubkey,
}

#[event]
pub struct EscrowResolved {
    pub escrow: Pubkey,
    pub arbiter: Pubkey,
    pub release_to_recipient: bool,
}
