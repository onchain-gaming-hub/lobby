---
name: lobby-integration
description: How to integrate a game program or a frontend with the blockchain-games-hub lobby program on Solana (rooms, quick match, ready check, rematch, game CPI, permissionless cranks, rent and relayer). Use when wiring a game's begin/end match CPI to lobby, building lobby transactions in a frontend, deriving lobby PDAs, reading Room / PlayerClaim state, handling lobby errors and retries, or registering lobby instructions with the gasless relayer.
---

# Lobby Integration Guide

> Program `lobby` · IDL **0.1.0** (frozen) · Anchor 1.2.1 · **devnet** `5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu`
> Source of truth: `idls/lobby.json` and `programs/lobby/src/`. If this guide and the IDL disagree, the IDL wins. Please report the mismatch.
> Repository: https://github.com/onchain-gaming-hub/lobby · paths in this guide are relative to its root.

## 0. Mental model

- **One lobby program serves every game.** Each game gets its own `LobbyConfig` and `Matchmaker`, and its rooms are keyed by `(game_program, room_id)`.
- **The game is registered in platform-core first.** That registration gives the game a `GameEntry` PDA with an `owner` and a `game_authority`: the game's PDA signer, seed `[b"game_authority"]` under the game program. Lobby trusts only that registration.
- **The game calls lobby; lobby never calls the game.** Lobby can still cancel a room on its own when a deadline passes, so a game must check the room's phase rather than wait to be told (§5.3).
- **No timers on chain.** Every waiting phase stores an anchor timestamp and a TTL. Once the deadline passes, anyone may call `tick` to apply it. Until then, normal actions still work, so a late `ready()` that lands before the `tick` still counts.
- **One player, one room, hub-wide.** Each player has one `PlayerClaim` PDA (`[b"claim", player]`). It is never "released": a claim is *live* only while the room it points at exists, isn't finished, and still seats that player as an active human. Leaving, being kicked, the room ending or the room being closed all free the player automatically.
- **Rooms and claims are funded by a `payer` signer**, which can be the player or a gasless relayer (§7). The payer is stored as `rent_payer`, and `close_room` / `close_claim` refund only that address. `LobbyConfig` and `Matchmaker` are paid by the game owner and are never closed.

## 1. Accounts and PDAs

| Account | Seeds (program = lobby) | Created by | Size | Notes |
|---|---|---|---|---|
| `LobbyConfig` | `[b"lobby_config", game_program]` | `init_lobby` (game owner) | 109 B | Per-game rules. `update_config` affects **new** rooms only. |
| `Matchmaker` | `[b"matchmaker", game_program]` | `init_lobby` | 43 B | `next_room_id` starts at 1 and is never reused. `open_public_room` is the quick-match pointer. Never closed. |
| `Room` | `[b"room", game_program, room_id.to_le_bytes()]` | `create_room` / `quick_match_create` / `rematch_create` | `211 + 35 × max_seats` B (351 B at 4 seats, 491 B at 8) | Snapshot of the config taken at creation. |
| `PlayerClaim` | `[b"claim", player]` | `init_claim` | 188 B | One per player across **all** games. |
| `GameEntry` (platform-core) | `[b"game", game_program]` under platform-core | platform-core | — | Must be `Active` to create rooms. |

```ts
// @solana/kit — the same derivation the Lobby Worker uses
import { address, getProgramDerivedAddress, getAddressEncoder, getU64Encoder } from '@solana/kit';
const LOBBY = address('5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu');
const enc = getAddressEncoder();
const roomPda = async (game: string, roomId: bigint) =>
  (await getProgramDerivedAddress({ programAddress: LOBBY, seeds: ['room', enc.encode(address(game)), getU64Encoder().encode(roomId)] }))[0];
const claimPda = async (player: string) =>
  (await getProgramDerivedAddress({ programAddress: LOBBY, seeds: ['claim', enc.encode(address(player))] }))[0];
const matchmakerPda = async (game: string) =>
  (await getProgramDerivedAddress({ programAddress: LOBBY, seeds: ['matchmaker', enc.encode(address(game))] }))[0];
```

