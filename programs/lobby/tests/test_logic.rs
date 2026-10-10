//! Pure-logic guards that are not reachable through ixs yet (P4 will reach them).
use {::lobby::{logic::start_if_all_ready, *}, anchor_lang::prelude::Pubkey};

fn room(phase: Phase, seats: Vec<Seat>) -> Room {
    Room {
        version: 1, bump: 0, rent_payer: Pubkey::default(), game_program: Pubkey::default(), room_id: 1,
        phase, end_reason: EndReason::None, is_public: true, min_seats: 4, min_humans: 1, bot_fill: true,
        rematch_enabled: true,
        ttl: RoomTtl { wait: 1, ready: 1, lobby: 1, start: 1, max_game: 1, rematch_window: 1, retention: 1 },
        phase_since: 0, last_human_activity_at: 0, host: None, rematch_room_id: None, seats, reserved: [0; 32],
    }
}

#[test]
fn readycheck_with_zero_humans_is_not_all_ready() {
    let mut r = room(Phase::ReadyCheck, vec![Seat { kind: SeatKind::Bot, ..Seat::EMPTY }; 4]);
    assert!(!start_if_all_ready(&mut r, 0));
    assert_eq!(r.phase, Phase::ReadyCheck);
}
