//! IDL 0.1.0 freeze: error numbers and enum order are registered with the relayer (retryable codes)
//! and baked into games / indexers. Tests compute codes from the enum, so a reorder would keep every
//! other test green — this snapshot is the guard. Changing it = new IDL version + announcement.
#[test]
fn idl_matches_the_frozen_0_1_0_snapshot() {
    // The whole IDL (ix account order games compile against, events, types, errors) is frozen.
    // Any change must bump the IDL version and add a new frozen file — never edit this one.
    let current: serde_json::Value = serde_json::from_str(include_str!("../../../idls/lobby.json")).unwrap();
    let frozen: serde_json::Value = serde_json::from_str(include_str!("../../../idls/lobby-0.1.0.frozen.json")).unwrap();
    assert_eq!(current, frozen, "idls/lobby.json differs from the frozen 0.1.0 snapshot — bump the version");
}

fn idl() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../idls/lobby.json")).unwrap()
}

#[test]
fn idl_version_is_0_1_0() {
    assert_eq!(idl()["metadata"]["version"], "0.1.0");
}

#[test]
fn error_codes_are_frozen() {
    let frozen = [
        "StaleMatchmaker", "StaleRematch", "RoomFull", "NotWaiting", "PrevRoomRequired", "PrevRoomMismatch",
        "ClaimLive", "AlreadyInRoom", "GameNotActive", "ModeDisabled", "WrongPhase", "NotInRoom", "NotHost",
        "PublicNoHostControl", "NotAHuman", "CannotKickSelf", "SeatNotEmpty", "SeatNotBot", "BotsDisabled",
        "KickCooldown", "NotEnoughSeats", "NotEnoughHumans", "RematchClosed", "NotInRoster", "MatchmakerRequired",
        "RetentionNotReached", "BadGameAuthority", "InvalidConfig", "NotGameOwner", "InvalidSeat", "MathOverflow",
        "WrongRoom", "ClaimNotIdle", "CreateCooldown",
    ];
    let errs = idl()["errors"].as_array().unwrap().clone();
    assert!(errs.len() >= frozen.len(), "errors may only be appended");
    for (i, name) in frozen.iter().enumerate() {
        assert_eq!(errs[i]["code"], 6000 + i as u64);
        assert_eq!(errs[i]["name"], *name, "code {} renamed/reordered", 6000 + i);
    }
}

#[test]
fn enum_variant_order_is_frozen() {
    let types = idl()["types"].as_array().unwrap().clone();
    let variants = |name: &str| -> Vec<String> {
        let t = types.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("type {name}"));
        t["type"]["variants"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
    };
    let frozen: &[(&str, &[&str])] = &[
        ("Phase", &["Waiting", "ReadyCheck", "Starting", "Playing", "Ended", "Cancelled"]),
        ("EndReason", &["None", "Completed", "GameCancelled", "NoHumans", "LobbyExpired", "StartExpired", "GameExpired", "Abandoned"]),
        ("SeatKind", &["Empty", "Human", "Bot"]),
        ("SeatCause", &["Join", "Leave", "Kick", "AddBot", "RemoveBot", "ReadyTimeout", "Backfill", "Purge"]),
        ("MatchResult", &["Completed", "Cancelled"]),
    ];
    for (name, want) in frozen {
        let got = variants(name);
        assert!(got.len() >= want.len(), "{name}: variants may only be appended");
        assert_eq!(&got[..want.len()], *want, "{name} reordered");
    }
}

#[test]
fn event_and_struct_field_order_is_frozen() {
    // indexers decode events and memcmp/deserialize accounts by field order
    let idl = idl();
    let events: Vec<&str> = idl["events"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    for e in ["ClaimClosed", "ClaimCreated", "HostChanged", "LobbyConfigUpdated", "LobbyInitialized", "MatchmakerPointer",
              "PhaseChanged", "PlayerLeftSeat", "PlayerReady", "RoomClosed", "RoomCreated", "SeatChanged"] {
        assert!(events.contains(&e), "event {e} removed/renamed");
    }
    let frozen: &[(&str, &[&str])] = &[
        ("Room", &["version", "bump", "rent_payer", "game_program", "room_id", "phase", "end_reason", "is_public", "min_seats",
                   "min_humans", "bot_fill", "rematch_enabled", "ttl", "phase_since", "last_human_activity_at", "host",
                   "rematch_room_id", "seats", "reserved"]),
        ("PlayerClaim", &["version", "bump", "rent_payer", "player", "last_joined_at", "current", "kick", "last_created_at", "reserved"]),
        ("LobbyConfig", &["version", "bump", "game_program", "max_seats", "min_seats", "min_humans", "public_enabled",
                          "private_enabled", "bot_fill", "rematch_enabled", "ttl", "reserved"]),
        ("Matchmaker", &["version", "bump", "next_room_id", "open_public_room", "reserved"]),
        ("Seat", &["kind", "player", "ready", "left"]),
        ("RoomTtl", &["wait", "ready", "lobby", "start", "max_game", "rematch_window", "retention"]),
        ("RoomRef", &["game_program", "room_id"]),
        ("KickMark", &["room", "until"]),
        ("ConfigParams", &["max_seats", "min_seats", "min_humans", "public_enabled", "private_enabled", "bot_fill", "rematch_enabled", "ttl"]),
        ("SeatChanged", &["game_program", "room_id", "seat", "kind", "player", "cause", "at"]),
        ("PhaseChanged", &["game_program", "room_id", "from", "to", "end_reason", "at"]),
        ("RoomCreated", &["game_program", "room_id", "is_public", "host", "rent_payer", "rematch_of", "at"]),
        ("RoomClosed", &["game_program", "room_id", "refunded_to", "lamports", "at"]),
        ("ClaimCreated", &["player", "rent_payer", "at"]),
        ("ClaimClosed", &["player", "refunded_to", "lamports", "at"]),
    ];
    let types = idl["types"].as_array().unwrap();
    for (name, want) in frozen {
        let t = types.iter().find(|t| t["name"] == *name).unwrap_or_else(|| panic!("type {name}"));
        let got: Vec<&str> = t["type"]["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
        assert_eq!(&got[..], *want, "{name} fields changed");
    }
}
