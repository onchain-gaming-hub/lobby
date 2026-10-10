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
fn close_claim_rules_and_refund() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let relayer = funded(&mut svm);
    let p = funded(&mut svm);
    ok(send(&mut svm, &[ix_init_claim(&p.pubkey(), &relayer.pubkey())], &[&p, &relayer]));
    // never joined (current = None, prev = placeholder) → closable
    let claim_lamports = lamports(&svm, &claim_pda(&p.pubkey()));
    let before = lamports(&svm, &relayer.pubkey());
    { let ix = ix_close_claim(&svm, &p.pubkey(), &relayer.pubkey()); ok(crank(&mut svm, ix)); }
    assert_eq!(lamports(&svm, &relayer.pubkey()) - before, claim_lamports);
    assert!(gone(&svm, &claim_pda(&p.pubkey())));
    // re-init works (new rent_payer = whoever pays now)
    ok(send(&mut svm, &[ix_init_claim(&p.pubkey(), &p.pubkey())], &[&p]));
    let c: PlayerClaim = read(&svm, &claim_pda(&p.pubkey()));
    assert_eq!(c.rent_payer, p.pubkey());

    // live → ClaimLive
    let id = create_room(&mut svm, &game, &p);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); assert_err(&crank(&mut svm, ix), code(E::ClaimLive)); }
    // R3: wrong rent_payer
    let thief = funded(&mut svm);
    { let ix = ix_close_claim(&svm, &p.pubkey(), &thief.pubkey()); assert_err(&send(&mut svm, &[ix], &[&thief]), HAS_ONE); }
    ok(send(&mut svm, &[ix_leave_room(&game, id, &p.pubkey(), false)], &[&p]));
    { let ix = ix_close_claim(&svm, &p.pubkey(), &p.pubkey()); ok(crank(&mut svm, ix)); }
}

#[test]
fn close_claim_blocked_during_kick_cooldown() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &host);
    join(&mut svm, &game, id, &b);
    ok(send(&mut svm, &[ix_kick(&game, id, &host.pubkey(), 1, &b.pubkey())], &[&host]));
    // B's claim is dead (not seated) but the kick mark is active → cannot be wiped by close + re-init
    { let ix = ix_close_claim(&svm, &b.pubkey(), &b.pubkey()); assert_err(&crank(&mut svm, ix), code(E::KickCooldown)); }
    advance(&mut svm, KICK_COOLDOWN_SECS);
    { let ix = ix_close_claim(&svm, &b.pubkey(), &b.pubkey()); ok(crank(&mut svm, ix)); }
}
