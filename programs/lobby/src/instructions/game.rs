//! Game CPI (L4 · 02 §6 ix 17–18) + `leave_seat` (ix 8). GameEntry is attached but Active is
//! NOT checked — a Suspended game must still be able to finish rooms already open (rev 3 #7).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, platform_core::accounts::GameEntry, state::*},
    anchor_lang::prelude::*,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchResult {
    Completed,
    Cancelled,
}

#[derive(Accounts)]
pub struct GameCpi<'info> {
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    #[account(
        seeds = [PC_GAME_SEED, room.game_program.as_ref()],
        bump = game_entry.bump,
        seeds::program = crate::platform_core::ID,
    )]
    pub game_entry: Box<Account<'info, GameEntry>>,
    /// The game's `[b"game_authority"]` PDA, signing via invoke_signed (platform-core 03 §2.2).
    #[account(address = game_entry.game_authority @ LobbyError::BadGameAuthority)]
    pub game_authority: Signer<'info>,
}

pub fn handle_begin_match(ctx: Context<GameCpi>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &mut ctx.accounts.room;
    require!(room.phase == Phase::Starting, LobbyError::WrongPhase);
    set_phase(room, Phase::Playing, EndReason::None, now);
    Ok(())
}

pub fn handle_end_match(ctx: Context<GameCpi>, result: MatchResult) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &mut ctx.accounts.room;
    // Starting/Playing rooms are never the matchmaker pointer (cleared on entering ReadyCheck).
    match (room.phase, result) {
        (Phase::Playing, MatchResult::Completed) => set_phase(room, Phase::Ended, EndReason::Completed, now),
        (Phase::Playing | Phase::Starting, MatchResult::Cancelled) => set_phase(room, Phase::Cancelled, EndReason::GameCancelled, now),
        _ => return err!(LobbyError::WrongPhase),
    }
    Ok(())
}

#[derive(Accounts)]
pub struct LeaveSeat<'info> {
    pub player: Signer<'info>,
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
}

/// Playing only: the seat stays (`left = true` — the game sees who dropped, D7) · the claim dies.
pub fn handle_leave_seat(ctx: Context<LeaveSeat>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &mut ctx.accounts.room;
    require!(room.phase == Phase::Playing, LobbyError::WrongPhase);
    let player = ctx.accounts.player.key();
    let i = room.seat_of(&player).ok_or(LobbyError::NotInRoom)?;
    require!(!room.seats[i].left, LobbyError::NotInRoom);
    room.seats[i].left = true;
    room.last_human_activity_at = now;
    emit!(PlayerLeftSeat { game_program: room.game_program, room_id: room.room_id, player, seat: i as u8, at: now });
    if room.humans() == 0 {
        set_phase(room, Phase::Cancelled, EndReason::Abandoned, now); // rev 3 #9 · R13
    }
    Ok(())
}
