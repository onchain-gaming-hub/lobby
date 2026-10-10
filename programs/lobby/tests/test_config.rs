//! P1 — init_lobby / update_config (02 §6 ix 1–2 · §8 validation + B2 caps).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    anchor_lang::prelude::Pubkey,
    common::*,
    solana_signer::Signer,
};

#[test]
fn init_lobby_writes_config_and_matchmaker() {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    let g = setup_game(&mut svm, game, default_params());
    let c: LobbyConfig = read(&svm, &config_pda(&game));
    assert_eq!((c.version, c.game_program, c.max_seats, c.min_seats, c.min_humans), (1, game, 4, 4, 1));
    assert_eq!(c.ttl, default_params().ttl);
    let m: Matchmaker = read(&svm, &matchmaker_pda(&game));
    assert_eq!((m.next_room_id, m.open_public_room), (1, None));
    // owner paid (not anyone else)
    assert!(lamports(&svm, &g.owner.pubkey()) < 100 * SOL);
}

#[test]
fn only_game_owner_and_only_active_game() {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    let owner = funded(&mut svm);
    let stranger = funded(&mut svm);
    put_game_entry(&mut svm, &game, &owner.pubkey(), false);
    let r = send(&mut svm, &[ix_init_lobby(&game, &stranger.pubkey(), default_params())], &[&stranger]);
    assert_err(&r, code(E::NotGameOwner));

    let suspended = Pubkey::new_unique();
    put_game_entry(&mut svm, &suspended, &owner.pubkey(), true);
    let r = send(&mut svm, &[ix_init_lobby(&suspended, &owner.pubkey(), default_params())], &[&owner]);
    assert_err(&r, code(E::GameNotActive));
}

#[test]
fn game_entry_of_another_game_is_rejected() {
    let mut svm = new_svm();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    let owner = funded(&mut svm);
    put_game_entry(&mut svm, &a, &owner.pubkey(), false);
    put_game_entry(&mut svm, &b, &owner.pubkey(), false);
    let mut ix = ix_init_lobby(&a, &owner.pubkey(), default_params());
    ix.accounts[2].pubkey = game_entry_pda(&b); // swap in B's entry
    let r = send(&mut svm, &[ix], &[&owner]);
    assert!(format!("{:?}", r.unwrap_err().err).contains("Custom(2006)"), "ConstraintSeeds");
}

#[test]
fn init_twice_fails() {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    let g = setup_game(&mut svm, game, default_params());
    let r = send(&mut svm, &[ix_init_lobby(&game, &g.owner.pubkey(), default_params())], &[&g.owner]);
    assert!(r.is_err());
}

#[test]
fn invalid_configs_are_rejected() {
    let base = default_params();
    let mut bad: Vec<(&str, ConfigParams)> = Vec::new();
    let mut p = base; p.max_seats = 1; bad.push(("max_seats 1", p));
    let mut p = base; p.max_seats = MAX_SEATS + 1; bad.push(("max_seats > MAX_SEATS", p));
    let mut p = base; p.min_humans = 0; bad.push(("min_humans 0", p));
    let mut p = base; p.min_seats = 2; p.min_humans = 3; bad.push(("min_humans > min_seats", p));
    let mut p = base; p.min_seats = 5; bad.push(("min_seats > max_seats", p));
    for f in 0..7 {
        let mut p = base;
        let t = &mut p.ttl;
        *[&mut t.wait, &mut t.ready, &mut t.lobby, &mut t.start, &mut t.max_game, &mut t.rematch_window, &mut t.retention][f] = 0;
        bad.push(("ttl zero", p));
    }
    let mut p = base; p.ttl.retention = 59; bad.push(("retention < rematch_window", p));
    let mut p = base; p.ttl.ready = MAX_READY_TTL + 1; bad.push(("ready cap", p));
    let mut p = base; p.ttl.start = MAX_START_TTL + 1; bad.push(("start cap", p));
    let mut p = base; p.ttl.lobby = MAX_LOBBY_TTL + 1; bad.push(("lobby cap", p));
    let mut p = base; p.ttl.max_game = CAP_MAX_GAME_TTL + 1; bad.push(("max_game cap", p));
    let mut p = base; p.ttl.retention = MAX_RETENTION + 1; bad.push(("retention cap", p));

    let mut svm = new_svm();
    let owner = funded(&mut svm);
    for (why, p) in bad {
        let game = Pubkey::new_unique();
        put_game_entry(&mut svm, &game, &owner.pubkey(), false);
        let r = send(&mut svm, &[ix_init_lobby(&game, &owner.pubkey(), p)], &[&owner]);
        assert!(r.is_err(), "{why} should fail");
        assert_err(&r, code(E::InvalidConfig));
    }
}

#[test]
fn caps_themselves_are_allowed() {
    let mut p = default_params();
    p.max_seats = MAX_SEATS;
    p.min_seats = MAX_SEATS;
    p.ttl = RoomTtl { wait: 1, ready: MAX_READY_TTL, lobby: MAX_LOBBY_TTL, start: MAX_START_TTL, max_game: CAP_MAX_GAME_TTL, rematch_window: 1, retention: MAX_RETENTION };
    let mut svm = new_svm();
    setup_game(&mut svm, Pubkey::new_unique(), p);
}

#[test]
fn update_config_owner_only_and_validated() {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    let g = setup_game(&mut svm, game, default_params());
    let mut p = default_params();
    p.bot_fill = false;
    p.ttl.lobby = 900;
    ok(send(&mut svm, &[ix_update_config(&game, &g.owner.pubkey(), p)], &[&g.owner]));
    let c: LobbyConfig = read(&svm, &config_pda(&game));
    assert!(!c.bot_fill);
    assert_eq!(c.ttl.lobby, 900);

    let stranger = funded(&mut svm);
    let r = send(&mut svm, &[ix_update_config(&game, &stranger.pubkey(), p)], &[&stranger]);
    assert_err(&r, code(E::NotGameOwner));

    p.ttl.ready = MAX_READY_TTL + 1;
    let r = send(&mut svm, &[ix_update_config(&game, &g.owner.pubkey(), p)], &[&g.owner]);
    assert_err(&r, code(E::InvalidConfig));
}