### Room fields that matter to clients

`phase` · `end_reason` · `is_public` · `host: Option<Pubkey>` · `seats: Vec<Seat>` (length = `max_seats` for the room's whole life; seats are never re-packed) · `rematch_room_id: Option<u64>` · `ttl` (the room's snapshot of the config TTLs) · `phase_since` · `last_human_activity_at` · `rent_payer`.

`Seat = { kind: Empty | Human | Bot, player, ready, left }`. An *active human* is `kind == Human && !left`.

Raw offsets for `getProgramAccounts` filters are stable only **up to the first `Option`**:
- Room: `phase` @ 82, `rent_payer` @ 10, `game_program` @ 42, and the fixed prefix ends at byte 133.
- PlayerClaim: `player` @ 42, `last_joined_at` @ 74, `current` @ 82.

Decode anything after those offsets with the IDL.

## 2. Phases and deadlines

```mermaid
stateDiagram-v2
    [*] --> Waiting: create_room / quick_match_create / rematch_create
    Waiting --> ReadyCheck: public room fills · public tick after wait (bots fill) · private request_start
    Waiting --> Starting: private request_start when the host is the only human
    Waiting --> Cancelled: last human leaves (NoHumans) · tick after lobby TTL (LobbyExpired)
    ReadyCheck --> Starting: last human calls ready()
    ReadyCheck --> Waiting: tick after ready TTL, removes humans who did not confirm
    ReadyCheck --> Cancelled: tick after ready TTL, no human left (NoHumans)
    Starting --> Playing: game CPI begin_match
    Starting --> Cancelled: game CPI end_match(Cancelled) · tick after start TTL (StartExpired)
    Playing --> Ended: game CPI end_match(Completed)
    Playing --> Cancelled: end_match(Cancelled) · tick after max_game TTL (GameExpired) · every human leave_seat (Abandoned)
    Ended --> [*]: close_room after retention
    Cancelled --> [*]: close_room after retention
```

| Phase | Deadline applied by `tick` | Anchor timestamp |
|---|---|---|
| Waiting | `ttl.lobby` → Cancelled(LobbyExpired). Public rooms with bots allowed: after `ttl.wait`, empty seats fill with bots → ReadyCheck (needs `humans ≥ min_humans`). | `last_human_activity_at` (lobby) · `phase_since` (wait) |
| ReadyCheck | `ttl.ready` → humans who didn't confirm are removed. Public rooms also drop their bots. → back to Waiting, or Cancelled if no human is left. | `phase_since` |
| Starting | `ttl.start` → Cancelled(StartExpired) | `phase_since` |
| Playing | `ttl.max_game` → Cancelled(GameExpired) | `phase_since` |
| Ended / Cancelled | `close_room` becomes possible after `ttl.retention` | `phase_since` |

**Program ceilings on game config:**

| Ceiling | Value |
|---|---|
| `ready` | ≤ 120 s |
| `start` | ≤ 300 s |
| `lobby` | ≤ 21 600 s |
| `max_game` | ≤ 86 400 s |
| `retention` | ≤ 86 400 s |
| Seats | `2 ≤ max_seats ≤ 8` |

`InvalidConfig` also rejects:
- any TTL of 0
- `min_humans` not satisfying `1 ≤ min_humans ≤ min_seats ≤ max_seats`
- `wait ≥ lobby`
- `rematch_window < 60`
- `retention < rematch_window`

## 3. Instructions

Caller types:
- **P**: the player signs.
- **H**: the host of a private room signs.
- **A**: anyone may call it.
- **G**: the game's `game_authority` PDA signs via CPI.
- **O**: the game owner (`GameEntry.owner`) signs.

**$** marks the instructions that take a separate `payer` signer and always create an account (rent).

