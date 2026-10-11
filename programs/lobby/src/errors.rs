//! Error codes (02 §8). Append-only — the relayer registers some of these as retryable by number.
use anchor_lang::prelude::*;

#[error_code]
pub enum LobbyError {
    // ── retryable races (register with relayer · SKILL v3 §3.9) ──
    #[msg("Matchmaker changed (pointer or next_room_id) — re-read and retry")]
    StaleMatchmaker,
    #[msg("Rematch pointer changed or rematch room not joinable — re-read and retry")]
    StaleRematch,
    #[msg("Room is full")]
    RoomFull,
    #[msg("Room is not in Waiting")]
    NotWaiting,
    // ── everything else ──
    #[msg("prev_room is required (claim points at a room)")]
    PrevRoomRequired,
    #[msg("prev_room is not the room the claim points at")]
    PrevRoomMismatch,
    #[msg("Claim is still live (player is in a room)")]
    ClaimLive,
    #[msg("Player is already in a room")]
    AlreadyInRoom,
    #[msg("Game is not Active in platform-core")]
    GameNotActive,
    #[msg("This mode is disabled for the game")]
    ModeDisabled,
    #[msg("Wrong phase for this instruction")]
    WrongPhase,
    #[msg("Caller is not in this room")]
    NotInRoom,
    #[msg("Only the host can do this")]
    NotHost,
    #[msg("Public rooms have no host controls")]
    PublicNoHostControl,
    #[msg("Seat is not a human")]
    NotAHuman,
    #[msg("Host cannot kick themselves")]
    CannotKickSelf,
    #[msg("Seat is not empty")]
    SeatNotEmpty,
    #[msg("Seat is not a bot")]
    SeatNotBot,
    #[msg("Bots are disabled for this game")]
    BotsDisabled,
    #[msg("Kicked from this room recently — wait for the cooldown")]
    KickCooldown,
    #[msg("Not enough occupied seats to start")]
    NotEnoughSeats,
    #[msg("Not enough humans to start")]
    NotEnoughHumans,
    #[msg("Rematch window is closed")]
    RematchClosed,
    #[msg("Caller was not in the old room's roster")]
    NotInRoster,
    #[msg("Matchmaker account is required for public rooms")]
    MatchmakerRequired,
    #[msg("Room retention period has not passed")]
    RetentionNotReached,
    #[msg("Signer is not the game's game_authority PDA")]
    BadGameAuthority,
    #[msg("Invalid lobby config")]
    InvalidConfig,
    #[msg("Signer is not GameEntry.owner")]
    NotGameOwner,
    #[msg("Seat index out of range, or the victim claim does not belong to that seat")]
    InvalidSeat,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    /// Defensive: quick_match_join on a room that is not public (unreachable while invariant 7 holds).
    #[msg("Room is not a public room")]
    WrongRoom,
    #[msg("Claim has not been idle for CLAIM_IDLE_SECS yet")]
    ClaimNotIdle,
    #[msg("Created a room too recently — wait CREATE_COOLDOWN_SECS")]
    CreateCooldown,
}
