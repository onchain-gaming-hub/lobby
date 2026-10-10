//! Events for indexers (02 §7). Every account create/close event carries rent_payer (relayer SKILL v3 §2 (P) 7).
use {crate::state::*, anchor_lang::prelude::*};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeatCause {
    Join,
    Leave,
    Kick,
    AddBot,
    RemoveBot,
    ReadyTimeout,
    Backfill,
    Purge,
}

#[event]
pub struct LobbyInitialized {
    pub game_program: Pubkey,
    pub owner: Pubkey,
    pub at: i64,
}

#[event]
pub struct LobbyConfigUpdated {
    pub game_program: Pubkey,
    pub at: i64,
}

#[event]
pub struct RoomCreated {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub is_public: bool,
    pub host: Option<Pubkey>,
    pub rent_payer: Pubkey,
    pub rematch_of: Option<u64>,
    pub at: i64,
}

#[event]
pub struct SeatChanged {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub seat: u8,
    pub kind: SeatKind,
    pub player: Pubkey,
    pub cause: SeatCause,
    pub at: i64,
}

#[event]
pub struct HostChanged {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub host: Pubkey,
    pub at: i64,
}

#[event]
pub struct PhaseChanged {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub from: Phase,
    pub to: Phase,
    pub end_reason: EndReason,
    pub at: i64,
}

#[event]
pub struct PlayerReady {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub player: Pubkey,
    pub at: i64,
}

#[event]
pub struct PlayerLeftSeat {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub player: Pubkey,
    pub seat: u8,
    pub at: i64,
}

#[event]
pub struct MatchmakerPointer {
    pub game_program: Pubkey,
    pub open_public_room: Option<u64>,
    pub at: i64,
}

#[event]
pub struct RoomClosed {
    pub game_program: Pubkey,
    pub room_id: u64,
    pub refunded_to: Pubkey,
    pub lamports: u64,
    pub at: i64,
}

#[event]
pub struct ClaimCreated {
    pub player: Pubkey,
    pub rent_payer: Pubkey,
    pub at: i64,
}

#[event]
pub struct ClaimClosed {
    pub player: Pubkey,
    pub refunded_to: Pubkey,
    pub lamports: u64,
    pub at: i64,
}