| Instruction | Who | Phase | Effect |
|---|---|---|---|
| `init_lobby(params)` | O | — | Creates the game's `LobbyConfig` and `Matchmaker`. `GameEntry` must be Active. The owner pays with their own wallet. |
| `update_config(params)` | O | — | Updates `LobbyConfig`. Existing rooms keep their snapshot. |
| `init_claim()` $ | P | — | Creates the player's `PlayerClaim`. Done once per player, hub-wide. |
| `create_room()` $ | P | — | New private room; the caller is host in seat 0. Needs `private_enabled`, `GameEntry` Active, and the create cooldown. |
| `join_room()` | P | Waiting | Joins a **private** room in the lowest free seat. A public room → `ModeDisabled`. |
| `quick_match_join()` | P | Waiting | Joins the room in `matchmaker.open_public_room`. If that fills the room → ReadyCheck. |
| `quick_match_create()` $ | P | — | New public room when the pointer is empty. Needs `public_enabled` and the create cooldown. |
| `leave_room()` | P | **Waiting only** | Frees the seat. If the host leaves, the next human becomes host. If the last human leaves → Cancelled. |
| `ready()` | P | ReadyCheck | Confirms. When the last human confirms → Starting. |
| `leave_seat()` | P | Playing | Marks the seat `left`; the game still sees who dropped. If every human leaves → Cancelled(Abandoned). |
| `add_bot(seat)` / `remove_bot(seat)` | H | Waiting | Private rooms only. `add_bot` needs `bot_fill`. |
| `kick_player(seat)` | H | Waiting | Empties the seat. The victim can't rejoin **this room** for 10 s. |
| `request_start()` | H | Waiting | Needs `occupied ≥ min_seats` and `humans ≥ min_humans` → ReadyCheck, with the host already ready. |
| `rematch_join()` | P | old room Ended | Joins the rematch room the old room points at. That room must be in Waiting with a free seat (else `StaleRematch`). |
| `rematch_create()` $ | P | old room Ended | Creates a new **private** rematch room with the caller as host, and points the old room at it. Has the create cooldown. The host then calls `request_start` as usual. |
| `tick()` | A | any | Applies the first deadline that is due. If nothing is due it is a **successful no-op**. |
| `close_room()` | A | Ended/Cancelled | After `retention`, closes the room and refunds `rent_payer` (who need not sign). |
| `close_claim()` | A | — | Needs the claim idle ≥ **30 days**, not live, and no running kick cooldown. Closes it and refunds `rent_payer`. |
| `begin_match()` | G | Starting | → Playing |
| `end_match(result)` | G | Playing (Completed or Cancelled) · Starting (Cancelled only) | → Ended or Cancelled(GameCancelled) |

**Rematch conditions** (both rematch instructions): the old room is `Ended` (not Cancelled) · `rematch_enabled` was on when it was created (else `ModeDisabled`) · `now < phase_since + ttl.rematch_window` (else `RematchClosed`) · the caller sat in it as a human, even if they later left (else `NotInRoster`). Accepted limitations: while the rematch room stays joinable, nobody can create a replacement (`rematch_create` → `StaleRematch`) until `ttl.rematch_window` closes, so an idle rematch host can hold the group; players can always leave and quick-match or create their own room. A group can also split into two rooms if the first rematch room becomes unjoinable.

**Limits:**

| Limit | Value |
|---|---|
| Create cooldown | 60 s per player, across all three create instructions |
| Kick cooldown | 10 s |
| Claim idle before `close_claim` | 30 days |
| Max seats | 8 |

### 3.1 Account lists (order = IDL order)

Mark `?` = optional. When you don't use an optional account, **pass the lobby program ID** in its slot, read-only. Never drop the slot: later accounts would shift into it.

