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
    // wrong proof key → WrongRoom
    let (old2, h2, o2) = ended_room(&mut svm, &game);
    let ra = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, old2, &h2.pubkey(), &h2.pubkey(), None, ra); ok(send(&mut svm, &[ix], &[&h2])); }
    { let ix = ix_rematch_create(&svm, &game, old2, &o2.pubkey(), &o2.pubkey(), Some(r2), ra + 1); assert_err(&send(&mut svm, &[ix], &[&o2]), code(E::WrongRoom)); }
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
fn rematch_create_respects_the_create_cooldown() {
    let (mut svm, g) = game_world();
    let game = g.program;
    let (id, host, _) = starting_room(&mut svm, &game); // host created < 60s ago
    ok(crank(&mut svm, ix_start_game(&game, id)));
    ok(crank(&mut svm, ix_finish_game(&game, id, true)));
    let nid = next_room_id(&svm, &game);
    { let ix = ix_rematch_create(&svm, &game, id, &host.pubkey(), &host.pubkey(), None, nid); assert_err(&send(&mut svm, &[ix], &[&host]), code(E::CreateCooldown)); }
}
