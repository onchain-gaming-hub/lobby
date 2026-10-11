# Relayer registration — lobby IDL 0.1.0

This file holds the values for registering lobby instructions as program actions on the hub's gasless relayer. The relayer pays fees and can front rent through a registered payer slot.

- Source: [`idls/lobby.json`](../idls/lobby.json) (0.1.0) and the CU measurements in `programs/lobby/tests/test_cu.rs` (LiteSVM 0.17 · Anchor 1.2.1).
- `programs/lobby/tests/test_registration.rs` checks every table row below against the IDL, so this file cannot drift silently.
- Program `5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu` · devnet · deploy slot 509795184.

## 1. Sponsored instructions

Column meanings:
- **accounts**: the number of accounts in IDL order. Unused optional accounts are passed as the lobby program ID, and they count.
- **payer_index**: the 0-based position of the payer slot.
- **CU**: sample measurements for 4-seat and 8-seat rooms, not fixed values. `find_program_address` cost varies by key.

| ix | discriminator (hex) | accounts | payer_index | max_data_len | max_rent_lamports | CU (n=4 / n=8) |
|----|---------------------|---------:|:-----------:|-------------:|------------------:|---------------:|
| `init_claim` | `305be43eaa120413` | 4 | **1** | 8 | **1,605,280** | ~7k–12k |
| `create_room` | `82a62002f778b235` | 9 | **1** | 8 | **3,144,520** | 22,007 / 22,539 |
| `quick_match_create` | `9fafa9182ee7c519` | 9 | **1** | 8 | **3,144,520** | ~19.3k–21.3k / ~19.8k–21.3k |
| `rematch_create` | `d2aed22228bf873c` | 11 | **1** | 8 | **3,144,520** | 31,678 / 33,754 · with a closed proof room 35,788 / 38,292 (program max) |
| `join_room` | `5fe8bc517c824e8b` | 4 | — | 8 | 0 | 14,248 / 15,816 |
| `quick_match_join` | `c850ecd7d412d35f` | 5 | — | 8 | 0 | 13,415 / 14,591 |
| `rematch_join` | `cfa102ca815f71ac` | 5 | — | 8 | 0 | 18,544 / 20,516 |
| `leave_room` | `f92aef80c014729c` | 3 | — | 8 | 0 | 6,983 / 8,159 · last human leaves a public room 9,969 / 11,145 |
| `ready` | `15753c496079d14b` | 2 | — | 8 | 0 | 7,001 / 8,265 |
| `leave_seat` | `6cf2b1062de3918e` | 2 | — | 8 | 0 | 6,605 / 7,781 |
| `add_bot` | `21fbf0a29f2c334a` | 2 | — | 9 | 0 | 6,571 / 7,687 |
| `remove_bot` | `31ef62d47862e1fc` | 2 | — | 9 | 0 | 6,569 / 7,685 |
| `kick_player` | `e6e1f4c13a0bc0c7` | 3 | — | 9 | 0 | 9,952 / 11,068 |
| `request_start` | `8c0ee8555d590199` | 2 | — | 8 | 0 | 7,123 / 8,427 |
| `tick` (crank) | `5c4f2c0865503f0f` | **3..3** (room, matchmaker or placeholder, then the user as trailing read-only signer — a fixed count so a client that forgets the placeholder fails loudly instead of looping on a retryable 3012) | — | 8 | 0 | no-op ~6.1k / ~7.3k · backfill 10,522 / 13,274 · ready timeout 10,503–10,536 / 13,187–13,236 |

**Notes:**
- `max_rent_lamports` for the room-creating instructions is the rent of an **8-seat** Room, because one lobby serves every game. A 4-seat room actually uses 2,433,320.
- Devnet rent was checked on 2026-10-11 and follows `(128 + bytes) × 5,080`. The registered values equal the rent exactly, with no margin. If the cluster's rent rate changes, re-derive them before registering.
- A new player sends `[init_claim, <enter-room ix>]` in one transaction: about 50k CU in total, with 2 signers (payer + player).
- No ComputeBudget instruction is needed (max ≈ 38k CU), and the priority fee is 0.

## 2. Not sponsored

| ix | reason |
|----|--------|
| `init_lobby` · `update_config` | Signed by the game owner's own wallet |
| `close_room` · `close_claim` | Run by cranks (e.g. the Lobby Worker) with their own wallets. Through the relayer they would be rejected, because the relayer key sits outside the registered payer slot. |
| `begin_match` · `end_match` | Reached only by CPI from a game program, so they are registered on the game's side. If a game marks lobby's 6010 / 6026 or a CPI-level 3012 as retryable, those failures would loop (see the integration guide §5.3). |

## 3. Project values

| value | requested |
|-------|-----------|
| `retryable_errors` | **6000** `StaleMatchmaker` · **6001** `StaleRematch` · **6002** `RoomFull` · **6003** `NotWaiting` · **3012** `AccountNotInitialized` (the room was closed meanwhile, or the player has no claim yet — the client must then send `init_claim`) |
| `cooldown_ms` per wallet | **≤ 1,000** (join → ready → tick follow each other quickly) |
| `max_instructions` | 4 (normal flows use 1–2) |
| `max_cu` / priority fee | default / 0 |
| `min_balance_lamports` | ≥ 4 × 3,144,520 + 1,000,000 = **13,578,080** |
| `project_daily_rent_budget` | **must be > 0**, otherwise every payer-slot instruction is refused. Estimate as new players/day × 1,605,280 + rooms created/day × 3,144,520. |
| per-user daily rent cap | **Required before onboarding players**, and not yet available in the relayer. It bounds one account looping create → leave. The program's 60 s create cooldown slows a single player but does not stop many accounts, and account-level abuse remains the relayer's job. |
| `deploy_slot` | **509795184** (devnet) |