```
init_claim          player(s) · payer(s,w) · claim(w) · system_program
create_room         player(s) · payer(s,w) · game_entry · config · matchmaker(w) · claim(w) · prev_room? · new_room(w) · system_program
quick_match_create  player(s) · payer(s,w) · game_entry · config · matchmaker(w) · claim(w) · prev_room? · new_room(w) · system_program
quick_match_join    player(s) · matchmaker(w) · room(w) · claim(w) · prev_room?
join_room           player(s) · room(w) · claim(w) · prev_room?
leave_room          player(s) · room(w) · matchmaker?(w)        ← required for public rooms
ready               player(s) · room(w)
leave_seat          player(s) · room(w)
add_bot / remove_bot / request_start   host(s) · room(w)
kick_player         host(s) · room(w) · victim_claim(w)          ← claim PDA of the player in `seat`
rematch_join        player(s) · old_room · claim(w) · prev_room? · rematch_room(w)
rematch_create      player(s) · payer(s,w) · game_entry · config · matchmaker(w) · old_room(w) · claim(w) · prev_room? · rematch_room? · new_room(w) · system_program
tick                room(w) · matchmaker?(w)                     ← matchmaker for public rooms in Waiting / ReadyCheck
close_room          room(w) · rent_payer(w)
close_claim         claim(w) · rent_payer(w) · prev_room?
begin_match / end_match   room(w) · game_entry · game_authority(s)
init_lobby          owner(s,w) · game_program · game_entry · config(w) · matchmaker(w) · system_program
update_config       owner(s) · game_entry · config(w)
```

- **`prev_room`** is the room PDA that `claim.current` points at. Pass it whenever `claim.current` is `Some`, even when it is the room you are entering, or the old room in a rematch. Pass the program ID when `claim.current` is `None`. Lobby reads it to decide whether the claim is still live.
- **`new_room`** is `roomPda(game, matchmaker.next_room_id)`. Read the Matchmaker just before you build the tx.
- **`rematch_room` in `rematch_create`** is the room PDA of `old_room.rematch_room_id` when that is `Some`. Pass the program ID when it is `None`.

## 4. Frontend recipes

**First action of a new player.** If the claim account doesn't exist yet, send `[init_claim, <enter-room ix>]` in **one** transaction. A claim that already exists makes `init_claim` fail, so send it only when the claim is missing.

**Private room:** `create_room`. Share the `room_id` you derived `new_room` from (also in the `RoomCreated` event). Don't re-read the Matchmaker counter afterwards: it has already moved on. Friends call `join_room`. The host calls `request_start`, and everyone calls `ready()`.

**Quick match:**
```
read Matchmaker
  open_public_room = Some(id) → quick_match_join(room = roomPda(game, id))
  None                        → quick_match_create(new_room = roomPda(game, next_room_id))
on 6000 StaleMatchmaker → re-read the Matchmaker and rebuild (another player moved first)
```

**Rematch** (old room is `Ended`, inside `ttl.rematch_window`):
```
old_room.rematch_room_id = Some(id) → rematch_join(rematch_room = roomPda(game, id))
None                                → rematch_create
on 6001 StaleRematch → re-read old_room: join the new pointer, or create if that room is no longer joinable
```

**Resume after reload:** there is no instruction for this. Read `PlayerClaim`. If `current` is `Some`, read that Room and run the liveness check client-side: the room exists, its phase is not terminal, and the player is in a seat with `Human && !left`.

**Deadlines:** show countdowns using the deadline formulas in §2. When a deadline passes, any client in the room should send `tick()`. The Lobby Worker is a best-effort backstop with limited throughput (a few rooms per minute across all games), so clients should still tick. Remember that `Clock` on chain can drift a few seconds from wall time.

**Ready-timeout griefing (accepted risk):** a player removed by the ready timeout can quick-match straight back into the same room. Frontends should disable quick match for **60 s** after the player sees their own `SeatChanged { cause: ReadyTimeout }`.

## 5. Game integration (CPI)

