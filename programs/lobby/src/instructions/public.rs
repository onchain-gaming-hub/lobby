//! Public rooms — quick match (02 §3.4 · invariant 7: the pointer always names a joinable room).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, platform_core::{accounts::GameEntry, types::GameStatus}, state::*},
    anchor_lang::prelude::*,
};

// ─────────────── quick_match_join (no payer) ───────────────

/// No `config`: `public_enabled` gates creation only — the room the pointer names stays joinable.
#[derive(Accounts)]
pub struct QuickMatchJoin<'info> {
    pub player: Signer<'info>,
    #[account(mut, seeds = [MATCHMAKER_SEED, room.game_program.as_ref()], bump = matchmaker.bump)]
    pub matchmaker: Box<Account<'info, Matchmaker>>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    #[account(mut, seeds = [CLAIM_SEED, player.key().as_ref()], bump = claim.bump)]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: the room `claim.current` points at (verified in `claim_is_live`) · placeholder when None.
    pub prev_room: Option<UncheckedAccount<'info>>,
}

pub fn handle_quick_match_join(ctx: Context<QuickMatchJoin>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    let room = &mut a.room;
    require!(a.matchmaker.open_public_room == Some(room.room_id), LobbyError::StaleMatchmaker);
    // pointer consistency guarantees these; checked anyway (defence in depth)
    require!(room.is_public, LobbyError::WrongRoom);
    require!(room.phase == Phase::Waiting, LobbyError::NotWaiting);
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require_claim_free(&a.claim, prev.as_ref())?;
    seat_player(room, &mut a.claim, now)?;
    if room.lowest_empty().is_none() {
        // full → ReadyCheck · clear the pointer in the same ix (invariant 7)
        clear_pointer_if(&mut a.matchmaker, room.game_program, room.room_id, now);
        enter_ready_check(room, None, now);
    }
    Ok(())
}

// ─────────────── quick_match_create (payer: creates Room) ───────────────

#[derive(Accounts)]
pub struct QuickMatchCreate<'info> {
    pub player: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        seeds = [PC_GAME_SEED, config.game_program.as_ref()],
        bump = game_entry.bump,
        seeds::program = crate::platform_core::ID,
        constraint = matches!(game_entry.status, GameStatus::Active) @ LobbyError::GameNotActive,
    )]
    pub game_entry: Box<Account<'info, GameEntry>>,
    #[account(seeds = [LOBBY_CONFIG_SEED, config.game_program.as_ref()], bump = config.bump)]
    pub config: Box<Account<'info, LobbyConfig>>,
    #[account(mut, seeds = [MATCHMAKER_SEED, config.game_program.as_ref()], bump = matchmaker.bump)]
    pub matchmaker: Box<Account<'info, Matchmaker>>,
    #[account(mut, seeds = [CLAIM_SEED, player.key().as_ref()], bump = claim.bump)]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: see QuickMatchJoin.
    pub prev_room: Option<UncheckedAccount<'info>>,
    /// CHECK: must be Room PDA `[room, game, matchmaker.next_room_id]` — created by hand (02 §3.3).
    #[account(mut)]
    pub new_room: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_quick_match_create(ctx: Context<QuickMatchCreate>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    // retryable race check first (a client that read an empty pointer must learn to join instead)
    require!(a.matchmaker.open_public_room.is_none(), LobbyError::StaleMatchmaker);
    require!(a.config.public_enabled, LobbyError::ModeDisabled);
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require_claim_free(&a.claim, prev.as_ref())?;
    let player = a.player.key();
    let new_room = a.new_room.to_account_info();
    let room = create_room_account(
        &new_room,
        &a.payer.to_account_info(),
        &a.system_program.to_account_info(),
        &mut a.matchmaker,
        &a.config,
        true,
        None,
        player,
        now,
    )?;
    mark_room_created(&mut a.claim, now)?;
    enter_room(&mut a.claim, &room.room_ref(), now);
    a.matchmaker.open_public_room = Some(room.room_id);
    write_room(&new_room, &room)?;
    emit_room_created(&room, None, now);
    emit_seat(&room, 0, player, SeatCause::Join, now);
    emit!(MatchmakerPointer { game_program: room.game_program, open_public_room: Some(room.room_id), at: now });
    Ok(())
}
