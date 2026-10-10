//! `init_claim` — once per player, hub-wide (02 §3.2). Plain `init` → re-init fails → R1 by construction.
use {
    crate::{constants::*, events::*, state::*},
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitClaim<'info> {
    pub player: Signer<'info>,
    /// Relayer (social login) or the player — the only source of the claim's rent (R2).
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init,
        payer = payer,
        space = 8 + PlayerClaim::INIT_SPACE,
        seeds = [CLAIM_SEED, player.key().as_ref()],
        bump,
    )]
    pub claim: Account<'info, PlayerClaim>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_claim(ctx: Context<InitClaim>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let c = &mut ctx.accounts.claim;
    c.version = ACCOUNT_VERSION;
    c.bump = ctx.bumps.claim;
    c.rent_payer = ctx.accounts.payer.key(); // R1 + R2: written once, from the payer signer only
    c.player = ctx.accounts.player.key();
    c.last_joined_at = now;
    c.current = None;
    c.kick = None;
    emit!(ClaimCreated { player: c.player, rent_payer: c.rent_payer, at: now });
    Ok(())
}