### 5.1 Prerequisites
1. **Register the game in platform-core.** Platform-core's `register_game` instruction is signed by a hub account holding the GameRegistrar or Admin role, co-signed by the future game owner. The game program must already be deployed (upgradeable loader). Registration records `GameEntry.owner` and `game_authority`, the game's PDA from seed `[b"game_authority"]`. Ask the hub admins to register your game. Registration sets the entry `Active`; lobby rejects `init_lobby` and room creation while it is not (e.g. Suspended).
2. **The owner calls `init_lobby(params)` once**, from their own wallet (not relayed). `ConfigParams`:

   | Field | Meaning |
   |---|---|
   | `max_seats` | Seats per room, 2–8 |
   | `min_seats` | Occupied seats (humans + bots) needed for `request_start` |
   | `min_humans` | Humans needed to start, and for a public room to be bot-filled |
   | `public_enabled` / `private_enabled` | Which create instructions are allowed (joining existing rooms is never blocked) |
   | `bot_fill` | Hosts may add bots, and public rooms fill with bots after `ttl.wait`. Set `false` unless your game can play bots. |
   | `rematch_enabled` | Rooms created from now on allow rematch |
   | `ttl` | `wait`, `ready`, `lobby`, `start`, `max_game`, `rematch_window`, `retention` in seconds (limits in §2) |

3. Copy [`idls/lobby.json`](../../idls/lobby.json) into your program and use `declare_program!(lobby)` (Anchor 1.2.1).

### Lifecycle of one match
1. Players gather in a room. The last `ready()` (or `request_start` when the host is the only human) moves it to **Starting**.
2. Your frontend sees `Starting` (from the Room account or a `PhaseChanged` event) and any player sends **your** start-game instruction within `ttl.start` (≤ 300 s), otherwise `tick` cancels the room (StartExpired).
3. Your start-game instruction reads the roster from `room.seats`, creates your match account storing `room_id` (and anything else you need), and CPIs `begin_match` → Playing.
4. Your move instructions run while the room is Playing (rules in §5.3).
5. Your finish instruction CPIs `end_match(Completed)` → Ended, or `end_match(Cancelled)` → Cancelled.

### 5.2 Begin and end a match (CPI)

`begin_match` / `end_match` do not check that `GameEntry` is Active: a suspended game can still start and finish matches in rooms that already exist. Only `init_lobby` and room creation require Active.

```rust
declare_program!(lobby);
pub const GAME_AUTHORITY_SEED: &[u8] = b"game_authority";

// inside your start-game instruction (room is in Starting):
let seeds: &[&[&[u8]]] = &[&[GAME_AUTHORITY_SEED, &[ctx.bumps.game_authority]]];
lobby::cpi::begin_match(CpiContext::new_with_signer(
    ctx.accounts.lobby_program.key(),
    lobby::cpi::accounts::BeginMatch {
        room: ctx.accounts.room.to_account_info(),               // writable
        game_entry: ctx.accounts.game_entry.to_account_info(),   // platform-core GameEntry of THIS game
        game_authority: ctx.accounts.game_authority.to_account_info(),
    },
    seeds,
))?;
// end: lobby::cpi::end_match(ctx, lobby::types::MatchResult::Completed | Cancelled)
```

A minimal, tested CPI example lives in [`tests/mock-game/src/lib.rs`](../../tests/mock-game/src/lib.rs). It is test-only (never deploy it) and it **omits the §5.3 checks** — your game must add them. Read the roster from `room.seats` in the same transaction as `begin_match`.

### 5.3 Rules every game must follow
1. **Every move instruction takes the `Room` read-only, requires `room.phase == Playing`, and binds the room to the match:** `require!(room.game_program == crate::ID && room.room_id == match.room_id)`.
   - `Account<lobby::Room>` checks only owner and discriminator. Without the binding, anyone could pass another match's Playing room.
   - If you compare PDAs instead, derive under `lobby::ID` (`seeds::program = lobby::ID`).
2. **Lobby can cancel a room without calling you.** That happens through `tick` (StartExpired or GameExpired) or when every human calls `leave_seat`. Rule 1 is how your game notices.
3. **Read `room.phase` before calling `end_match`.** If lobby has already cancelled the room (`Cancelled`), skip `end_match` and take your own cancel/refund path: calling it would revert the whole transaction with 6010. Otherwise settle in the same transaction as `end_match` where you can. Don't rely on the `Room` after `retention`, because anyone can close it then.
4. **Errors that come back through CPI:**

   | Error | Meaning | Action |
   |---|---|---|
   | 6010 `WrongPhase` | Lobby rejected the call for the room's phase | Never mark retryable in your relayer registration |
   | 6026 `BadGameAuthority` | Lobby rejected the signer | Never mark retryable |
   | 3012 | The room is already closed, so the failure is final | Never retry |
   | Anchor 2xxx / 3xxx | Your game passed the wrong accounts | Fix the call |

   Give your own errors an offset so they don't collide with lobby's 6000–6033: `#[error_code(offset = 6100)]`.
