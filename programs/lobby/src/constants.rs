//! Seeds, limits and program-level caps (01 §2 · 02 §8). Seeds and enums are append-only.

pub const LOBBY_CONFIG_SEED: &[u8] = b"lobby_config";
pub const MATCHMAKER_SEED: &[u8] = b"matchmaker";
pub const ROOM_SEED: &[u8] = b"room";
pub const CLAIM_SEED: &[u8] = b"claim";
/// The hub-wide PDA-signer seed of every game program (L4 · platform-core X5).
pub const GAME_AUTHORITY_SEED: &[u8] = b"game_authority";
/// platform-core `GameEntry` seed (read-only).
pub const PC_GAME_SEED: &[u8] = b"game";

pub const ACCOUNT_VERSION: u8 = 1;

/// Program ceiling on seats per room. 🔒 Contract with rewards-pool: must stay ≤ 32 (MAX_WINNERS).
pub const MAX_SEATS: u8 = 8;
const _: () = assert!(MAX_SEATS <= 32, "rewards-pool MAX_WINNERS contract");
/// L1 — a kicked player cannot rejoin the same room for this long.
pub const KICK_COOLDOWN_SECS: i64 = 10;
/// L13 (c) · maintainer 2026-10-10: a claim may be closed (by anyone, incl. its owner) only after this long
/// since `last_joined_at` — the relayer's rent *budget* is not refunded on close, so early closes would burn it.
pub const CLAIM_IDLE_SECS: i64 = 30 * 86_400;

// ── B2 caps (maintainer 2026-10-10 · L8) — a game owner cannot exceed these ──
/// ReadyCheck: players cannot leave → bounds how long a player can be frozen.
pub const MAX_READY_TTL: u32 = 120;
/// Starting: players cannot leave → bounds how long a player can be frozen.
pub const MAX_START_TTL: u32 = 300;
/// Waiting: rent liveness (players can always `leave_room`).
pub const MAX_LOBBY_TTL: u32 = 21_600;
/// Playing: rent liveness (players can always `leave_seat`).
pub const CAP_MAX_GAME_TTL: u32 = 86_400;
/// Ended/Cancelled: `close_room` must always become possible.
pub const MAX_RETENTION: u32 = 86_400;

// ── Fixed offsets that off-chain code memcmps (01 §0). Asserted by tests/test_layout.rs. ──
pub const ROOM_RENT_PAYER_OFFSET: usize = 10;
pub const ROOM_GAME_PROGRAM_OFFSET: usize = 42;
pub const ROOM_PHASE_OFFSET: usize = 82;
pub const ROOM_FIXED_PREFIX_LEN: usize = 133;
pub const CLAIM_RENT_PAYER_OFFSET: usize = 10;
pub const CLAIM_PLAYER_OFFSET: usize = 42;
pub const CLAIM_LAST_JOINED_OFFSET: usize = 74;
pub const CLAIM_CURRENT_OFFSET: usize = 82;
