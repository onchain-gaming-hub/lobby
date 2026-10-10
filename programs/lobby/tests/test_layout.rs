//! Layout guard (01 §0, §6): sizes + fixed offsets that the Worker / relayer / indexers memcmp.
//! If this fails, a field moved — that is a breaking change (versioned IDL · P2).
use anchor_lang::{prelude::Pubkey, AccountSerialize, Discriminator, Space};
use ::lobby::*;

fn ser<T: AccountSerialize>(a: &T) -> Vec<u8> {
    let mut v = Vec::new();
    a.try_serialize(&mut v).unwrap();
    v
}

fn ttl() -> RoomTtl {
    RoomTtl { wait: 60, ready: 30, lobby: 1800, start: 60, max_game: 3600, rematch_window: 60, retention: 600 }
}

#[test]
fn sizes_match_design() {
    assert_eq!(8 + LobbyConfig::INIT_SPACE, 109);
    assert_eq!(8 + Matchmaker::INIT_SPACE, 43);
    assert_eq!(8 + PlayerClaim::INIT_SPACE, 188);
    assert_eq!(Seat::INIT_SPACE, 35);
    assert_eq!(Room::space(2), 281);
    assert_eq!(Room::space(4), 351);
    assert_eq!(Room::space(MAX_SEATS as usize), 491);
}

#[test]
fn room_offsets_and_full_size() {
    let rent_payer = Pubkey::new_unique();
    let game = Pubkey::new_unique();
    let room = Room {
        version: 1,
        bump: 254,
        rent_payer,
        game_program: game,
        room_id: 7,
        phase: Phase::Cancelled,
        end_reason: EndReason::Abandoned,
        is_public: true,
        min_seats: 4,
        min_humans: 1,
        bot_fill: true,
        rematch_enabled: true,
        ttl: ttl(),
        phase_since: 111,
        last_human_activity_at: 222,
        host: Some(Pubkey::new_unique()),
        rematch_room_id: Some(9),
        seats: vec![Seat::EMPTY; 4],
        reserved: [0; 32],
    };
    let d = ser(&room);
    assert_eq!(&d[..8], Room::DISCRIMINATOR);
    // every Option = Some → serialized size == allocated size
    assert_eq!(d.len(), Room::space(4));
    assert_eq!(&d[ROOM_RENT_PAYER_OFFSET..ROOM_RENT_PAYER_OFFSET + 32], rent_payer.as_ref());
    assert_eq!(&d[ROOM_GAME_PROGRAM_OFFSET..ROOM_GAME_PROGRAM_OFFSET + 32], game.as_ref());
    assert_eq!(&d[74..82], &7u64.to_le_bytes());
    assert_eq!(d[ROOM_PHASE_OFFSET], Phase::Cancelled as u8);
    assert_eq!(d[84], 1); // is_public
    assert_eq!(d[87], 1); // bot_fill
    assert_eq!(&d[89..93], &60u32.to_le_bytes()); // ttl.wait
    assert_eq!(&d[117..125], &111i64.to_le_bytes()); // phase_since
    assert_eq!(&d[125..133], &222i64.to_le_bytes()); // last_human_activity_at
    assert_eq!(d[ROOM_FIXED_PREFIX_LEN], 1); // host = Some
}

#[test]
fn claim_offsets_and_full_size() {
    let rent_payer = Pubkey::new_unique();
    let player = Pubkey::new_unique();
    let r = RoomRef { game_program: Pubkey::new_unique(), room_id: 3 };
    let claim = PlayerClaim {
        version: 1,
        bump: 255,
        rent_payer,
        player,
        last_joined_at: 333,
        current: Some(r),
        kick: Some(KickMark { room: r, until: 444 }),
        reserved: [0; 16],
    };
    let d = ser(&claim);
    assert_eq!(d.len(), 8 + PlayerClaim::INIT_SPACE);
    assert_eq!(&d[CLAIM_RENT_PAYER_OFFSET..CLAIM_RENT_PAYER_OFFSET + 32], rent_payer.as_ref());
    assert_eq!(&d[CLAIM_PLAYER_OFFSET..CLAIM_PLAYER_OFFSET + 32], player.as_ref());
    assert_eq!(&d[CLAIM_LAST_JOINED_OFFSET..CLAIM_LAST_JOINED_OFFSET + 8], &333i64.to_le_bytes());
    assert_eq!(d[CLAIM_CURRENT_OFFSET], 1); // current = Some
}

#[test]
fn enums_are_append_only() {
    // explicit values — reordering a variant breaks every memcmp filter and stored account
    assert_eq!(Phase::Waiting as u8, 0);
    assert_eq!(Phase::Playing as u8, 3);
    assert_eq!(Phase::Cancelled as u8, 5);
    assert_eq!(EndReason::Abandoned as u8, 7);
    assert_eq!(SeatKind::Bot as u8, 2);
}

#[test]
fn caps_respect_contracts() {
    // MAX_SEATS <= 32 is a compile-time assert in constants.rs
    assert_eq!((MAX_READY_TTL, MAX_START_TTL, MAX_LOBBY_TTL, CAP_MAX_GAME_TTL, MAX_RETENTION), (120, 300, 21_600, 86_400, 86_400));
}