5. Lobby only marks a seat as `Bot`; it never decides who moves for it. Your game (or its off-chain dealer) plays bots. If your game cannot play bots, set `bot_fill = false`.

## 6. Errors

Anchor custom errors 6000–6033. They are append-only, so the numbers never change.

**Possible races** — for player-signed instructions, re-read the state, then decide: retry, switch instruction (e.g. `quick_match_create` ↔ `quick_match_join`), or show the new state. Retrying unchanged is pointless when the state has really moved on (e.g. `NotWaiting` from `leave_room` after the match started). Inside a game CPI, 3012 is final (§5.3).

| Code | Name | Cause |
|---|---|---|
| 6000 | `StaleMatchmaker` | Another player changed the pointer or counter first |
| 6001 | `StaleRematch` | The rematch pointer changed, or the rematch room isn't joinable. From `rematch_create` it can also mean the proof room wasn't passed. |
| 6002 | `RoomFull` | Room is full |
| 6003 | `NotWaiting` | Room is no longer in Waiting |
| 3012 | Anchor `AccountNotInitialized` | The room was closed meanwhile, **or** the player has no claim. In the second case, send `init_claim` first. |

**Not retryable:**

| Code | Name | Code | Name | Code | Name |
|---|---|---|---|---|---|
| 6004 | PrevRoomRequired | 6014 | NotAHuman | 6024 | MatchmakerRequired |
| 6005 | PrevRoomMismatch | 6015 | CannotKickSelf | 6025 | RetentionNotReached |
| 6006 | ClaimLive | 6016 | SeatNotEmpty | 6026 | BadGameAuthority |
| 6007 | AlreadyInRoom | 6017 | SeatNotBot | 6027 | InvalidConfig |
| 6008 | GameNotActive | 6018 | BotsDisabled | 6028 | NotGameOwner |
| 6009 | ModeDisabled | 6019 | KickCooldown | 6029 | InvalidSeat |
| 6010 | WrongPhase | 6020 | NotEnoughSeats | 6030 | MathOverflow |
| 6011 | NotInRoom | 6021 | NotEnoughHumans | 6031 | WrongRoom |
| 6012 | NotHost | 6022 | RematchClosed | 6032 | ClaimNotIdle |
| 6013 | PublicNoHostControl | 6023 | NotInRoster | 6033 | CreateCooldown |

## 7. Rent, the relayer and cranks

A *crank* is a permissionless maintenance transaction (`tick`, `close_room`, `close_claim`): anyone may send it, and the program decides whether anything happens.

- **Rent on devnet** is `(128 + bytes) × 5 080` lamports:

  | Account | Size | Rent (lamports) |
  |---|---|---|
  | PlayerClaim | 188 B | 1 605 280 |
  | Room, 4 seats | 351 B | 2 433 320 |
  | Room, 8 seats | 491 B | 3 144 520 |

