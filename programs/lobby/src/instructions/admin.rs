//! `init_lobby` / `update_config` — signed by the game owner (`GameEntry.owner`), not relayed (02 §6 ix 1–2).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, platform_core::{accounts::GameEntry, types::GameStatus}, state::*},
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitLobby<'info> {
    /// Game owner — pays rent for config + matchmaker with their own wallet (not the relayer).
    #[account(mut, address = game_entry.owner @ LobbyError::NotGameOwner)]
    pub owner: Signer<'info>,
    /// CHECK: seed only — bound to game_entry by its seeds.
    pub game_program: UncheckedAccount<'info>,
    #[account(
        seeds = [PC_GAME_SEED, game_program.key().as_ref()],
        bump = game_entry.bump,
        seeds::program = crate::platform_core::ID,
        constraint = matches!(game_entry.status, GameStatus::Active) @ LobbyError::GameNotActive,
    )]
    pub game_entry: Box<Account<'info, GameEntry>>,
    #[account(
        init,
        payer = owner,
        space = 8 + LobbyConfig::INIT_SPACE,
        seeds = [LOBBY_CONFIG_SEED, game_program.key().as_ref()],
        bump,
    )]
    pub config: Box<Account<'info, LobbyConfig>>,
    #[account(
        init,
        payer = owner,
        space = 8 + Matchmaker::INIT_SPACE,
        seeds = [MATCHMAKER_SEED, game_program.key().as_ref()],
        bump,
    )]
    pub matchmaker: Box<Account<'info, Matchmaker>>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_lobby(ctx: Context<InitLobby>, params: ConfigParams) -> Result<()> {
    validate_config(&params)?;
    let game_program = ctx.accounts.game_program.key();
    let c = &mut ctx.accounts.config;
    c.version = ACCOUNT_VERSION;
    c.bump = ctx.bumps.config;
    c.game_program = game_program;
    apply_config(c, &params);
    let m = &mut ctx.accounts.matchmaker;
    m.version = ACCOUNT_VERSION;
    m.bump = ctx.bumps.matchmaker;
    m.next_room_id = 1;
    m.open_public_room = None;
    emit!(LobbyInitialized { game_program, owner: ctx.accounts.owner.key(), at: Clock::get()?.unix_timestamp });
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(address = game_entry.owner @ LobbyError::NotGameOwner)]
    pub owner: Signer<'info>,
    /// Not checked for Active — config changes only affect rooms created later (snapshot).
    #[account(
        seeds = [PC_GAME_SEED, config.game_program.as_ref()],
        bump = game_entry.bump,
        seeds::program = crate::platform_core::ID,
    )]
    pub game_entry: Box<Account<'info, GameEntry>>,
    #[account(mut, seeds = [LOBBY_CONFIG_SEED, config.game_program.as_ref()], bump = config.bump)]
    pub config: Box<Account<'info, LobbyConfig>>,
}

pub fn handle_update_config(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
    validate_config(&params)?;
    apply_config(&mut ctx.accounts.config, &params);
    emit!(LobbyConfigUpdated { game_program: ctx.accounts.config.game_program, at: Clock::get()?.unix_timestamp });
    Ok(())
}
