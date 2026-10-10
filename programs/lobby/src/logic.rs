//! Pure rules shared by instructions (02). No account I/O except `claim_is_live`.
use {
    crate::{constants::*, errors::LobbyError, events::*, state::*},
    anchor_lang::prelude::*,
};

/// Config the game owner sets (`init_lobby` / `update_config`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConfigParams {
    pub max_seats: u8,
    pub min_seats: u8,
    pub min_humans: u8,
    pub public_enabled: bool,
    pub private_enabled: bool,
    pub bot_fill: bool,
    pub rematch_enabled: bool,
    pub ttl: RoomTtl,
}

/// 02 §8 `InvalidConfig` + B2 program caps.
pub fn validate_config(p: &ConfigParams) -> Result<()> {
    let t = &p.ttl;
    let ok = (2..=MAX_SEATS).contains(&p.max_seats)
        && 1 <= p.min_humans
        && p.min_humans <= p.min_seats
        && p.min_seats <= p.max_seats
        && [t.wait, t.ready, t.lobby, t.start, t.max_game, t.rematch_window, t.retention].iter().all(|x| *x > 0)
        && t.retention >= t.rematch_window
        && t.ready <= MAX_READY_TTL
        && t.start <= MAX_START_TTL
        && t.lobby <= MAX_LOBBY_TTL
        && t.max_game <= CAP_MAX_GAME_TTL
        && t.retention <= MAX_RETENTION;
    require!(ok, LobbyError::InvalidConfig);
    Ok(())
}

pub fn apply_config(c: &mut LobbyConfig, p: &ConfigParams) {
    c.max_seats = p.max_seats;
    c.min_seats = p.min_seats;
    c.min_humans = p.min_humans;
    c.public_enabled = p.public_enabled;
    c.private_enabled = p.private_enabled;
    c.bot_fill = p.bot_fill;
    c.rematch_enabled = p.rematch_enabled;
    c.ttl = p.ttl;
}

pub fn room_pda(game_program: &Pubkey, room_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[ROOM_SEED, game_program.as_ref(), &room_id.to_le_bytes()], &crate::ID).0
}

// ─────────────────── claim liveness (02 §2) ───────────────────

/// A claim is live while the room it points at exists, is not terminal, and still seats the
/// player as an active human. Nothing ever releases a claim — liveness is derived.
/// `prev_room` = the room `claim.current` points at (read-only, may be closed already).
/// It may be the same account as another room in the ix: its data still holds the pre-ix state
/// because Anchor writes `Account<>` structs back only at the end of the instruction.
pub fn claim_is_live(claim: &PlayerClaim, prev_room: Option<&AccountInfo>) -> Result<bool> {
    let Some(r) = claim.current else { return Ok(false) };
    let prev = prev_room.ok_or(LobbyError::PrevRoomRequired)?;
    require_keys_eq!(prev.key(), room_pda(&r.game_program, r.room_id), LobbyError::PrevRoomMismatch);
    if prev.owner != &crate::ID || prev.data_is_empty() {
        return Ok(false); // closed
    }
    let room = Room::try_deserialize(&mut &prev.try_borrow_data()?[..])?;
    if room.phase.is_terminal() {
        return Ok(false);
    }
    Ok(room.seats.iter().any(|s| s.is_active_human() && s.player == claim.player))
}

pub fn require_claim_free(claim: &PlayerClaim, prev_room: Option<&AccountInfo>) -> Result<()> {
    require!(!claim_is_live(claim, prev_room)?, LobbyError::AlreadyInRoom);
    Ok(())
}

pub fn require_not_kicked(claim: &PlayerClaim, room: &RoomRef, now: i64) -> Result<()> {
    if let Some(k) = claim.kick {
        require!(!(k.room == *room && now < k.until), LobbyError::KickCooldown);
    }
    Ok(())
}

// ─────────────────── phase / seats ───────────────────

pub fn set_phase(room: &mut Room, to: Phase, reason: EndReason, now: i64) {
    let from = room.phase;
    room.phase = to;
    room.end_reason = reason;
    room.phase_since = now;
    emit!(PhaseChanged { game_program: room.game_program, room_id: room.room_id, from, to, end_reason: reason, at: now });
}

pub fn emit_seat(room: &Room, seat: usize, cause: SeatCause, now: i64) {
    let s = room.seats[seat];
    emit!(SeatChanged {
        game_program: room.game_program,
        room_id: room.room_id,
        seat: seat as u8,
        kind: s.kind,
        player: s.player,
        cause,
        at: now,
    });
}

/// Clear the matchmaker pointer only if it still points at this room (conditional — §5 lobby-state).
pub fn clear_pointer_if(mm: &mut Matchmaker, game_program: Pubkey, room_id: u64, now: i64) {
    if mm.open_public_room == Some(room_id) {
        mm.open_public_room = None;
        emit!(MatchmakerPointer { game_program, open_public_room: None, at: now });
    }
}

/// Cancel a room. Public rooms must pass their Matchmaker so the pointer stays consistent (invariant 7).
pub fn cancel_room(room: &mut Room, mm: Option<&mut Matchmaker>, reason: EndReason, now: i64) -> Result<()> {
    if room.is_public {
        let mm = mm.ok_or(LobbyError::MatchmakerRequired)?;
        clear_pointer_if(mm, room.game_program, room.room_id, now);
    }
    set_phase(room, Phase::Cancelled, reason, now);
    Ok(())
}