- Every close refunds the stored `rent_payer`, and only that address. `rent_payer` is written once when the account is created.
- **Gasless relayer:**
  - Only the four $ instructions take a payer slot, at index 1.
  - Player-facing instructions can be fee-sponsored (table below). `init_lobby` / `update_config` are signed by the game owner's own wallet, and `close_room` / `close_claim` are run by cranks with their own wallets: these four are **not** sponsored. `begin_match` / `end_match` are sponsored, if at all, as part of the game's own instruction.
  - When sending `tick` through the relayer, append the user as a trailing **read-only signer**. `tick` ignores `remaining_accounts`.
  - Registration values (full version with CU samples and project values: [`docs/relayer-registration.md`](../../docs/relayer-registration.md)). Data length is 8 (discriminator only) or 9 (`+ seat: u8`). Max CU measured ≈ 38k, so no ComputeBudget instruction is needed. Retryable codes: 6000–6003 and 3012.

    | Instruction | Discriminator (hex) | Accounts | Payer index | `max_rent_lamports` |
    |---|---|---|---|---|
    | `init_claim` | `305be43eaa120413` | 4 | 1 | 1 605 280 |
    | `create_room` | `82a62002f778b235` | 9 | 1 | 3 144 520 |
    | `quick_match_create` | `9fafa9182ee7c519` | 9 | 1 | 3 144 520 |
    | `rematch_create` | `d2aed22228bf873c` | 11 | 1 | 3 144 520 |
    | `join_room` | `5fe8bc517c824e8b` | 4 | — | 0 |
    | `quick_match_join` | `c850ecd7d412d35f` | 5 | — | 0 |
    | `rematch_join` | `cfa102ca815f71ac` | 5 | — | 0 |
    | `leave_room` | `f92aef80c014729c` | 3 | — | 0 |
    | `ready` | `15753c496079d14b` | 2 | — | 0 |
    | `leave_seat` | `6cf2b1062de3918e` | 2 | — | 0 |
    | `add_bot` | `21fbf0a29f2c334a` | 2 | — | 0 |
    | `remove_bot` | `31ef62d47862e1fc` | 2 | — | 0 |
    | `kick_player` | `e6e1f4c13a0bc0c7` | 3 | — | 0 |
    | `request_start` | `8c0ee8555d590199` | 2 | — | 0 |
    | `tick` | `5c4f2c0865503f0f` | 3 (room, matchmaker or placeholder, user as trailing read-only signer) | — | 0 |

    `max_rent_lamports` for room creation is the 8-seat ceiling, because one lobby serves every game.
  - The relayer's rent budget is **not** refunded when an account closes. That is why the program has the create cooldown and the 30-day `close_claim` rule.
- **Lobby Worker** (`worker/`) runs on its own fee-only wallet, which has no privilege in the program:
  - `tick` every minute
  - `close_room` every 10 minutes
  - `close_claim` hourly

  It simulates each transaction first and sends only when the transaction changes state. Anyone else may run the same cranks.

## 8. Events (for indexers)

All events are emitted with `emit!`, which writes `Program data:` logs:

| Event | When |
|---|---|
| `LobbyInitialized` · `LobbyConfigUpdated` | Game owner sets up or changes the config |
| `ClaimCreated` · `ClaimClosed` | A claim is created or closed |
| `RoomCreated { room_id, is_public, host, rent_payer, rematch_of }` | A room is created |
| `SeatChanged { seat, kind, player, cause }` | A seat changes. `cause` is Join, Leave, Kick, AddBot, RemoveBot, ReadyTimeout, Backfill or Purge, and the event carries the post-state. |
| `PlayerReady` · `PlayerLeftSeat` · `HostChanged` | Player confirms, leaves during play, or the host changes |
| `PhaseChanged { from, to, end_reason }` | Every phase transition. Any state change made by `tick` emits at least one event. |
| `MatchmakerPointer { open_public_room }` | The quick-match pointer changes |
| `RoomClosed { refunded_to, lamports }` | A room is closed and its rent refunded |

Events carry no TTLs. Read them from the `Room` account.

## 9. Checklist

- [ ] Game registered in platform-core and `init_lobby` called by the owner
- [ ] `declare_program!(lobby)` built from `idls/lobby.json` 0.1.0
- [ ] Every move instruction checks `Playing` and binds `Room` to the match (§5.3)
- [ ] Game error codes offset above 6033
- [ ] Frontend:
  - [ ] always passes `prev_room`, with the program ID placeholder when there is none
  - [ ] uses `[init_claim, …]` for new players
  - [ ] on 6000–6003 and 3012, re-reads state before retrying or switching instruction
- [ ] Frontend shows countdowns and sends `tick` when a deadline passes, with the user as trailing read-only signer if relayed
- [ ] Frontend disables quick match for 60 s after the player is removed by ReadyTimeout
- [ ] Relayer registration copied from `06-relayer-registration.md`
