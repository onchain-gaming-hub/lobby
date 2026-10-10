//! Events (02 §7): SeatChanged carries the post-state + the affected player · host pre-ready emits PlayerReady.
mod common;
use {
    ::lobby::{events::*, *},
    anchor_lang::prelude::Pubkey,
    common::*,
    solana_signer::Signer,
};

#[test]
fn seat_events_are_post_state_with_affected_player() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (h, b, c) = (player(&mut svm), player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    let m = ok({ let ix = ix_join_room(&svm, &game, id, &b.pubkey()); send(&mut svm, &[ix], &[&b]) });
    let e = &events::<SeatChanged>(&m)[0];
    assert_eq!((e.seat, e.kind, e.player, e.cause), (1, SeatKind::Human, b.pubkey(), SeatCause::Join));

    let m = ok(send(&mut svm, &[ix_kick(&game, id, &h.pubkey(), 1, &b.pubkey())], &[&h]));
    let e = &events::<SeatChanged>(&m)[0];
    assert_eq!((e.seat, e.kind, e.player, e.cause), (1, SeatKind::Empty, b.pubkey(), SeatCause::Kick));

    join(&mut svm, &game, id, &c);
    let m = ok(send(&mut svm, &[ix_leave_room(&game, id, &c.pubkey(), false)], &[&c]));
    let e = &events::<SeatChanged>(&m)[0];
    assert_eq!((e.kind, e.player, e.cause), (SeatKind::Empty, c.pubkey(), SeatCause::Leave));

    let m = ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), 2)], &[&h]));
    let e = &events::<SeatChanged>(&m)[0];
    assert_eq!((e.kind, e.player, e.cause), (SeatKind::Bot, Pubkey::default(), SeatCause::AddBot));
}

#[test]
fn request_start_emits_player_ready_for_the_host_and_ready_timeout_names_the_player() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (h, b) = (player(&mut svm), player(&mut svm));
    let id = create_room(&mut svm, &game, &h);
    join(&mut svm, &game, id, &b);
    for s in 2..4 {
        ok(send(&mut svm, &[ix_add_bot(&game, id, &h.pubkey(), s)], &[&h]));
    }
    let m = ok(send(&mut svm, &[ix_request_start(&game, id, &h.pubkey())], &[&h]));
    let r = events::<PlayerReady>(&m);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].player, h.pubkey());
    advance(&mut svm, 30);
    let m = ok(crank(&mut svm, ix_tick(&game, id, false)));
    let e = &events::<SeatChanged>(&m)[0];
    assert_eq!((e.seat, e.kind, e.player, e.cause), (1, SeatKind::Empty, b.pubkey(), SeatCause::ReadyTimeout));
    // revert clears stale ready flags
    assert!(room(&svm, &game, id).seats.iter().all(|s| !s.ready));
}
