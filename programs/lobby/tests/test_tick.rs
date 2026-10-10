//! P3 — tick: every deadline of a private room (02 §1, §4 · invariant 3). Public rooms → P4.
mod common;
use {::lobby::*, common::*, solana_signer::Signer};

#[test]
fn nothing_due_is_a_successful_no_op() {
    let (mut svm, g) = game_world();
    let host = player(&mut svm);
    let id = create_room(&mut svm, &g.program, &host);
    let before = svm.get_account(&room_pda(&g.program, id)).unwrap().data;
    ok(crank(&mut svm, ix_tick(&g.program, id, false)));
    assert_eq!(svm.get_account(&room_pda(&g.program, id)).unwrap().data, before);
}

#[test]
fn lobby_ttl_counts_from_last_human_activity() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &host);
    advance(&mut svm, 1000);
    join(&mut svm, &game, id, &b); // activity
    advance(&mut svm, 1000); // 2000s since create, 1000s since activity
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Waiting);
    advance(&mut svm, 800); // 1800s since activity
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::LobbyExpired));
    create_room(&mut svm, &game, &b); // claim free
}

#[test]
fn ready_timeout_private_removes_unready_keeps_host_bots_and_reverts() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, b, c) = (player(&mut svm), player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &host);
    join(&mut svm, &game, id, &b);
    join(&mut svm, &game, id, &c);
    ok(send(&mut svm, &[ix_add_bot(&game, id, &host.pubkey(), 3)], &[&host]));
    ok(send(&mut svm, &[ix_request_start(&game, id, &host.pubkey())], &[&host]));
    ok(send(&mut svm, &[ix_ready(&game, id, &c.pubkey())], &[&c]));
    advance(&mut svm, 29);
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    assert_eq!(room(&svm, &game, id).phase, Phase::ReadyCheck); // not yet
    advance(&mut svm, 1);
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    let r = room(&svm, &game, id);
    assert_eq!(r.phase, Phase::Waiting);
    assert_eq!(r.seats[1].kind, SeatKind::Empty); // B never readied
    assert_eq!(r.seats[2].player, c.pubkey());
    assert_eq!(r.seats[3].kind, SeatKind::Bot); // host's bot kept (private)
    assert_eq!(r.host, Some(host.pubkey()));
    assert_eq!(r.last_human_activity_at, now(&svm));
    join(&mut svm, &game, id, &b); // B's claim died → may rejoin (no kick mark)
}

#[test]
fn ready_after_deadline_but_before_tick_still_counts() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (host, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &host);
    join(&mut svm, &game, id, &b);
    for s in 2..4 {
        ok(send(&mut svm, &[ix_add_bot(&game, id, &host.pubkey(), s)], &[&host]));
    }
    ok(send(&mut svm, &[ix_request_start(&game, id, &host.pubkey())], &[&host]));
    advance(&mut svm, 45);
    ok(send(&mut svm, &[ix_ready(&game, id, &b.pubkey())], &[&b])); // 02 §1: deadline = earliest tick
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting);
}

#[test]
fn start_ttl_and_game_ttl_cancel() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, _, _) = starting_room(&mut svm, &game);
    advance(&mut svm, 59);
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting);
    advance(&mut svm, 1);
    ok(crank(&mut svm, ix_tick(&game, id, false)));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::StartExpired));
    assert_err(&crank(&mut svm, ix_start_game(&game, id)), code(::lobby::errors::LobbyError::WrongPhase)); // too late now

    let (id2, _, _) = starting_room(&mut svm, &game);
    advance(&mut svm, 120);
    ok(crank(&mut svm, ix_start_game(&game, id2))); // late but before any tick → allowed
    advance(&mut svm, 3599);
    ok(crank(&mut svm, ix_tick(&game, id2, false)));
    assert_eq!(room(&svm, &game, id2).phase, Phase::Playing);
    advance(&mut svm, 1);
    ok(crank(&mut svm, ix_tick(&game, id2, false)));
    let r = room(&svm, &game, id2);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::GameExpired));
    // terminal → tick is a no-op
    ok(crank(&mut svm, ix_tick(&game, id2, false)));
}

#[test]
fn relayer_crank_shape_placeholder_plus_trailing_readonly_user_signer() {
    // SKILL v3 §3.7: FE appends the user as a readonly signer after the crank's accounts;
    // private rooms pass the matchmaker placeholder (= program id) so the signer can't land in that slot.
    let (mut svm, g) = game_world();
    let game = g.program;
    let host = player(&mut svm);
    let id = create_room(&mut svm, &game, &host);
    advance(&mut svm, 1800);
    let relayer = funded(&mut svm); // fee payer (slot 0)
    let user = funded(&mut svm); // separate readonly signer for quota
    let mut ix = ix_tick(&game, id, false);
    assert_eq!(ix.accounts[1].pubkey, ::lobby::ID); // placeholder present
    ix.accounts.push(anchor_lang::prelude::AccountMeta::new_readonly(user.pubkey(), true));
    let m = ok(send(&mut svm, &[ix], &[&relayer, &user]));
    assert!(!m.logs.is_empty());
    assert_eq!(room(&svm, &game, id).end_reason, EndReason::LobbyExpired);
}
