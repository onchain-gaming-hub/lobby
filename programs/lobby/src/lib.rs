//! lobby — rooms / matchmaking / readyCheck / rematch for blockchain-games-hub.
//!
//! Design: `specs/designs/` (01 accounts · 02 instructions · 03 proposals · 04 worker).
//! Games CPI `begin_match` / `end_match` signed by their `[b"game_authority"]` PDA (L4).
//! Every deadline has a permissionless `tick`; claims are derived, never released (02 §2).
#![allow(unexpected_cfgs)]
#![allow(clippy::result_large_err)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod logic;
pub mod state;

pub use constants::*;
pub use instructions::*;
pub use logic::ConfigParams;
pub use state::*;

declare_id!("5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu");

// Read-only consumer of platform-core (03-consumer-integration-v1): accounts only, never CPI.
declare_program!(platform_core);

#[program]
pub mod lobby {
    use super::*;

    // ── game owner (not relayed) ──
    pub fn init_lobby(ctx: Context<InitLobby>, params: ConfigParams) -> Result<()> {
        instructions::admin::handle_init_lobby(ctx, params)
    }
    pub fn update_config(ctx: Context<UpdateConfig>, params: ConfigParams) -> Result<()> {
        instructions::admin::handle_update_config(ctx, params)
    }

    // ── player ──
    pub fn init_claim(ctx: Context<InitClaim>) -> Result<()> {
        instructions::claim::handle_init_claim(ctx)
    }
    pub fn create_room(ctx: Context<CreateRoom>) -> Result<()> {
        instructions::room::handle_create_room(ctx)
    }
    pub fn join_room(ctx: Context<JoinRoom>) -> Result<()> {
        instructions::room::handle_join_room(ctx)
    }
    pub fn leave_room(ctx: Context<LeaveRoom>) -> Result<()> {
        instructions::room::handle_leave_room(ctx)
    }
    pub fn ready(ctx: Context<Ready>) -> Result<()> {
        instructions::room::handle_ready(ctx)
    }

    // ── host (private rooms) ──
    pub fn add_bot(ctx: Context<HostRoom>, seat: u8) -> Result<()> {
        instructions::host::handle_add_bot(ctx, seat)
    }
    pub fn remove_bot(ctx: Context<HostRoom>, seat: u8) -> Result<()> {
        instructions::host::handle_remove_bot(ctx, seat)
    }
    pub fn kick_player(ctx: Context<KickPlayer>, seat: u8) -> Result<()> {
        instructions::host::handle_kick_player(ctx, seat)
    }
    pub fn request_start(ctx: Context<HostRoom>) -> Result<()> {
        instructions::host::handle_request_start(ctx)
    }
}
