//! P3 — close_room / close_claim: anyone, refund pinned to rent_payer (L13 · R3 · 02 §6 ix 15–16).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    common::*,
    solana_signer::Signer,
};

const HAS_ONE: u32 = 2001; // Anchor ConstraintHasOne

#[test]
fn close_room_after_retention_refunds_rent_payer_anyone_can_call() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let host = player(&mut svm);
    let relayer = funded(&mut svm);
    let id = next_room_id(&svm, &game);
    { let ix = ix_create_room(&svm, &game, &host.pubkey(), &relayer.pubkey(), id); ok(send(&mut svm, &[ix], &[&host, &relayer])); }
    assert_err(&crank(&mut svm, ix_close_room(&game, id, &relayer.pubkey())), code(E::WrongPhase));
    ok(send(&mut svm, &[ix_leave_room(&game, id, &host.pubkey(), false)], &[&host])); // → Cancelled
    advance(&mut svm, 599);
    assert_err(&crank(&mut svm, ix_close_room(&game, id, &relayer.pubkey())), code(E::RetentionNotReached));
    advance(&mut svm, 1);
    // R3: a caller cannot redirect the refund
    let thief = funded(&mut svm);
    assert_err(&send(&mut svm, &[ix_close_room(&game, id, &thief.pubkey())], &[&thief]), HAS_ONE);
    let room_lamports = lamports(&svm, &room_pda(&game, id));
    let before = lamports(&svm, &relayer.pubkey());
    ok(crank(&mut svm, ix_close_room(&game, id, &relayer.pubkey())));
    assert_eq!(lamports(&svm, &relayer.pubkey()) - before, room_lamports);
    assert!(gone(&svm, &room_pda(&game, id)));
    // a claim pointing at a closed room is free
    create_room(&mut svm, &game, &host);
}

#[test]
fn close_claim_only_after_claim_idle_then_refunds_rent_payer() {
    let (mut svm, _g) = game_world();
    let relayer = funded(&mut svm);
    let p = funded(&mut svm);
    ok(send(&mut svm, &[ix_init_claim(&p.pubkey(), &relayer.pubkey())], &[&p, &relayer]));
    // L13 (c): nobody — not even the owner — before CLAIM_IDLE_SECS since last_joined_at
    advance(&mut svm, CLAIM_IDLE_SECS - 1);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &relayer.pubkey()); assert_err(&send(&mut svm, &[ix], &[&p]), code(E::ClaimNotIdle)); }
    advance(&mut svm, 1);
    // R3: wrong rent_payer
    let thief = funded(&mut svm);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &thief.pubkey()); assert_err(&send(&mut svm, &[ix], &[&thief]), HAS_ONE); }
    // never joined (current = None, prev = placeholder) → closable by anyone once idle
    let claim_lamports = lamports(&svm, &claim_pda(&p.pubkey()));
    let before = lamports(&svm, &relayer.pubkey());
    { let ix = ix_close_claim(&svm, &p.pubkey(), &relayer.pubkey()); ok(crank(&mut svm, ix)); }
    assert_eq!(lamports(&svm, &relayer.pubkey()) - before, claim_lamports);
    assert!(gone(&svm, &claim_pda(&p.pubkey())));
    // re-init works (new rent_payer = whoever pays now)
    ok(send(&mut svm, &[ix_init_claim(&p.pubkey(), &p.pubkey())], &[&p]));
    let c: PlayerClaim = read(&svm, &claim_pda(&p.pubkey()));
    assert_eq!(c.rent_payer, p.pubkey());
}

#[test]
fn joining_resets_the_idle_clock() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let p = player(&mut svm);
    advance(&mut svm, CLAIM_IDLE_SECS - 10);
    let id = create_room(&mut svm, &game, &p); // last_joined_at = now
    ok(send(&mut svm, &[ix_leave_room(&game, id, &p.pubkey(), false)], &[&p]));
    advance(&mut svm, 20); // 30d since init, but only 20s since the join
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); assert_err(&crank(&mut svm, ix), code(E::ClaimNotIdle)); }
    advance(&mut svm, CLAIM_IDLE_SECS);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); ok(crank(&mut svm, ix)); }
}

#[test]
fn idle_but_still_seated_in_an_unticked_room_is_live() {
    // nobody ticked the room → it is still Waiting after 30 days → claim is live
    let (mut svm, g) = game_world();
    let game = g.program;
    let p = player(&mut svm);
    create_room(&mut svm, &game, &p);
    advance(&mut svm, CLAIM_IDLE_SECS);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); assert_err(&crank(&mut svm, ix), code(E::ClaimLive)); }
}

#[test]
fn close_claim_blocked_during_kick_cooldown() {
    // seated 30+ days in an unticked Waiting room, then kicked → idle, dead, but the kick mark is active
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &host);
    join(&mut svm, &game, id, &b);
    advance(&mut svm, CLAIM_IDLE_SECS);
    ok(send(&mut svm, &[ix_kick(&game, id, &host.pubkey(), 1, &b.pubkey())], &[&host]));
    { let ix = ix_close_claim(&svm, &b.pubkey(), &b.pubkey()); assert_err(&crank(&mut svm, ix), code(E::KickCooldown)); }
    advance(&mut svm, KICK_COOLDOWN_SECS);
    { let ix = ix_close_claim(&svm, &b.pubkey(), &b.pubkey()); ok(crank(&mut svm, ix)); }
}

#[test]
fn join_room_also_resets_the_idle_clock() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, p) = (player(&mut svm), player(&mut svm));
    advance(&mut svm, CLAIM_IDLE_SECS - 10);
    let id = create_room(&mut svm, &game, &host);
    join(&mut svm, &game, id, &p);
    assert_eq!(read::<PlayerClaim>(&svm, &claim_pda(&p.pubkey())).last_joined_at, now(&svm));
    ok(send(&mut svm, &[ix_leave_room(&game, id, &p.pubkey(), false)], &[&p]));
    advance(&mut svm, 20);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); assert_err(&crank(&mut svm, ix), code(E::ClaimNotIdle)); }
}

#[test]
fn close_claim_with_forged_prev_room_is_rejected() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (a, b) = (player(&mut svm), player(&mut svm));
    let ra = create_room(&mut svm, &game, &a);
    let rb = create_room(&mut svm, &game, &b);
    advance(&mut svm, CLAIM_IDLE_SECS);
    // claim points at ra; pass rb (a terminal-looking or other room) to fake "dead"
    let mut ix = ix_close_claim(&svm, &a.pubkey(), &a.pubkey());
    ix.accounts[2].pubkey = room_pda(&game, rb);
    assert_err(&crank(&mut svm, ix), code(E::PrevRoomMismatch));
    let _ = ra;
}
