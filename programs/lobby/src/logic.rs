//! Pure rules shared by instructions (02). No account I/O except `claim_is_live`.
use {
    crate::{constants::*, errors::LobbyError, state::*},
    anchor_lang::prelude::*,
};

/// Config the game owner sets (`init_lobby` / `update_config`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConfigParams {
    pub max_seats: u8,
    pub min_seats: u8,
    pub min_humans: u8,
    pub public_enabled: bool,
    pub private_enabled: bool,
    pub bot_fill: bool,
    pub rematch_enabled: bool,
    pub ttl: RoomTtl,
}

/// 02 §8 `InvalidConfig` + B2 program caps.
pub fn validate_config(p: &ConfigParams) -> Result<()> {
    let t = &p.ttl;
    let ok = (2..=MAX_SEATS).contains(&p.max_seats)
        && 1 <= p.min_humans
        && p.min_humans <= p.min_seats
        && p.min_seats <= p.max_seats
        && [t.wait, t.ready, t.lobby, t.start, t.max_game, t.rematch_window, t.retention].iter().all(|x| *x > 0)
        && t.retention >= t.rematch_window
        && t.ready <= MAX_READY_TTL
        && t.start <= MAX_START_TTL
        && t.lobby <= MAX_LOBBY_TTL
        && t.max_game <= CAP_MAX_GAME_TTL
        && t.retention <= MAX_RETENTION;
    require!(ok, LobbyError::InvalidConfig);
    Ok(())
}

pub fn apply_config(c: &mut LobbyConfig, p: &ConfigParams) {
    c.max_seats = p.max_seats;
    c.min_seats = p.min_seats;
    c.min_humans = p.min_humans;
    c.public_enabled = p.public_enabled;
    c.private_enabled = p.private_enabled;
    c.bot_fill = p.bot_fill;
    c.rematch_enabled = p.rematch_enabled;
    c.ttl = p.ttl;
}

pub fn room_pda(game_program: &Pubkey, room_id: u64) -> Pubkey {
    Pubkey::find_program_address(&[ROOM_SEED, game_program.as_ref(), &room_id.to_le_bytes()], &crate::ID).0
}
