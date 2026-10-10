//! P4 — public rooms: quick_match_join / quick_match_create + public tick paths (02 §3.4, §4 · invariant 7).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    common::*,
    solana_signer::Signer,
};

#[test]
fn fill_flow_pointer_and_idle_clock() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let ps: Vec<_> = (0..4).map(|_| player(&mut svm)).collect();
    advance(&mut svm, 5);
    let id = qm(&mut svm, &game, &ps[0]);
    assert_eq!(pointer(&svm, &game), Some(id));
    let r = room(&svm, &game, id);
    assert!(r.is_public && r.host.is_none());
    for p in &ps[1..] {
        assert_eq!(qm(&mut svm, &game, p), id);
        assert_pointer_ok(&svm, &game);
    }
    for p in &ps {
        assert_eq!(read::<PlayerClaim>(&svm, &claim_pda(&p.pubkey())).last_joined_at, now(&svm)); // enter_room on both paths
    }
    let r = room(&svm, &game, id);
    assert_eq!(r.phase, Phase::ReadyCheck);
    assert_eq!(pointer(&svm, &game), None); // cleared in the same ix
    for p in &ps {
        ok(send(&mut svm, &[ix_ready(&game, id, &p.pubkey())], &[p]));
    }
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting);
    assert_pointer_ok(&svm, &game);
}

#[test]
fn concurrent_create_second_gets_retryable_stale_then_joins() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let ix_b = ix_qm_create(&svm, &game, &b.pubkey(), &b.pubkey(), 1);
    qm(&mut svm, &game, &a);
    assert_err(&send(&mut svm, &[ix_b], &[&b]), code(E::StaleMatchmaker));
    assert_eq!(qm(&mut svm, &game, &b), 1);
}

#[test]
fn join_with_stale_pointer_is_retryable() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let ps: Vec<_> = (0..5).map(|_| player(&mut svm)).collect();
    let id = qm(&mut svm, &game, &ps[0]);
    let late_ix = ix_qm_join(&svm, &game, id, &ps[4].pubkey()); // read pointer = id
    for p in &ps[1..4] {
        qm(&mut svm, &game, p);
    }
    assert_err(&send(&mut svm, &[late_ix], &[&ps[4]]), code(E::StaleMatchmaker));
}

#[test]
fn create_cooldown_applies_but_never_masks_stale_pointer() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = qm(&mut svm, &game, &a);
    ok(send(&mut svm, &[ix_leave_room(&game, id, &a.pubkey(), true)], &[&a])); // cancelled, pointer cleared
    assert_eq!(pointer(&svm, &game), None);
    // A on cooldown and the pointer is empty → CreateCooldown (FE shows a countdown)
    let r = { let ix = ix_qm_create(&svm, &game, &a.pubkey(), &a.pubkey(), next_room_id(&svm, &game)); send(&mut svm, &[ix], &[&a]) };
    assert_err(&r, code(E::CreateCooldown));
    // B opens a room; A (still on cooldown) racing a create must get the retryable StaleMatchmaker
    let stale = ix_qm_create(&svm, &game, &a.pubkey(), &a.pubkey(), next_room_id(&svm, &game));
    let b_room = qm(&mut svm, &game, &b);
    assert_err(&send(&mut svm, &[stale], &[&a]), code(E::StaleMatchmaker));
    assert_eq!(qm(&mut svm, &game, &a), b_room); // joining is not limited
}

#[test]
fn public_disabled_blocks_create_but_pointer_room_stays_joinable() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = qm(&mut svm, &game, &a);
    let mut p = default_params();
    p.public_enabled = false;
    ok(send(&mut svm, &[ix_update_config(&game, &g.owner.pubkey(), p)], &[&g.owner]));
    assert_eq!(qm(&mut svm, &game, &b), id);
    let c = player(&mut svm);
    // empty the pointer by filling → then create is refused
    let d = player(&mut svm);
    qm(&mut svm, &game, &c);
    qm(&mut svm, &game, &d);
    let e = player(&mut svm);
    let r = { let ix = ix_qm_create(&svm, &game, &e.pubkey(), &e.pubkey(), next_room_id(&svm, &game)); send(&mut svm, &[ix], &[&e]) };
    assert_err(&r, code(E::ModeDisabled));
}

#[test]
fn tick_backfill_then_ready_timeout_with_no_humans_cancels() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let a = player(&mut svm);
    let id = qm(&mut svm, &game, &a);
    advance(&mut svm, 59);
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Waiting);
    advance(&mut svm, 1);
    assert_err(&crank(&mut svm, ix_tick(&game, id, false)), code(E::MatchmakerRequired));
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    let r = room(&svm, &game, id);
    assert_eq!(r.phase, Phase::ReadyCheck);
    assert_eq!(r.seats.iter().filter(|s| s.kind == SeatKind::Bot).count(), 3);
    assert_eq!(pointer(&svm, &game), None);
    advance(&mut svm, 30); // A never readies
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::NoHumans));
    assert_pointer_ok(&svm, &game);
}

