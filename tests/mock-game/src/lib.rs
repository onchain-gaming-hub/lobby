//! TEST ONLY — never deploy. Plays the game side of L4: CPI into lobby signed by this
//! program's `[b"game_authority"]` PDA, exactly as a real game would (03 §3).
#![allow(unexpected_cfgs)]
#![allow(clippy::result_large_err)]

use anchor_lang::prelude::*;

declare_id!("5PjkqfpUic5nNYr5aN6nbGdHLSYQEEzK8wr7PDPv2aRw");

// Reads idls/lobby.json (copied from the build by scripts/test.sh).
declare_program!(lobby);

pub const GAME_AUTHORITY_SEED: &[u8] = b"game_authority";

#[program]
pub mod mock_game {
    use super::*;

    /// start_game: (a real game would read room.seats + open a pool here) → CPI lobby::begin_match.
    pub fn start_game(ctx: Context<Play>) -> Result<()> {
        let bump = ctx.bumps.game_authority;
        let seeds: &[&[&[u8]]] = &[&[GAME_AUTHORITY_SEED, &[bump]]];
        lobby::cpi::begin_match(CpiContext::new_with_signer(
            ctx.accounts.lobby_program.key(),
            lobby::cpi::accounts::BeginMatch {
                room: ctx.accounts.room.to_account_info(),
                game_entry: ctx.accounts.game_entry.to_account_info(),
                game_authority: ctx.accounts.game_authority.to_account_info(),
            },
            seeds,
        ))
    }

    /// finish_game(completed): → CPI lobby::end_match(Completed | Cancelled).
    pub fn finish_game(ctx: Context<Play>, completed: bool) -> Result<()> {
        let bump = ctx.bumps.game_authority;
        let seeds: &[&[&[u8]]] = &[&[GAME_AUTHORITY_SEED, &[bump]]];
        let result = if completed { lobby::types::MatchResult::Completed } else { lobby::types::MatchResult::Cancelled };
        lobby::cpi::end_match(
            CpiContext::new_with_signer(
                ctx.accounts.lobby_program.key(),
                lobby::cpi::accounts::EndMatch {
                    room: ctx.accounts.room.to_account_info(),
                    game_entry: ctx.accounts.game_entry.to_account_info(),
                    game_authority: ctx.accounts.game_authority.to_account_info(),
                },
                seeds,
            ),
            result,
        )
    }
}

#[derive(Accounts)]
pub struct Play<'info> {
    /// CHECK: lobby verifies it.
    #[account(mut)]
    pub room: UncheckedAccount<'info>,
    /// CHECK: lobby verifies it (platform-core GameEntry of this game).
    pub game_entry: UncheckedAccount<'info>,
    /// CHECK: this program's PDA signer.
    #[account(seeds = [GAME_AUTHORITY_SEED], bump)]
    pub game_authority: UncheckedAccount<'info>,
    pub lobby_program: Program<'info, lobby::program::Lobby>,
}
