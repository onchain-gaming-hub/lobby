//! Accounts (design notes: 01-accounts). Borsh layout — never reorder fields / enum variants.
//! Fields that off-chain code memcmps sit before the first `Option`/`Vec` (01 §0).
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum Phase {
    Waiting,
    ReadyCheck,
    Starting,
    Playing,
    Ended,
    Cancelled,
}

impl Phase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Ended | Phase::Cancelled)
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum EndReason {
    None,
    /// game CPI `end_match(Completed)` → Ended
    Completed,
    /// game CPI `end_match(Cancelled)` → Cancelled (R13, D10)
    GameCancelled,
    /// last human left (Waiting / ReadyCheck) → Cancelled
    NoHumans,
    /// LOBBY_TTL → Cancelled
    LobbyExpired,
    /// START_TTL — game never called begin_match → Cancelled
    StartExpired,
    /// MAX_GAME_TTL — game never called end_match → Cancelled
    GameExpired,
    /// Playing but every human `leave_seat` → Cancelled (rev 3 #9)
    Abandoned,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum SeatKind {
    Empty,
    Human,
    Bot,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub struct Seat {
    pub kind: SeatKind,
    /// `Pubkey::default()` when Empty / Bot.
    pub player: Pubkey,
    /// ReadyCheck only · reset when entering ReadyCheck.
    pub ready: bool,
    /// `leave_seat` during Playing (seat stays · claim dies).
    pub left: bool,
}

impl Seat {
    pub const EMPTY: Seat = Seat { kind: SeatKind::Empty, player: Pubkey::new_from_array([0; 32]), ready: false, left: false };
    /// A human that still holds this seat (not `left`).
    pub fn is_active_human(&self) -> bool {
        self.kind == SeatKind::Human && !self.left
    }
}

/// Seconds. Snapshotted into every Room at creation (01 §3).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub struct RoomTtl {
    pub wait: u32,
    pub ready: u32,
    pub lobby: u32,
    pub start: u32,
    pub max_game: u32,
    pub rematch_window: u32,
    pub retention: u32,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub struct RoomRef {
    pub game_program: Pubkey,
    pub room_id: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub struct KickMark {
    pub room: RoomRef,
    pub until: i64,
}

/// PDA `[b"lobby_config", game_program]` — one per game.
#[account]
#[derive(InitSpace, Debug)]
pub struct LobbyConfig {
    pub version: u8,
    pub bump: u8,
    pub game_program: Pubkey,
    pub max_seats: u8,
    pub min_seats: u8,
    pub min_humans: u8,
    pub public_enabled: bool,
    pub private_enabled: bool,
    pub bot_fill: bool,
    pub rematch_enabled: bool,
    pub ttl: RoomTtl,
    pub reserved: [u8; 32],
}

/// PDA `[b"matchmaker", game_program]` — one per game. Never closed in v1 (room_id uniqueness).
#[account]
#[derive(InitSpace, Debug)]
pub struct Matchmaker {
    pub version: u8,
    pub bump: u8,
    /// Starts at 1 · only `checked_add(1)` in the ix that creates Room `next_room_id`.
    pub next_room_id: u64,
    pub open_public_room: Option<u64>,
    pub reserved: [u8; 16],
}

/// PDA `[b"room", game_program, room_id.to_le_bytes()]`.
#[account]
#[derive(Debug)]
pub struct Room {
    // ── fixed offsets (memcmp-able) ──
    pub version: u8,
    pub bump: u8,
    /// L3/D11 — `close_room` refunds only here · written once at creation.
    pub rent_payer: Pubkey,
    pub game_program: Pubkey,
    pub room_id: u64,
    pub phase: Phase,
    pub end_reason: EndReason,
    pub is_public: bool,
    // ── rules snapshot from LobbyConfig ──
    pub min_seats: u8,
    pub min_humans: u8,
    pub bot_fill: bool,
    pub rematch_enabled: bool,
    pub ttl: RoomTtl,
    // ── deadline anchors ──
    pub phase_since: i64,
    pub last_human_activity_at: i64,
    // ── variable offsets (do not memcmp past here) ──
    pub host: Option<Pubkey>,
    pub rematch_room_id: Option<u64>,
    /// len = max_seats for the room's whole life (never push/pop).
    pub seats: Vec<Seat>,
    pub reserved: [u8; 32],
}

impl Room {
    /// Bytes before `seats` (excluding the discriminator) when every Option is `Some`.
    pub const FIXED_LEN: usize = 1 + 1 + 32 + 32 + 8 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + RoomTtl::INIT_SPACE + 8 + 8 + (1 + 32) + (1 + 8) + 32;

    /// Allocated account size for `n` seats: 8 disc + fixed + 4 vec prefix + 35·n (01 §6).
    pub const fn space(n: usize) -> usize {
        8 + Self::FIXED_LEN + 4 + n * Seat::INIT_SPACE
    }

    pub fn humans(&self) -> usize {
        self.seats.iter().filter(|s| s.is_active_human()).count()
    }

    pub fn occupied(&self) -> usize {
        self.seats.iter().filter(|s| s.kind != SeatKind::Empty).count()
    }

    pub fn seat_of(&self, player: &Pubkey) -> Option<usize> {
        self.seats.iter().position(|s| s.kind == SeatKind::Human && s.player == *player)
    }

    pub fn lowest_empty(&self) -> Option<usize> {
        self.seats.iter().position(|s| s.kind == SeatKind::Empty)
    }

    pub fn room_ref(&self) -> RoomRef {
        RoomRef { game_program: self.game_program, room_id: self.room_id }
    }
}

/// PDA `[b"claim", player]` — one per player, hub-wide (D8). Created only by `init_claim`.
#[account]
#[derive(InitSpace, Debug)]
pub struct PlayerClaim {
    pub version: u8,
    pub bump: u8,
    /// 🔒 R1: written once in `init_claim`, never overwritten.
    pub rent_payer: Pubkey,
    pub player: Pubkey,
    pub last_joined_at: i64,
    // ── variable offsets ──
    pub current: Option<RoomRef>,
    pub kick: Option<KickMark>,
    /// n1: last room this player created (CREATE_COOLDOWN_SECS) · 0 = never. Took 8 B of `reserved`.
    pub last_created_at: i64,
    pub reserved: [u8; 8],
}
