//! lobby — rooms / matchmaking / readyCheck / rematch for blockchain-games-hub.
//!
//! Design: `specs/designs/` (01 accounts · 02 instructions · 03 proposals · 04 worker).
//! Games CPI `begin_match` / `end_match` signed by their `[b"game_authority"]` PDA (L4).
//! Every deadline has a permissionless `tick`; claims are derived, never released (02 §2).
#![allow(unexpected_cfgs)]
#![allow(clippy::result_large_err)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod state;

pub use constants::*;
pub use state::*;

declare_id!("5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu");

// Read-only consumer of platform-core (03-consumer-integration-v1): accounts only, never CPI.
declare_program!(platform_core);

#[program]
pub mod lobby {}