#[test]
fn ready_timeout_reverts_purges_bots_and_reopens_pointer() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = qm(&mut svm, &game, &a);
    qm(&mut svm, &game, &b);
    advance(&mut svm, 60);
    ok(crank(&mut svm, ix_tick(&game, id, true))); // backfill 2 bots → ReadyCheck
    ok(send(&mut svm, &[ix_ready(&game, id, &a.pubkey())], &[&a]));
    advance(&mut svm, 30);
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    let r = room(&svm, &game, id);
    assert_eq!(r.phase, Phase::Waiting);
    assert!(r.seats.iter().all(|s| s.kind != SeatKind::Bot)); // public Waiting is human-only
    assert_eq!(r.seat_of(&b.pubkey()), None);
    assert_eq!(pointer(&svm, &game), Some(id)); // re-opened
    assert_pointer_ok(&svm, &game);
}

#[test]
fn revert_while_another_room_holds_the_pointer_is_an_accepted_orphan() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b, c) = (player(&mut svm), player(&mut svm), player(&mut svm));
    let id = qm(&mut svm, &game, &a);
    qm(&mut svm, &game, &b);
    advance(&mut svm, 60);
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    ok(send(&mut svm, &[ix_ready(&game, id, &a.pubkey())], &[&a]));
    let other = qm(&mut svm, &game, &c); // pointer was empty → new room
    advance(&mut svm, 30);
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Waiting);
    assert_eq!(pointer(&svm, &game), Some(other)); // not overwritten
    assert_pointer_ok(&svm, &game);
}

#[test]
fn last_human_leaves_or_lobby_expires_clears_pointer() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = qm(&mut svm, &game, &a);
    assert_err(&send(&mut svm, &[ix_leave_room(&game, id, &a.pubkey(), false)], &[&a]), code(E::MatchmakerRequired));
    ok(send(&mut svm, &[ix_leave_room(&game, id, &a.pubkey(), true)], &[&a]));
    assert_eq!(pointer(&svm, &game), None);

    let mut p = default_params();
    p.bot_fill = false; // no backfill → the lobby TTL is what ends it
    ok(send(&mut svm, &[ix_update_config(&game, &g.owner.pubkey(), p)], &[&g.owner]));
    let id2 = qm(&mut svm, &game, &b);
    advance(&mut svm, 1800);
    ok(crank(&mut svm, ix_tick(&game, id2, true)));
    let r = room(&svm, &game, id2);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::LobbyExpired));
    assert_eq!(pointer(&svm, &game), None);
}

#[test]
fn no_backfill_below_min_humans_and_no_host_controls() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let mut p = default_params();
    p.min_humans = 2;
    p.min_seats = 4;
    ok(send(&mut svm, &[ix_update_config(&game, &g.owner.pubkey(), p)], &[&g.owner]));
    let a = player(&mut svm);
    let id = qm(&mut svm, &game, &a);
    advance(&mut svm, 60);
    ok(crank(&mut svm, ix_tick(&game, id, true)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Waiting); // 1 human < min_humans 2
    assert_err(&send(&mut svm, &[ix_add_bot(&game, id, &a.pubkey(), 1)], &[&a]), code(E::PublicNoHostControl));
}

#[test]
fn r4_quick_match_create_relayer_pays_exactly_room_rent() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let a = player(&mut svm);
    let relayer = funded(&mut svm);
    let before = lamports(&svm, &relayer.pubkey());
    { let ix = ix_qm_create(&svm, &game, &a.pubkey(), &relayer.pubkey(), 1); ok(send(&mut svm, &[ix], &[&a, &relayer])); }
    assert_eq!(before - lamports(&svm, &relayer.pubkey()), svm.minimum_balance_for_rent_exemption(351));
    assert_eq!(room(&svm, &game, 1).rent_payer, relayer.pubkey());
}

#[test]
fn counter_race_on_quick_match_create_is_stale_even_for_a_player_on_cooldown() {
    // pointer stays None, but a private create_room moved next_room_id → key check → StaleMatchmaker,
    // and it must win over CreateCooldown (mark_room_created runs after the race checks)
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &a); // A on cooldown now
    ok(send(&mut svm, &[ix_leave_room(&game, id, &a.pubkey(), false)], &[&a]));
    let stale = ix_qm_create(&svm, &game, &a.pubkey(), &a.pubkey(), next_room_id(&svm, &game));
    create_room(&mut svm, &game, &b); // moves the counter
    assert_eq!(pointer(&svm, &game), None);
    assert_err(&send(&mut svm, &[stale], &[&a]), code(E::StaleMatchmaker));
}

#[test]
fn create_room_counter_race_beats_cooldown() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &a);
    ok(send(&mut svm, &[ix_leave_room(&game, id, &a.pubkey(), false)], &[&a]));
    let stale = ix_create_room(&svm, &game, &a.pubkey(), &a.pubkey(), next_room_id(&svm, &game));
    create_room(&mut svm, &game, &b);
    assert_err(&send(&mut svm, &[stale], &[&a]), code(E::StaleMatchmaker));
}

#[test]
fn live_player_cannot_quick_match_into_a_second_room() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    create_room(&mut svm, &game, &a); // private, live
    let id = qm(&mut svm, &game, &b);
    let r = { let ix = ix_qm_join(&svm, &game, id, &a.pubkey()); send(&mut svm, &[ix], &[&a]) };
    assert_err(&r, code(E::AlreadyInRoom));
}
