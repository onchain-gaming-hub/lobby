//! Every per-game account is seed-bound to the room's / config's game (02 §3.1 🔒). Swapping in
//! another game's account must fail with ConstraintSeeds (2006) — Opus gate P5 F1.
mod common;
use {anchor_lang::prelude::Pubkey, common::*, solana_signer::Signer};

const SEEDS: u32 = 2006;

#[test]
fn create_room_with_another_games_active_entry_cannot_bypass_suspend() {
    let mut svm = new_svm();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    let ga = setup_game(&mut svm, a, default_params());
    setup_game(&mut svm, b, default_params());
    put_game_entry(&mut svm, &a, &ga.owner.pubkey(), true); // A suspended
    let p = player(&mut svm);
    let mut ix = ix_create_room(&svm, &a, &p.pubkey(), &p.pubkey(), next_room_id(&svm, &a));
    ix.accounts[2].pubkey = game_entry_pda(&b); // B is Active
    assert_err(&send(&mut svm, &[ix], &[&p]), SEEDS);
}

#[test]
fn quick_match_create_with_another_games_entry_or_config() {
    let mut svm = new_svm();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    setup_game(&mut svm, a, default_params());
    setup_game(&mut svm, b, default_params());
    let p = player(&mut svm);
    let mut ix = ix_qm_create(&svm, &a, &p.pubkey(), &p.pubkey(), next_room_id(&svm, &a));
    ix.accounts[2].pubkey = game_entry_pda(&b);
    assert_err(&send(&mut svm, &[ix], &[&p]), SEEDS);
    let mut ix = ix_qm_create(&svm, &a, &p.pubkey(), &p.pubkey(), next_room_id(&svm, &a));
    ix.accounts[4].pubkey = matchmaker_pda(&b); // config A + matchmaker B
    assert_err(&send(&mut svm, &[ix], &[&p]), SEEDS);
}

#[test]
fn another_games_matchmaker_is_rejected_on_join_tick_and_leave() {
    let mut svm = new_svm();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    setup_game(&mut svm, a, default_params());
    setup_game(&mut svm, b, default_params());
    let pa = player(&mut svm);
    let ida = qm(&mut svm, &a, &pa);
    let pb = player(&mut svm);
    qm(&mut svm, &b, &pb);
    let j = player(&mut svm);
    let mut ix = ix_qm_join(&svm, &a, ida, &j.pubkey());
    ix.accounts[1].pubkey = matchmaker_pda(&b);
    assert_err(&send(&mut svm, &[ix], &[&j]), SEEDS);
    let mut ix = ix_leave_room(&a, ida, &pa.pubkey(), true);
    ix.accounts[2].pubkey = matchmaker_pda(&b);
    assert_err(&send(&mut svm, &[ix], &[&pa]), SEEDS);
    advance(&mut svm, 1801);
    let mut t = ix_tick(&a, ida, true);
    t.accounts[1].pubkey = matchmaker_pda(&b);
    assert_err(&crank(&mut svm, t), SEEDS);
}

#[test]
fn closed_room_returns_retryable_3012() {
    // 06 §3 registers Anchor 3012 as retryable for "room was closed first"
    let (mut svm, g) = game_world();
    let game = g.program;
    let host = player(&mut svm);
    let id = create_room(&mut svm, &game, &host);
    ok(send(&mut svm, &[ix_leave_room(&game, id, &host.pubkey(), false)], &[&host]));
    advance(&mut svm, 601);
    ok(crank(&mut svm, ix_close_room(&game, id, &host.pubkey())));
    let p = player(&mut svm);
    let r = { let ix = ix_join_room(&svm, &game, id, &p.pubkey()); send(&mut svm, &[ix], &[&p]) };
    assert_err(&r, 3012);
    assert_err(&crank(&mut svm, ix_tick(&game, id, false)), 3012);
}

#[test]
fn update_config_with_another_games_entry_is_rejected() {
    // B's owner passes B's entry with A's config → entry is seed-bound to config.game_program
    let mut svm = new_svm();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    setup_game(&mut svm, a, default_params());
    let gb = setup_game(&mut svm, b, default_params());
    let mut ix = ix_update_config(&a, &gb.owner.pubkey(), default_params());
    ix.accounts[1].pubkey = game_entry_pda(&b);
    assert_err(&send(&mut svm, &[ix], &[&gb.owner]), SEEDS);
}

#[test]
fn rematch_create_cannot_bypass_suspend_with_another_games_entry() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let other = Pubkey::new_unique();
    setup_game(&mut svm, other, default_params());
    let (old, host, _) = ended_room(&mut svm, &game);
    put_game_entry(&mut svm, &game, &g.owner.pubkey(), true); // suspend the mock game
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&host]), code(::lobby::errors::LobbyError::GameNotActive)); }
    let mut ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, nid);
    ix.accounts[2].pubkey = game_entry_pda(&other);
    assert_err(&send(&mut svm, &[ix], &[&host]), SEEDS);
}

#[test]
fn quick_match_create_on_a_suspended_game_is_game_not_active() {
    let mut svm = new_svm();
    let a = Pubkey::new_unique();
    let ga = setup_game(&mut svm, a, default_params());
    put_game_entry(&mut svm, &a, &ga.owner.pubkey(), true);
    let p = player(&mut svm);
    let ix = ix_qm_create(&svm, &a, &p.pubkey(), &p.pubkey(), next_room_id(&svm, &a));
    assert_err(&send(&mut svm, &[ix], &[&p]), code(::lobby::errors::LobbyError::GameNotActive));
}
