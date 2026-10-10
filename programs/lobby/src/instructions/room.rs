//! Private room entry + exit + ready (02 §3, §6 ix 3, 4, 6, 7).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, platform_core::{accounts::GameEntry, types::GameStatus}, state::*},
    anchor_lang::prelude::*,
};

fn now() -> Result<i64> {
    Ok(Clock::get()?.unix_timestamp)
}

// ─────────────── create_room (payer: creates Room) ───────────────

#[derive(Accounts)]
pub struct CreateRoom<'info> {
    pub player: Signer<'info>,
    /// Relayer or the player — funds the Room rent only (R2 · SKILL v3 (P)).
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
    /// CHECK: the room `claim.current` points at (verified in `claim_is_live`) · placeholder when None.
    pub prev_room: Option<UncheckedAccount<'info>>,
    /// CHECK: must be Room PDA `[room, game, matchmaker.next_room_id]` — created by hand (02 §3.3).
    #[account(mut)]
    pub new_room: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_room(ctx: Context<CreateRoom>) -> Result<()> {
    let now = now()?;
    let a = ctx.accounts;
    require!(a.config.private_enabled, LobbyError::ModeDisabled);
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
        false,
        Some(player),
        player,
        now,
    )?;
    a.claim.current = Some(room.room_ref());
    a.claim.last_joined_at = now;
    write_room(&new_room, &room)?;
    emit_room_created(&room, None, now);
    emit_seat(&room, 0, SeatCause::Join, now);
    Ok(())
}

// ─────────────── join_room (no payer) ───────────────

#[derive(Accounts)]
pub struct JoinRoom<'info> {
    pub player: Signer<'info>,
    #[account(seeds = [LOBBY_CONFIG_SEED, room.game_program.as_ref()], bump = config.bump)]
    pub config: Box<Account<'info, LobbyConfig>>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    #[account(mut, seeds = [CLAIM_SEED, player.key().as_ref()], bump = claim.bump)]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: see CreateRoom · may be this same room (rejoin) — still pass it (02 §2).
    pub prev_room: Option<UncheckedAccount<'info>>,
}

pub fn handle_join_room(ctx: Context<JoinRoom>) -> Result<()> {
    let now = now()?;
    let a = ctx.accounts;
    require!(a.config.private_enabled && !a.room.is_public, LobbyError::ModeDisabled);
    require!(a.room.phase == Phase::Waiting, LobbyError::NotWaiting);
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require_claim_free(&a.claim, prev.as_ref())?; // reads prev data before we touch seats
    require_not_kicked(&a.claim, &a.room.room_ref(), now)?;
    seat_player(&mut a.room, &mut a.claim, now)?;
    Ok(())
}

// ─────────────── leave_room (Waiting only) ───────────────

#[derive(Accounts)]
pub struct LeaveRoom<'info> {
    pub player: Signer<'info>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    /// Required for public rooms (pointer) · placeholder for private.
    #[account(mut, seeds = [MATCHMAKER_SEED, room.game_program.as_ref()], bump = matchmaker.bump)]
    pub matchmaker: Option<Account<'info, Matchmaker>>,
}

pub fn handle_leave_room(ctx: Context<LeaveRoom>) -> Result<()> {
    let now = now()?;
    let a = ctx.accounts;
    let room = &mut a.room;
    require!(room.phase == Phase::Waiting, LobbyError::NotWaiting);
    let player = a.player.key();
    let i = room.seat_of(&player).ok_or(LobbyError::NotInRoom)?;
    room.seats[i] = Seat::EMPTY;
    room.last_human_activity_at = now;
    emit_seat(room, i, SeatCause::Leave, now);
    if room.host == Some(player) {
        reassign_host(room, now);
    }
    if room.humans() == 0 {
        cancel_room(room, a.matchmaker.as_deref_mut(), EndReason::NoHumans, now)?;
    }
    Ok(())
}

// ─────────────── ready (ReadyCheck) ───────────────

#[derive(Accounts)]
pub struct Ready<'info> {
    pub player: Signer<'info>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
}

pub fn handle_ready(ctx: Context<Ready>) -> Result<()> {
    let now = now()?;
    let room = &mut ctx.accounts.room;
    require!(room.phase == Phase::ReadyCheck, LobbyError::WrongPhase);
    let player = ctx.accounts.player.key();
    let i = room.seat_of(&player).ok_or(LobbyError::NotInRoom)?;
    room.last_human_activity_at = now;
    if !room.seats[i].ready {
        room.seats[i].ready = true;
        emit!(PlayerReady { game_program: room.game_program, room_id: room.room_id, player, at: now });
    }
    start_if_all_ready(room, now);
    Ok(())
}
