//! Rematch — the old room serializes the group; create + pointer write in one tx (02 §5 · lobby-state §14).
use {
    crate::{constants::*, errors::LobbyError, events::*, logic::*, platform_core::{accounts::GameEntry, types::GameStatus}, state::*},
    anchor_lang::prelude::*,
};

/// Checks shared by rematch_join / rematch_create.
fn check_rematch(old: &Room, player: &Pubkey, now: i64) -> Result<()> {
    require!(old.phase == Phase::Ended, LobbyError::WrongPhase); // not Cancelled — the group never played
    require!(old.rematch_enabled, LobbyError::ModeDisabled);
    require!(now < old.phase_since.saturating_add(old.ttl.rematch_window as i64), LobbyError::RematchClosed);
    // anyone who sat as a human, including those who left mid-game
    require!(old.seats.iter().any(|s| s.kind == SeatKind::Human && s.player == *player), LobbyError::NotInRoster);
    Ok(())
}

/// Joinable rematch room: private, Waiting, has a free seat.
fn joinable(r: &Room) -> bool {
    !r.is_public && r.phase == Phase::Waiting && r.lowest_empty().is_some()
}

// ─────────────── rematch_join (no payer) ───────────────

#[derive(Accounts)]
pub struct RematchJoin<'info> {
    pub player: Signer<'info>,
    #[account(seeds = [ROOM_SEED, old_room.game_program.as_ref(), &old_room.room_id.to_le_bytes()], bump = old_room.bump)]
    pub old_room: Box<Account<'info, Room>>,
    #[account(mut, seeds = [CLAIM_SEED, player.key().as_ref()], bump = claim.bump)]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: the room `claim.current` points at — usually `old_room` itself (pass it anyway · 02 §2).
    pub prev_room: Option<UncheckedAccount<'info>>,
    #[account(mut, seeds = [ROOM_SEED, rematch_room.game_program.as_ref(), &rematch_room.room_id.to_le_bytes()], bump = rematch_room.bump)]
    pub rematch_room: Box<Account<'info, Room>>,
}

pub fn handle_rematch_join(ctx: Context<RematchJoin>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    let player = a.player.key();
    check_rematch(&a.old_room, &player, now)?;
    let r = &mut a.rematch_room;
    require!(
        r.game_program == a.old_room.game_program && a.old_room.rematch_room_id == Some(r.room_id),
        LobbyError::StaleRematch
    );
    require!(joinable(r), LobbyError::StaleRematch); // dead / full → client goes to rematch_create
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require_claim_free(&a.claim, prev.as_ref())?;
    require_not_kicked(&a.claim, &r.room_ref(), now)?; // joinable but kicked → wait
    seat_player(r, &mut a.claim, now)?;
    Ok(())
}

// ─────────────── rematch_create (payer: creates Room) ───────────────

#[derive(Accounts)]
pub struct RematchCreate<'info> {
    pub player: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        seeds = [PC_GAME_SEED, old_room.game_program.as_ref()],
        bump = game_entry.bump,
        seeds::program = crate::platform_core::ID,
        constraint = matches!(game_entry.status, GameStatus::Active) @ LobbyError::GameNotActive,
    )]
    pub game_entry: Box<Account<'info, GameEntry>>,
    #[account(seeds = [LOBBY_CONFIG_SEED, old_room.game_program.as_ref()], bump = config.bump)]
    pub config: Box<Account<'info, LobbyConfig>>,
    #[account(mut, seeds = [MATCHMAKER_SEED, old_room.game_program.as_ref()], bump = matchmaker.bump)]
    pub matchmaker: Box<Account<'info, Matchmaker>>,
    #[account(mut, seeds = [ROOM_SEED, old_room.game_program.as_ref(), &old_room.room_id.to_le_bytes()], bump = old_room.bump)]
    pub old_room: Box<Account<'info, Room>>,
    #[account(mut, seeds = [CLAIM_SEED, player.key().as_ref()], bump = claim.bump)]
    pub claim: Box<Account<'info, PlayerClaim>>,
    /// CHECK: see RematchJoin.
    pub prev_room: Option<UncheckedAccount<'info>>,
    /// CHECK: required when `old_room.rematch_room_id` is Some — proves the pointed room is stale
    /// (may be closed already, hence Unchecked). Key must be that room's PDA.
    pub rematch_room: Option<UncheckedAccount<'info>>,
    /// CHECK: must be Room PDA `[room, game, matchmaker.next_room_id]` — created by hand (02 §3.3).
    #[account(mut)]
    pub new_room: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_rematch_create(ctx: Context<RematchCreate>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let a = ctx.accounts;
    let player = a.player.key();
    check_rematch(&a.old_room, &player, now)?;
    // retryable race checks first: if a joinable rematch room exists, the client must join it
    if let Some(id) = a.old_room.rematch_room_id {
        let info = a.rematch_room.as_ref().ok_or(LobbyError::StaleRematch)?.to_account_info();
        // a proof for a room the pointer no longer names = stale read (concurrent create) → retryable
        require_keys_eq!(info.key(), room_pda(&a.old_room.game_program, id), LobbyError::StaleRematch);
        if info.owner == &crate::ID && !info.data_is_empty() {
            let r = Room::try_deserialize(&mut &info.try_borrow_data()?[..])?;
            if joinable(&r) {
                // still joinable: the group goes there — a kicked caller waits, never redirects it
                require_not_kicked(&a.claim, &r.room_ref(), now)?;
                return err!(LobbyError::StaleRematch);
            }
        } // closed → stale
    }
    let prev = a.prev_room.as_ref().map(|p| p.to_account_info());
    require_claim_free(&a.claim, prev.as_ref())?;
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
    record_room_created(&mut a.claim, now); // not cooled down (see record_room_created)
    enter_room(&mut a.claim, &room.room_ref(), now);
    a.old_room.rematch_room_id = Some(room.room_id);
    write_room(&new_room, &room)?;
    emit_room_created(&room, Some(a.old_room.room_id), now);
    emit_seat(&room, 0, player, SeatCause::Join, now);
    Ok(())
}