/// Enter ReadyCheck: everyone un-ready, then optionally pre-ready one seat (the host).
pub fn enter_ready_check(room: &mut Room, pre_ready: Option<usize>, now: i64) {
    for s in room.seats.iter_mut() {
        s.ready = false;
    }
    if let Some(i) = pre_ready {
        room.seats[i].ready = true;
    }
    set_phase(room, Phase::ReadyCheck, EndReason::None, now);
}

/// ReadyCheck → Starting when every active human is ready (lobby-state §8B ⭐ — evaluated in the same ix).
pub fn start_if_all_ready(room: &mut Room, now: i64) -> bool {
    if room.phase == Phase::ReadyCheck && room.seats.iter().filter(|s| s.is_active_human()).all(|s| s.ready) {
        set_phase(room, Phase::Starting, EndReason::None, now);
        return true;
    }
    false
}

/// Host left / was removed → lowest-seat human (skip bots). None if no human remains.
pub fn reassign_host(room: &mut Room, now: i64) {
    let next = room.seats.iter().find(|s| s.is_active_human()).map(|s| s.player);
    room.host = next;
    if let Some(h) = next {
        emit!(HostChanged { game_program: room.game_program, room_id: room.room_id, host: h, at: now });
    }
}

// ─────────────────── room creation (02 §3.3) ───────────────────

/// Create Room `matchmaker.next_room_id` by hand (System CPI) so a counter race returns the
/// retryable `StaleMatchmaker` instead of Anchor's ambiguous `ConstraintSeeds` (init runs first).
/// `payer` funds rent only (relayer SKILL v3 (P) 2) · pre-funded PDA → transfer the shortfall.
#[allow(clippy::too_many_arguments)]
pub fn create_room_account<'info>(
    new_room: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    mm: &mut Matchmaker,
    config: &LobbyConfig,
    is_public: bool,
    host: Option<Pubkey>,
    first_player: Pubkey,
    now: i64,
) -> Result<Room> {
    let game_program = config.game_program;
    let room_id = mm.next_room_id;
    let id_le = room_id.to_le_bytes();
    let (expected, bump) = Pubkey::find_program_address(&[ROOM_SEED, game_program.as_ref(), &id_le], &crate::ID);
    require_keys_eq!(new_room.key(), expected, LobbyError::StaleMatchmaker);
    require!(new_room.data_is_empty() && new_room.owner == &anchor_lang::system_program::ID, LobbyError::StaleMatchmaker);

    let space = Room::space(config.max_seats as usize);
    let rent = Rent::get()?.minimum_balance(space);
    let signer: &[&[&[u8]]] = &[&[ROOM_SEED, game_program.as_ref(), &id_le, &[bump]]];
    let have = new_room.lamports();
    if have == 0 {
        anchor_lang::system_program::create_account(
            CpiContext::new_with_signer(
                system_program.key(),
                anchor_lang::system_program::CreateAccount { from: payer.clone(), to: new_room.clone() },
                signer,
            ),
            rent,
            space as u64,
            &crate::ID,
        )?;
    } else {
        if have < rent {
            anchor_lang::system_program::transfer(
                CpiContext::new(system_program.key(), anchor_lang::system_program::Transfer { from: payer.clone(), to: new_room.clone() }),
                rent - have,
            )?;
        }
        anchor_lang::system_program::allocate(
            CpiContext::new_with_signer(system_program.key(), anchor_lang::system_program::Allocate { account_to_allocate: new_room.clone() }, signer),
            space as u64,
        )?;
        anchor_lang::system_program::assign(
            CpiContext::new_with_signer(system_program.key(), anchor_lang::system_program::Assign { account_to_assign: new_room.clone() }, signer),
            &crate::ID,
        )?;
    }

    let mut seats = vec![Seat::EMPTY; config.max_seats as usize];
    seats[0] = Seat { kind: SeatKind::Human, player: first_player, ready: false, left: false };
    let room = Room {
        version: ACCOUNT_VERSION,
        bump,
        rent_payer: payer.key(), // R2: from the payer signer only
        game_program,
        room_id,
        phase: Phase::Waiting,
        end_reason: EndReason::None,
        is_public,
        min_seats: config.min_seats,
        min_humans: config.min_humans,
        bot_fill: config.bot_fill,
        rematch_enabled: config.rematch_enabled,
        ttl: config.ttl,
        phase_since: now,
        last_human_activity_at: now,
        host,
        rematch_room_id: None,
        seats,
        reserved: [0; 32],
    };
    mm.next_room_id = room_id.checked_add(1).ok_or(LobbyError::MathOverflow)?;
    Ok(room)
}

/// Write a Room struct into an account we created by hand (Anchor won't do it for UncheckedAccount).
pub fn write_room(info: &AccountInfo, room: &Room) -> Result<()> {
    let mut data = info.try_borrow_mut_data()?;
    room.try_serialize(&mut &mut data[..])?;
    Ok(())
}

pub fn emit_room_created(room: &Room, rematch_of: Option<u64>, now: i64) {
    emit!(RoomCreated {
        game_program: room.game_program,
        room_id: room.room_id,
        is_public: room.is_public,
        host: room.host,
        rent_payer: room.rent_payer,
        rematch_of,
        at: now,
    });
}

/// Seat a player (claim already checked free) and point the claim at this room.
pub fn seat_player(room: &mut Room, claim: &mut PlayerClaim, now: i64) -> Result<usize> {
    let i = room.lowest_empty().ok_or(LobbyError::RoomFull)?;
    room.seats[i] = Seat { kind: SeatKind::Human, player: claim.player, ready: false, left: false };
    room.last_human_activity_at = now;
    claim.current = Some(room.room_ref());
    claim.last_joined_at = now;
    emit_seat(room, i, SeatCause::Join, now);
    Ok(i)
}
