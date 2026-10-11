//! IDL 0.1.0 freeze: error numbers and enum order are registered with the relayer (retryable codes)
//! and baked into games / indexers. Tests compute codes from the enum, so a reorder would keep every
//! other test green — this snapshot is the guard. Changing it = new IDL version + announcement.
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
