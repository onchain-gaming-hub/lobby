//! P4 — rematch: the old room serializes the group · create + pointer in one tx (02 §5).
mod common;
use {
    ::lobby::{errors::LobbyError as E, *},
    common::*,
    solana_signer::Signer,
};

#[test]
fn first_creates_rest_join_one_room() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let new_id = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), None, new_id); ok(send(&mut svm, &[ix], &[&other])); }
    assert_eq!(room(&svm, &game, old).rematch_room_id, Some(new_id));
    let r = room(&svm, &game, new_id);
    assert_eq!((r.is_public, r.host), (false, Some(other.pubkey())));
    // the host raced a create with a stale view → StaleRematch, then joins
    let r2 = { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, new_id + 1); send(&mut svm, &[ix], &[&host]) };
    assert_err(&r2, code(E::StaleRematch));
    let r2 = { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), Some(new_id), new_id + 1); send(&mut svm, &[ix], &[&host]) };
    assert_err(&r2, code(E::StaleRematch)); // pointed room still joinable → must join
    { let ix = ix_rematch_join(&svm, &game, old, &host.pubkey(), new_id); ok(send(&mut svm, &[ix], &[&host])); }
    let c: PlayerClaim = read(&svm, &claim_pda(&host.pubkey()));
    assert_eq!((c.current.unwrap().room_id, c.last_joined_at), (new_id, now(&svm)));
    assert_eq!(room(&svm, &game, new_id).seats[1].player, host.pubkey());
}

#[test]
fn window_roster_and_phase_rules() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, _) = ended_room(&mut svm, &game);
    let stranger = player(&mut svm);
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &stranger.pubkey(), &stranger.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&stranger]), code(E::NotInRoster)); }
    advance(&mut svm, 60);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&host]), code(E::RematchClosed)); }

    // cancelled match → no rematch
    let (id, h2, _) = starting_room(&mut svm, &game);
    ok(crank(&mut svm, ix_finish_game(&game, id, false)));
    advance(&mut svm, CREATE_COOLDOWN_SECS);
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, id, &h2.pubkey(), &h2.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&h2]), code(E::WrongPhase)); }
}

#[test]
fn dead_or_full_rematch_room_is_replaced_and_pointer_overwritten() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host])); }
    // host fills it with bots → full → stale for `other`
    for s in 1..4 {
        ok(send(&mut svm, &[ix_add_bot(&game, r1, &host.pubkey(), s)], &[&host]));
    }
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::StaleRematch)); }
    let r2 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), Some(r1), r2); ok(send(&mut svm, &[ix], &[&other])); }
    assert_eq!(room(&svm, &game, old).rematch_room_id, Some(r2));
    // pointer is Some(r2) but no proof account → StaleRematch (client must re-read)
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r2 + 1); assert_err(&send(&mut svm, &[ix], &[&host]), code(E::StaleRematch)); }
}

#[test]
fn closed_rematch_room_counts_as_stale() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host])); }
    svm.set_account(room_pda(&game, r1), solana_account::Account::default()).unwrap(); // as if closed
    let r2 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), Some(r1), r2); ok(send(&mut svm, &[ix], &[&other])); }
    // proof key for a room the pointer doesn't name → StaleRematch (stale read)
    let (old2, h2, o2) = ended_room(&mut svm, &game);
    let ra = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old2, &h2.pubkey(), &h2.pubkey(), None, ra); ok(send(&mut svm, &[ix], &[&h2])); }
    { let ix = ix_rematch_create(&svm, &game, old2, &o2.pubkey(), &o2.pubkey(), Some(r2), ra + 1); assert_err(&send(&mut svm, &[ix], &[&o2]), code(E::StaleRematch)); } // proof for a room the pointer doesn't name = stale read
}

#[test]
fn kicked_player_cannot_redirect_the_group() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host])); }
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1); ok(send(&mut svm, &[ix], &[&other])); }
    ok(send(&mut svm, &[ix_kick(&game, r1, &host.pubkey(), 1, &other.pubkey())], &[&host]));
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::KickCooldown)); }
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), Some(r1), nid); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::KickCooldown)); }
    assert_eq!(room(&svm, &game, old).rematch_room_id, Some(r1));
}

#[test]
fn player_who_left_mid_game_can_still_rematch() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, other) = starting_room(&mut svm, &game);
    ok(crank(&mut svm, ix_start_game(&game, id)));
    ok(send(&mut svm, &[ix_leave_seat(&game, id, &other.pubkey())], &[&other]));
    advance(&mut svm, CREATE_COOLDOWN_SECS);
    ok(crank(&mut svm, ix_finish_game(&game, id, true)));
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, id, &host.pubkey(), &host.pubkey(), None, nid); ok(send(&mut svm, &[ix], &[&host])); }
    { let ix = ix_rematch_join(&svm, &game, id, &other.pubkey(), nid); ok(send(&mut svm, &[ix], &[&other])); }
}

