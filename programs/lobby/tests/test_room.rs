//! P2 — private rooms: create / join / leave / host controls / ready (02 §2, §3, §6 · invariants 1, 2, 4, 6).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    anchor_lang::prelude::Pubkey,
    common::*,
    solana_signer::Signer,
};

fn world() -> (litesvm::LiteSVM, Pubkey) {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    setup_game(&mut svm, game, default_params());
    (svm, game)
}

#[test]
fn create_room_fields_and_payer_pays_exactly_room_rent() {
    let (mut svm, game) = world();
    let host = player(&mut svm);
    let relayer = funded(&mut svm);
    let before = lamports(&svm, &relayer.pubkey());
    ok({ let ix = ix_create_room(&svm, &game, &host.pubkey(), &relayer.pubkey(), 1); send(&mut svm, &[ix], &[&host, &relayer]) });
    // R4: relayer delta == rent of the Room it created (n = 4 → 351 B)
    assert_eq!(before - lamports(&svm, &relayer.pubkey()), svm.minimum_balance_for_rent_exemption(351));
    assert_eq!(svm.get_account(&room_pda(&game, 1)).unwrap().data.len(), 351);
    let r = room(&svm, &game, 1);
    assert_eq!((r.room_id, r.phase, r.is_public, r.host), (1, Phase::Waiting, false, Some(host.pubkey())));
    assert_eq!(r.rent_payer, relayer.pubkey()); // R2
    assert_eq!(r.seats.len(), 4);
    assert_eq!((r.seats[0].kind, r.seats[0].player), (SeatKind::Human, host.pubkey()));
    assert_eq!(r.ttl, default_params().ttl); // snapshot
    assert_eq!(next_room_id(&svm, &game), 2);
    let c: PlayerClaim = read(&svm, &claim_pda(&host.pubkey()));
    assert_eq!(c.current, Some(RoomRef { game_program: game, room_id: 1 }));
}

#[test]
fn stale_counter_returns_retryable_stale_matchmaker() {
    let (mut svm, game) = world();
    let a = player(&mut svm);
    let b = player(&mut svm);
    // both read next_room_id = 1; A lands first
    let ix_b = ix_create_room(&svm, &game, &b.pubkey(), &b.pubkey(), 1);
    create_room(&mut svm, &game, &a);
    let r = send(&mut svm, &[ix_b], &[&b]);
    assert_err(&r, code(E::StaleMatchmaker));
    // re-read and retry succeeds
    assert_eq!(create_room(&mut svm, &game, &b), 2);
}

#[test]
fn prefunded_room_pda_is_still_created_and_payer_covers_only_shortfall() {
    let (mut svm, game) = world();
    let host = player(&mut svm);
    let relayer = funded(&mut svm);
    svm.airdrop(&room_pda(&game, 1), 1_000_000).unwrap(); // griefer pre-funds the PDA
    let before = lamports(&svm, &relayer.pubkey());
    ok({ let ix = ix_create_room(&svm, &game, &host.pubkey(), &relayer.pubkey(), 1); send(&mut svm, &[ix], &[&host, &relayer]) });
    assert_eq!(before - lamports(&svm, &relayer.pubkey()), svm.minimum_balance_for_rent_exemption(351) - 1_000_000);
    assert_eq!(room(&svm, &game, 1).rent_payer, relayer.pubkey());
}

#[test]
fn suspended_game_or_private_disabled_cannot_create() {
    let (mut svm, game) = world();
    let host = player(&mut svm);
    let g: LobbyConfig = read(&svm, &config_pda(&game));
    let owner = funded(&mut svm);
    put_game_entry(&mut svm, &game, &owner.pubkey(), true);
    let r = { let ix = ix_create_room(&svm, &game, &host.pubkey(), &host.pubkey(), 1); send(&mut svm, &[ix], &[&host]) };
    assert_err(&r, code(E::GameNotActive));
    assert!(g.private_enabled);

    let game2 = Pubkey::new_unique();
    let mut p = default_params();
    p.private_enabled = false;
    setup_game(&mut svm, game2, p);
    let r = { let ix = ix_create_room(&svm, &game2, &host.pubkey(), &host.pubkey(), 1); send(&mut svm, &[ix], &[&host]) };
    assert_err(&r, code(E::ModeDisabled));
}

