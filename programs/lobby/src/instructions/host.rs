//! Host controls — private rooms, Waiting only (02 §6 ix 9–12 · lobby-state §8B).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, state::*},
    anchor_lang::prelude::*,
};

fn require_host(room: &Room, host: &Pubkey) -> Result<()> {
    require!(!room.is_public, LobbyError::PublicNoHostControl);
    require!(room.host == Some(*host), LobbyError::NotHost);
    require!(room.phase == Phase::Waiting, LobbyError::NotWaiting);
    Ok(())
}

fn seat_index(room: &Room, seat: u8) -> Result<usize> {
    let i = seat as usize;
    require!(i < room.seats.len(), LobbyError::InvalidSeat);
    Ok(i)
}

#[derive(Accounts)]
pub struct HostRoom<'info> {
    pub host: Signer<'info>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
}

pub fn handle_add_bot(ctx: Context<HostRoom>, seat: u8) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &mut ctx.accounts.room;
    require_host(room, &ctx.accounts.host.key())?;
    require!(room.bot_fill, LobbyError::BotsDisabled);
    let i = seat_index(room, seat)?;
    require!(room.seats[i].kind == SeatKind::Empty, LobbyError::SeatNotEmpty);
    room.seats[i] = Seat { kind: SeatKind::Bot, ..Seat::EMPTY };
    room.last_human_activity_at = now;
    emit_seat(room, i, Pubkey::default(), SeatCause::AddBot, now);
    Ok(())
}

pub fn handle_remove_bot(ctx: Context<HostRoom>, seat: u8) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &mut ctx.accounts.room;
    require_host(room, &ctx.accounts.host.key())?;
    let i = seat_index(room, seat)?;
    require!(room.seats[i].kind == SeatKind::Bot, LobbyError::SeatNotBot);
    room.seats[i] = Seat::EMPTY;
    room.last_human_activity_at = now;
    emit_seat(room, i, Pubkey::default(), SeatCause::RemoveBot, now);
    Ok(())
}

pub fn handle_request_start(ctx: Context<HostRoom>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let host = ctx.accounts.host.key();
    let room = &mut ctx.accounts.room;
    require_host(room, &host)?;
    require!(room.occupied() >= room.min_seats as usize, LobbyError::NotEnoughSeats);
    require!(room.humans() >= room.min_humans as usize, LobbyError::NotEnoughHumans);
    let hi = room.seat_of(&host).ok_or(LobbyError::NotInRoom)?;
    room.last_human_activity_at = now;
    enter_ready_check(room, Some(hi), now);
    emit!(PlayerReady { game_program: room.game_program, room_id: room.room_id, player: host, at: now }); // host auto-ready
    start_if_all_ready(room, now); // solo host + bots → Starting right away (no ReadyCheck loop)
    Ok(())
}

#[derive(Accounts)]
pub struct KickPlayer<'info> {
    pub host: Signer<'info>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    /// The kicked player's claim — carries the L1 cooldown (01 §4).
    #[account(mut, seeds = [CLAIM_SEED, victim_claim.player.as_ref()], bump = victim_claim.bump)]
    pub victim_claim: Box<Account<'info, PlayerClaim>>,
}

pub fn handle_kick_player(ctx: Context<KickPlayer>, seat: u8) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let host = ctx.accounts.host.key();
    let room = &mut ctx.accounts.room;
    require_host(room, &host)?;
    let i = seat_index(room, seat)?;
    let s = room.seats[i];
    require!(s.kind == SeatKind::Human, LobbyError::NotAHuman);
    require!(s.player != host, LobbyError::CannotKickSelf);
    let victim = &mut ctx.accounts.victim_claim;
    require_keys_eq!(victim.player, s.player, LobbyError::InvalidSeat);
    room.seats[i] = Seat::EMPTY;
    emit_seat(room, i, s.player, SeatCause::Kick, now);
    room.last_human_activity_at = now;
    // Single slot: a later kick elsewhere overwrites it (accepted — L1 is QoL, 01 §4).
    victim.kick = Some(KickMark { room: room.room_ref(), until: now + KICK_COOLDOWN_SECS });
    Ok(())
}
