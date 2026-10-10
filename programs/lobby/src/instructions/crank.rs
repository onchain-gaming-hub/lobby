//! Permissionless: `tick` (every deadline · 02 §4) · `close_room` / `close_claim` (refund pinned to
//! `rent_payer` — R3). None of these read `remaining_accounts` (relayer crank quota signer · SKILL §3.7).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, state::*},
    anchor_lang::prelude::*,
};

#[derive(Accounts)]
pub struct Tick<'info> {
    #[account(mut, seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()], bump = room.bump)]
    pub room: Box<Account<'info, Room>>,
    /// Required when a public room's transition touches the pointer · placeholder otherwise.
    #[account(mut, seeds = [MATCHMAKER_SEED, room.game_program.as_ref()], bump = matchmaker.bump)]
    pub matchmaker: Option<Account<'info, Matchmaker>>,
}

/// Applies the first due deadline. Nothing due → success with no change (idempotent).
pub fn handle_tick(ctx: Context<Tick>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    let room = &mut a.room;
    let mm = a.matchmaker.as_deref_mut();
    let due = |anchor: i64, ttl: u32| now >= anchor.saturating_add(ttl as i64);
    match room.phase {
        Phase::Waiting => {
            if due(room.last_human_activity_at, room.ttl.lobby) {
                // checked first: a ghost room should not get bots
                cancel_room(room, mm, EndReason::LobbyExpired, now)?;
            } else if room.is_public && room.bot_fill && room.humans() >= room.min_humans as usize && due(room.phase_since, room.ttl.wait) {
                let mm = mm.ok_or(LobbyError::MatchmakerRequired)?;
                for i in 0..room.seats.len() {
                    if room.seats[i].kind == SeatKind::Empty {
                        room.seats[i] = Seat { kind: SeatKind::Bot, ..Seat::EMPTY };
                        emit_seat(room, i, SeatCause::Backfill, now);
                    }
                }
                clear_pointer_if(mm, room.game_program, room.room_id, now);
                enter_ready_check(room, None, now);
            }
        }
        Phase::ReadyCheck => {
            if due(room.phase_since, room.ttl.ready) {
                for i in 0..room.seats.len() {
                    let s = room.seats[i];
                    if s.is_active_human() && !s.ready {
                        emit_seat(room, i, SeatCause::ReadyTimeout, now);
                        room.seats[i] = Seat::EMPTY; // claim dies with the seat
                    } else if room.is_public && s.kind == SeatKind::Bot {
                        room.seats[i] = Seat::EMPTY; // public Waiting is human-only (invariant 5)
                        emit_seat(room, i, SeatCause::Purge, now);
                    }
                }
                if room.humans() == 0 {
                    cancel_room(room, mm, EndReason::NoHumans, now)?;
                } else {
                    set_phase(room, Phase::Waiting, EndReason::None, now);
                    room.last_human_activity_at = now;
                    if room.is_public {
                        let mm = mm.ok_or(LobbyError::MatchmakerRequired)?;
                        if mm.open_public_room.is_none() {
                            mm.open_public_room = Some(room.room_id); // re-open · else orphan (accepted)
                            emit!(MatchmakerPointer { game_program: room.game_program, open_public_room: mm.open_public_room, at: now });
                        }
                    } else if room.host.is_none_or(|h| room.seat_of(&h).is_none()) {
                        reassign_host(room, now);
                    }
                }
            }
        }
        Phase::Starting => {
            if due(room.phase_since, room.ttl.start) {
                set_phase(room, Phase::Cancelled, EndReason::StartExpired, now);
            }
        }
        Phase::Playing => {
            if due(room.phase_since, room.ttl.max_game) {
                set_phase(room, Phase::Cancelled, EndReason::GameExpired, now);
            }
        }
        Phase::Ended | Phase::Cancelled => {}
    }
    Ok(())
}

#[derive(Accounts)]
pub struct CloseRoom<'info> {
    #[account(
        mut,
        close = rent_payer,
        has_one = rent_payer,
        seeds = [ROOM_SEED, room.game_program.as_ref(), &room.room_id.to_le_bytes()],
        bump = room.bump,
    )]
    pub room: Box<Account<'info, Room>>,
    /// CHECK: pinned by `has_one` (R3) — need not sign.
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
}

pub fn handle_close_room(ctx: Context<CloseRoom>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let room = &ctx.accounts.room;
    require!(room.phase.is_terminal(), LobbyError::WrongPhase);
    require!(now >= room.phase_since.saturating_add(room.ttl.retention as i64), LobbyError::RetentionNotReached);
    emit!(RoomClosed {
        game_program: room.game_program,
        room_id: room.room_id,
        refunded_to: room.rent_payer,
        lamports: room.to_account_info().lamports(),
        at: now,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseClaim<'info> {
    #[account(
        mut,
        close = rent_payer,
        has_one = rent_payer,
        seeds = [CLAIM_SEED, claim.player.as_ref()],
        bump = claim.bump,
    )]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: pinned by `has_one` (R3) — need not sign.
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
    /// CHECK: the room `claim.current` points at (verified in `claim_is_live`) · placeholder when None.
    pub prev_room: Option<UncheckedAccount<'info>>,
}

/// Anyone (maintainer 2026-10-10 · L13). Lobby Worker only does this after CLAIM_IDLE (04 §3) — policy, not a rule.
pub fn handle_close_claim(ctx: Context<CloseClaim>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require!(!claim_is_live(&a.claim, prev.as_ref())?, LobbyError::ClaimLive);
    // close + re-init must not wipe an active kick mark (round-1 review B1)
    require!(a.claim.kick.is_none_or(|k| now >= k.until), LobbyError::KickCooldown);
    emit!(ClaimClosed {
        player: a.claim.player,
        refunded_to: a.claim.rent_payer,
        lamports: a.claim.to_account_info().lamports(),
        at: now,
    });
    Ok(())
}