#[test]
fn first_rematch_create_after_the_game_is_exempt_but_records_the_create() {
    // solo-ish creator whose game ended within the cooldown can still rematch (review P4 m1)
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, _) = starting_room(&mut svm, &game); // host created < 60s ago
    ok(crank(&mut svm, ix_start_game(&game, id)));
    advance(&mut svm, 5); // a real match takes time; created strictly before the game ended
    ok(crank(&mut svm, ix_finish_game(&game, id, true)));
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, id, &host.pubkey(), &host.pubkey(), None, nid); ok(send(&mut svm, &[ix], &[&host])); } // first rematch: exempt
    assert_eq!(read::<PlayerClaim>(&svm, &claim_pda(&host.pubkey())).last_created_at, now(&svm));
    // …so a quick create right after is cooled down
    ok(send(&mut svm, &[ix_leave_room(&game, nid, &host.pubkey(), false)], &[&host]));
    let r = { let ix = ix_qm_create(&svm, &game, &host.pubkey(), &host.pubkey(), next_room_id(&svm, &game)); send(&mut svm, &[ix], &[&host]) };
    assert_err(&r, code(E::CreateCooldown));
}

#[test]
fn kicked_caller_may_replace_a_dead_or_full_rematch_room() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host])); }
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1); ok(send(&mut svm, &[ix], &[&other])); }
    ok(send(&mut svm, &[ix_kick(&game, r1, &host.pubkey(), 1, &other.pubkey())], &[&host]));
    for s in 1..4 {
        ok(send(&mut svm, &[ix_add_bot(&game, r1, &host.pubkey(), s)], &[&host])); // now full
    }
    // within the kick cooldown, but the room is not joinable → stale, so `other` may create
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::StaleRematch)); }
    let r2 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), Some(r1), r2); ok(send(&mut svm, &[ix], &[&other])); }
}

#[test]
fn rematch_room_of_another_game_with_the_same_id_is_stale() {
    // the real case: pointer = Some(r1) and the other game's room r1 exists too
    let (mut svm, g) = game_world();
    let game = g.program;
    let other_game = anchor_lang::prelude::Pubkey::new_unique();
    setup_game(&mut svm, other_game, default_params());
    let (old, host, other) = ended_room(&mut svm, &game);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host])); }
    let mut fid = 0;
    while fid < r1 {
        let h = player(&mut svm);
        fid = create_room(&mut svm, &other_game, &h);
    }
    assert_eq!(fid, r1);
    let mut ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), r1);
    ix.accounts[4].pubkey = room_pda(&other_game, r1);
    assert_err(&send(&mut svm, &[ix], &[&other]), code(E::StaleRematch));
}

#[test]
fn one_ended_room_cannot_be_looped_to_burn_rent_budget() {
    // Opus gate P4 M1 regression: rematch_create → leave → rematch_create (dead proof) …
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, _other) = ended_room(&mut svm, &game);
    let relayer = funded(&mut svm);
    let r1 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &relayer.pubkey(), None, r1); ok(send(&mut svm, &[ix], &[&host, &relayer])); } // first: exempt
    ok(send(&mut svm, &[ix_leave_room(&game, r1, &host.pubkey(), false)], &[&host]));
    advance(&mut svm, 1);
    let r2 = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &relayer.pubkey(), Some(r1), r2); assert_err(&send(&mut svm, &[ix], &[&host, &relayer]), code(E::CreateCooldown)); }
    advance(&mut svm, CREATE_COOLDOWN_SECS);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &relayer.pubkey(), Some(r1), r2); assert!(send(&mut svm, &[ix], &[&host, &relayer]).is_err(), "window (60s) is over by now"); }
}

#[test]
fn r4_rematch_create_relayer_pays_exactly_room_rent() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, _) = ended_room(&mut svm, &game);
    let relayer = funded(&mut svm);
    let before = lamports(&svm, &relayer.pubkey());
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &relayer.pubkey(), None, nid); ok(send(&mut svm, &[ix], &[&host, &relayer])); }
    assert_eq!(before - lamports(&svm, &relayer.pubkey()), svm.minimum_balance_for_rent_exemption(351));
    assert_eq!(room(&svm, &game, nid).rent_payer, relayer.pubkey());
}

#[test]
fn live_player_cannot_rematch_into_a_second_room() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (old, host, other) = ended_room(&mut svm, &game);
    let elsewhere = player(&mut svm);
    let r = create_room(&mut svm, &game, &elsewhere);
    join(&mut svm, &game, r, &other); // `other` is live elsewhere now
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old, &other.pubkey(), &other.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::AlreadyInRoom)); }
    { let ix = ix_rematch_create(&svm, &game, old, &host.pubkey(), &host.pubkey(), None, nid); ok(send(&mut svm, &[ix], &[&host])); }
    { let ix = ix_rematch_join(&svm, &game, old, &other.pubkey(), nid); assert_err(&send(&mut svm, &[ix], &[&other]), code(E::AlreadyInRoom)); }
}
