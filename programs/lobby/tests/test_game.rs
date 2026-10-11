//! P3 — game CPI (L4) via mock-game + leave_seat (02 §6 ix 8, 17, 18 · invariant 8).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    anchor_lang::prelude::Pubkey,
    common::*,
    solana_signer::Signer,
};

#[test]
fn full_private_flow_through_mock_game_cpi() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, other) = starting_room(&mut svm, &game);
    ok(crank(&mut svm, ix_start_game(&game, id)));
    assert_eq!(room(&svm, &game, id).phase, Phase::Playing);
    ok(crank(&mut svm, ix_finish_game(&game, id, true)));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Ended, EndReason::Completed));
    // claims die with the terminal room — both can start something new
    advance(&mut svm, CREATE_COOLDOWN_SECS); // host created the first room (n1 cooldown)
    create_room(&mut svm, &game, &host);
    create_room(&mut svm, &game, &other);
}

#[test]
fn only_the_games_pda_can_begin_or_end() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, _, _) = starting_room(&mut svm, &game);
    let impostor = funded(&mut svm);
    let r = send(&mut svm, &[ix_begin_match_direct(&game, id, &impostor.pubkey())], &[&impostor]);
    assert_err(&r, code(E::BadGameAuthority));

    // mock-game driving another game's room with the other game's entry → its PDA is not that authority
    let other_game = Pubkey::new_unique();
    setup_game(&mut svm, other_game, default_params());
    let (oid, _, _) = starting_room(&mut svm, &other_game);
    assert_err(&crank(&mut svm, ix_start_game(&other_game, oid)), code(E::BadGameAuthority));
    // …and with mock-game's OWN Active entry (whose authority it can sign for): GameEntry is
    // seed-bound to room.game_program → ConstraintSeeds. This is the check that matters (Opus gate P5 F1).
    let mut ix = ix_start_game(&other_game, oid);
    ix.accounts[1].pubkey = game_entry_pda(&GAME);
    assert_err(&crank(&mut svm, ix), 2006);
}

#[test]
fn suspended_game_can_still_begin_and_finish_open_rooms() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, _, _) = starting_room(&mut svm, &game);
    put_game_entry(&mut svm, &game, &g.owner.pubkey(), true);
    ok(crank(&mut svm, ix_start_game(&game, id)));
    ok(crank(&mut svm, ix_finish_game(&game, id, false)));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::GameCancelled));
    // …but no new rooms
    let p = player(&mut svm);
    let ix = ix_create_room(&svm, &game, &p.pubkey(), &p.pubkey(), next_room_id(&svm, &game));
    assert_err(&send(&mut svm, &[ix], &[&p]), code(E::GameNotActive));
}

#[test]
fn phase_rules_for_game_cpi() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, _, _) = starting_room(&mut svm, &game);
    assert_err(&crank(&mut svm, ix_finish_game(&game, id, true)), code(E::WrongPhase)); // Completed needs Playing
    ok(crank(&mut svm, ix_finish_game(&game, id, false))); // Starting → Cancelled is allowed
    assert_err(&crank(&mut svm, ix_start_game(&game, id)), code(E::WrongPhase));

    let (id2, _, _) = starting_room(&mut svm, &game);
    ok(crank(&mut svm, ix_start_game(&game, id2)));
    assert_err(&crank(&mut svm, ix_start_game(&game, id2)), code(E::WrongPhase));
}

#[test]
fn leave_seat_frees_the_claim_keeps_the_seat_and_last_one_abandons() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, other) = starting_room(&mut svm, &game);
    assert_err(&send(&mut svm, &[ix_leave_seat(&game, id, &other.pubkey())], &[&other]), code(E::WrongPhase));
    ok(crank(&mut svm, ix_start_game(&game, id)));
    ok(send(&mut svm, &[ix_leave_seat(&game, id, &other.pubkey())], &[&other]));
    let r = room(&svm, &game, id);
    assert!(r.seats[1].left && r.seats[1].player == other.pubkey()); // seat stays (D7)
    assert_eq!(r.phase, Phase::Playing);
    assert_err(&send(&mut svm, &[ix_leave_seat(&game, id, &other.pubkey())], &[&other]), code(E::NotInRoom));
    create_room(&mut svm, &game, &other); // claim died → free while the match continues
    ok(send(&mut svm, &[ix_leave_seat(&game, id, &host.pubkey())], &[&host]));
    let r = room(&svm, &game, id);
    assert_eq!((r.phase, r.end_reason), (Phase::Cancelled, EndReason::Abandoned));
}

#[test]
fn end_match_after_lobby_cancelled_is_wrong_phase() {
    // 03 §3 point 5: the game must read Room.phase before end_match — lobby can cancel on its own
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, other) = starting_room(&mut svm, &game);
    ok(crank(&mut svm, ix_start_game(&game, id)));
    ok(send(&mut svm, &[ix_leave_seat(&game, id, &host.pubkey())], &[&host]));
    ok(send(&mut svm, &[ix_leave_seat(&game, id, &other.pubkey())], &[&other]));
    assert_eq!(room(&svm, &game, id).end_reason, EndReason::Abandoned);
    assert_err(&crank(&mut svm, ix_finish_game(&game, id, true)), code(E::WrongPhase));
}
