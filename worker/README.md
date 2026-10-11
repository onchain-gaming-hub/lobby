# lobby-worker

Cloudflare cron backstop for the permissionless lobby cranks — design: [`../specs/designs/04-worker.md`](../specs/designs/04-worker.md).
No privileges, no cache: each run reads the chain, simulates, and sends only when the program accepts **and** emits an event.

| cron | job | finds (`getProgramAccounts`, raw prefix only) |
|------|-----|-----------------------------------------------|
| `* * * * *` | `tick` | Rooms with a deadline that may have passed |
| `*/10 * * * *` | `close_room` | Ended/Cancelled rooms past `retention` → rent back to `rent_payer` |
| `17 * * * *` | `close_claim` (hourly · cap = `MAX_CANDIDATES_PER_RUN`) | claims idle ≥ 30 days (program checks liveness + kick) |

## Env

| name | where | what |
|------|-------|------|
| `RPC_URL` | secret | devnet RPC with `getProgramAccounts` · **maintainer fills** |
| `WORKER_SECRET_KEY` | secret | JSON array (64 bytes) of `~/.config/solana/lobby-worker-devnet.json` · wallet `AvoF2kV5iiuu2bk7BAho98Nu7MQEtbhn8udzxVzb8wRE` · fees only |
| `LOBBY_PROGRAM_ID` · `EXPECTED_GENESIS_HASH` · `MAX_CANDIDATES_PER_RUN` · `LOW_BALANCE_LAMPORTS` | `wrangler.toml` | program · devnet guard (refuses any other cluster) · per-run cap (default 5: free-plan CPU 10 ms/run · max 23: 50 subrequests) · low-balance warning (0.05 SOL) |

Local: `cp .dev.vars.example .dev.vars` → fill → `npm run dev` → `curl "http://localhost:8787/__scheduled?cron=*+*+*+*+*"`.
Deployed: `npx wrangler secret put RPC_URL` · `npx wrangler secret put WORKER_SECRET_KEY` · `npx wrangler deploy` — **deploy needs maintainer's approval**.

## Checks

`npm run typecheck` · `npm test` — layout/discriminators re-derived from `../idls/lobby.json` (0.1.0), PDAs vs `solana find-program-derived-address`, deadline filters vs `handle_tick`, crons vs `wrangler.toml`.

## Known limits (P7 gate)

- **Free plan**: 10 ms CPU per cron run → cap 5 · check `cpuTime` with `wrangler tail` on the first devnet runs · Workers Paid → raise toward 23.
- **5 cron triggers per account (free)** — this Worker uses 3; other hub Workers on the same account count too.
- `getProgramAccounts` is unpaged and `tick` decodes every Room each minute (CPU grows with room count, not bounded by the cap) — fine on devnet, measure before mainnet.
- A tick that races another cranker after our simulate lands as a paid no-op (5,000 lamports) — accepted.
- ≥ cap claims that stay `ClaimLive` forever would sit at the head of `close_claim`'s oldest-first queue — revisit with the indexer (6a).
