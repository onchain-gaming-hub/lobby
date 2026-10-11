# lobby

A shared Solana program for **rooms, matchmaking, ready checks and rematch**, used by every game in blockchain-games-hub. A game never re-implements its lobby: players gather in a lobby `Room`, the lobby freezes the roster, and the game takes over through a CPI.

| | |
|---|---|
| Program ID (devnet) | `5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu` |
| IDL | [`idls/lobby.json`](idls/lobby.json), version **0.1.0** (frozen; a test compares it with [`idls/lobby-0.1.0.frozen.json`](idls/lobby-0.1.0.frozen.json)) |
| Framework | Anchor 1.2.1 (TS clients: `@anchor-lang/core` 1.2.1 or `@solana/kit`) |
| Cluster | **devnet only** |
| Depends on | platform-core program (devnet `AJnPSYsn7J2muHfKztRtpZz5kBP94dWkNZLr9aMhu18G`): every game must be registered there |

> **Integrating a game or a frontend?** Read the guide: [`skills/lobby-integration/SKILL.md`](skills/lobby-integration/SKILL.md).

## What it does

- **Private rooms** have a host who can add or remove bots, kick a player (10 s rejoin cooldown) and start the match. Others join by `room_id`.
- **Public rooms** have no host and are reached through quick match. They fill with humans, or with bots after a wait if the game allows bots.
- A **ready check** freezes the roster: every human must confirm before the room moves to `Starting`.
- The **game takes over by CPI**: `begin_match` → `Playing`, then `end_match` → `Ended` or `Cancelled`.
- **Rematch**: after a match ends, players move as a group into a new private room within a time window.
- **One player is in at most one room, hub-wide.** The program enforces this, not the client.
- **No on-chain timers.** Every waiting phase has a deadline, and anyone can call the permissionless `tick` to apply it. A Cloudflare cron Worker ([`worker/`](worker/)) does this as a backstop.
- **Rent**: rooms and player claims are funded by a `payer` signer (the player, or a gasless relayer on their behalf), and `close_room` / `close_claim` refund only that payer. The game owner funds the per-game config and matchmaker, which are never closed.

```
Waiting ──► ReadyCheck ──► Starting ──► Playing ──► Ended
   │             │             │            │
   └─────────────┴─────────────┴────────────┴──► Cancelled      (deadlines via `tick`, or the game)
```
Simplified: a ready check can also fall back to Waiting, and a solo host can go straight to Starting. Full diagram: [SKILL §2](skills/lobby-integration/SKILL.md#2-phases-and-deadlines).

## Repository layout

| Path | What |
|---|---|
| `programs/lobby/` | The Anchor program (21 instructions) and its LiteSVM tests (`programs/lobby/tests/`) |
| `tests/mock-game/` | **Test-only** example game that CPIs into lobby. Never deploy it. |
| `idls/` | The published IDL, its frozen snapshot, and the platform-core IDL used by `declare_program!` |
| `worker/` | Lobby Worker: Cloudflare cron that cranks `tick`, `close_room` and `close_claim` ([README](worker/README.md)) |
| `skills/lobby-integration/` | The integration guide (`SKILL.md`) |
| `specs/designs/` | Design notes (Thai). The relayer registration values are also given in English in the SKILL guide (§7). |

## Build and test

Requirements: Rust (see `rust-toolchain.toml`), anchor-cli 1.2.1, solana-cli (Agave) 4.x.

```bash
scripts/test.sh            # anchor build → copy IDL to idls/ → build mock-game → cargo test -p lobby
cd worker && npm ci && npm test && npm run typecheck
```

Tests run against the prebuilt `.so`, so always build first; `scripts/test.sh` does this for you.

## Versioning

The IDL is versioned and **0.1.0 is frozen**. Error codes (6000–6033), enum variants, account field order and instruction account order are append-only. Any breaking change will be announced with a new IDL version (and, if needed, a new program ID). Games CPI against a specific IDL version.

## Status

- Program deployed to devnet on 2026-10-11 (slot 509795184).
- Lobby Worker deployed on Cloudflare.
- Not yet audited and not on mainnet. A per-user rent cap is required in the relayer before players are onboarded.