#[test]
fn one_player_one_room_hub_wide() {
    let (mut svm, game) = world();
    let other_game = Pubkey::new_unique();
    setup_game(&mut svm, other_game, default_params());
    let a = player(&mut svm);
    let b = player(&mut svm);
    let r1 = create_room(&mut svm, &game, &a);
    let r2 = create_room(&mut svm, &game, &b);
    // A is live in r1 → cannot create another room (any game) or join r2
    let r = { let ix = ix_create_room(&svm, &other_game, &a.pubkey(), &a.pubkey(), 1); send(&mut svm, &[ix], &[&a]) };
    assert_err(&r, code(E::AlreadyInRoom));
    let r = { let ix = ix_join_room(&svm, &game, r2, &a.pubkey()); send(&mut svm, &[ix], &[&a]) };
    assert_err(&r, code(E::AlreadyInRoom));
    // leave → claim dies without being touched → can join r2
    ok(send(&mut svm, &[ix_leave_room(&game, r1, &a.pubkey(), false)], &[&a]));
    join(&mut svm, &game, r2, &a);
    assert_eq!(room(&svm, &game, r1).phase, Phase::Cancelled); // A was the only human
}

#[test]
fn prev_room_must_match_the_claim() {
    let (mut svm, game) = world();
    let a = player(&mut svm);
    let b = player(&mut svm);
    let r1 = create_room(&mut svm, &game, &a);
    let r2 = create_room(&mut svm, &game, &b);
    let mut ix = ix_join_room(&svm, &game, r2, &a.pubkey());
    ix.accounts[4].pubkey = room_pda(&game, r2); // lie about prev_room
    assert_err(&send(&mut svm, &[ix], &[&a]), code(E::PrevRoomMismatch));
    let mut ix = ix_join_room(&svm, &game, r2, &a.pubkey());
    ix.accounts[4].pubkey = ::lobby::ID; // placeholder = None while claim points at r1
    assert_err(&send(&mut svm, &[ix], &[&a]), code(E::PrevRoomRequired));
    let _ = r1;
}

#[test]
fn lowest_empty_seat_and_no_repack() {
    let (mut svm, game) = world();
    let (h, b, c, d) = (player(&mut svm), player(&mut svm), player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    join(&mut svm, &game, id, &b);
    join(&mut svm, &game, id, &c);
    ok(send(&mut svm, &[ix_leave_room(&game, id, &b.pubkey(), false)], &[&b]));
    join(&mut svm, &game, id, &d);
    let r = room(&svm, &game, id);
    assert_eq!(r.seats[1].player, d.pubkey()); // took the hole
    assert_eq!(r.seats[2].player, c.pubkey()); // C never moved
}

#[test]
fn rejoin_same_room_passes_prev_room_equal_to_room() {
    let (mut svm, game) = world();
    let (h, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    join(&mut svm, &game, id, &b);
    ok(send(&mut svm, &[ix_leave_room(&game, id, &b.pubkey(), false)], &[&b]));
    assert_eq!(prev_of(&svm, &b.pubkey()), Some(room_pda(&game, id)));
    join(&mut svm, &game, id, &b); // prev_room == room in the same ix
    // and while seated, joining again is refused (prev data is pre-ix state)
    let r = { let ix = ix_join_room(&svm, &game, id, &b.pubkey()); send(&mut svm, &[ix], &[&b]) };
    assert_err(&r, code(E::AlreadyInRoom));
}

#[test]
fn kick_cooldown_same_room_only() {
    let (mut svm, game) = world();
    let (h, b, x) = (player(&mut svm), player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    let other = create_room(&mut svm, &game, &x);
    join(&mut svm, &game, id, &b);
    ok(send(&mut svm, &[ix_kick(&game, id, &h.pubkey(), 1, &b.pubkey())], &[&h]));
    assert_eq!(room(&svm, &game, id).seats[1].kind, SeatKind::Empty);
    // same room within 10s → refused
    let t0 = now(&svm);
    set_time(&mut svm, t0 + KICK_COOLDOWN_SECS - 1);
    let r = { let ix = ix_join_room(&svm, &game, id, &b.pubkey()); send(&mut svm, &[ix], &[&b]) };
    assert_err(&r, code(E::KickCooldown));
    // another room right away → fine (claim died when kicked)
    join(&mut svm, &game, other, &b);
    ok(send(&mut svm, &[ix_leave_room(&game, other, &b.pubkey(), false)], &[&b]));
    // after the cooldown → can return
    set_time(&mut svm, t0 + KICK_COOLDOWN_SECS);
    join(&mut svm, &game, id, &b);
}

#[test]
fn kick_rules() {
    let (mut svm, game) = world();
    let (h, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    join(&mut svm, &game, id, &b);
    ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 2)], &[&h]));
    assert_err(&send(&mut svm, &[ix_kick(&game, id, &h.pubkey(), 0, &h.pubkey())], &[&h]), code(E::CannotKickSelf));
    assert_err(&send(&mut svm, &[ix_kick(&game, id, &h.pubkey(), 2, &b.pubkey())], &[&h]), code(E::NotAHuman));
    assert_err(&send(&mut svm, &[ix_kick(&game, id, &b.pubkey(), 0, &h.pubkey())], &[&b]), code(E::NotHost));
    // victim claim must belong to the seat's player
    assert_err(&send(&mut svm, &[ix_kick(&game, id, &h.pubkey(), 1, &h.pubkey())], &[&h]), code(E::InvalidSeat));
}

#[test]
fn bots_rules() {
    let (mut svm, game) = world();
    let h = player(&mut svm);
    let id = create_room(&mut svm, &game, &h);
    assert_err(&send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 0)], &[&h]), code(E::SeatNotEmpty));
    assert_err(&send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 4)], &[&h]), code(E::InvalidSeat));
    assert_err(&send(&mut svm, &[ix_remove_bot(&game, id, &h.pubkey(), 1)], &[&h]), code(E::SeatNotBot));
    ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 1)], &[&h]));
    ok(send(&mut svm, &[ix_remove_bot(&game, id, &h.pubkey(), 1)], &[&h]));

    let g2 = Pubkey::new_unique();
    let mut p = default_params();
    p.bot_fill = false;
    setup_game(&mut svm, g2, p);
    let h2 = player(&mut svm);
    let id2 = create_room(&mut svm, &g2, &h2);
    assert_err(&send(&mut svm, &[ix_add_bot(&g2, id2, &h2.pubkey(), 1)], &[&h2]), code(E::BotsDisabled));
}

#[test]
fn host_leaves_reassigns_to_lowest_human_skipping_bots_last_human_cancels() {
    let (mut svm, game) = world();
    let (h, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 1)], &[&h]));
    join(&mut svm, &game, id, &b); // seat 2
    ok(send(&mut svm, &[ix_leave_room(&game, id, &h.pubkey(), false)], &[&h]));
    assert_eq!(room(&svm, &game, id).host, Some(b.pubkey()));
    ok(send(&mut svm, &[ix_leave_room(&game, id, &b.pubkey(), false)], &[&b]));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::NoHumans));
    // claims pointing at a cancelled room are free
    create_room(&mut svm, &game, &b);
}

#[test]
fn request_start_rules_ready_check_and_freeze() {
    let (mut svm, game) = world();
    let (h, b, c) = (player(&mut svm), player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    join(&mut svm, &game, id, &b);
    assert_err(&send(&mut svm, &[ix_request_start(&game, id, &h.pubkey())], &[&h]), code(E::NotEnoughSeats));
    assert_err(&send(&mut svm, &[ix_request_start(&game, id, &b.pubkey())], &[&b]), code(E::NotHost));
    ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 2)], &[&h]));
    ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 3)], &[&h]));
    ok(send(&mut svm, &[ix_request_start(&game, id, &h.pubkey())], &[&h]));
    let r = room(&svm, &game, id);
    assert_eq!(r.phase, Phase::ReadyCheck);
    assert!(r.seats[0].ready && !r.seats[1].ready);
    // freeze: no join / leave / host controls
    assert_err(&{ let ix = ix_join_room(&svm, &game, id, &c.pubkey()); send(&mut svm, &[ix], &[&c]) }, code(E::NotWaiting));
    assert_err(&send(&mut svm, &[ix_leave_room(&game, id, &b.pubkey(), false)], &[&b]), code(E::NotWaiting));
    assert_err(&send(&mut svm, &[ix_remove_bot(&game, id, &h.pubkey(), 2)], &[&h]), code(E::NotWaiting));
    // non-member ready → NotInRoom · last human ready → Starting
    assert_err(&send(&mut svm, &[ix_ready(&game, id, &c.pubkey())], &[&c]), code(E::NotInRoom));
    ok(send(&mut svm, &[ix_ready(&game, id, &b.pubkey())], &[&b]));
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting);
    // ready outside ReadyCheck → WrongPhase
    assert_err(&send(&mut svm, &[ix_ready(&game, id, &b.pubkey())], &[&b]), code(E::WrongPhase));
}

#[test]
fn solo_host_with_bots_goes_straight_to_starting() {
    let (mut svm, game) = world();
    let h = player(&mut svm);
    let id = create_room(&mut svm, &game, &h);
    for s in 1..4 {
        ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), s)], &[&h]));
    }
    ok(send(&mut svm, &[ix_request_start(&game, id, &h.pubkey())], &[&h]));
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting); // lobby-state §8B ⭐ — no ReadyCheck loop
}

#[test]
fn join_room_rejects_full_rooms() {
    let (mut svm, game) = world();
    let h = player(&mut svm);
    let id = create_room(&mut svm, &game, &h);
    for _ in 0..3 {
        let p = player(&mut svm);
        join(&mut svm, &game, id, &p);
    }
    let late = player(&mut svm);
    assert_err(&{ let ix = ix_join_room(&svm, &game, id, &late.pubkey()); send(&mut svm, &[ix], &[&late]) }, code(E::RoomFull));
}
