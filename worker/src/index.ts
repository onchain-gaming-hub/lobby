// Lobby Worker — cron backstop for the permissionless lobby cranks (specs/designs/04-worker.md).
// No privileges, no cache: the program decides every transition; this only finds candidates.
import { type Address, address, createKeyPairSignerFromBytes, type Instruction, type KeyPairSigner } from '@solana/kit';
import { errorFields, fetchByDiscriminator, rpcFor, simulateThenSend, type Outcome } from './chain';
import { CLAIM, DISC, ROOM, decodeClaim, decodeRoom } from './layout';
import {
  EXPECTED_REJECTIONS,
  claimMaybeClosable,
  closeClaimIx,
  closeRoomIx,
  roomClosable,
  tickDueAt,
  tickIx,
} from './plan';

export interface Env {
  /** secret — fill in `.dev.vars` (local) / `wrangler secret put RPC_URL` (deployed) */
  RPC_URL: string;
  /** secret — JSON array of 64 bytes (solana-keygen file content) */
  WORKER_SECRET_KEY: string;
  LOBBY_PROGRAM_ID: string;
  /** refuse to run on any other cluster (Golden Rule: devnet only) */
  EXPECTED_GENESIS_HASH: string;
  /**
   * cap per run (clamped to MAX_CAP) — subrequests: 4 fixed + 2 per candidate (free plan: 50) ·
   * CPU: ~0.8 ms per candidate (sign + PDA) vs free plan 10 ms per cron run (P7 gate F1) → 5 on free
   */
  MAX_CANDIDATES_PER_RUN: string;
  LOW_BALANCE_LAMPORTS: string;
}

type Job = 'tick' | 'close_room' | 'close_claim';

/** Must match `[triggers] crons` in wrangler.toml (checked by test/plan.test.ts). */
export const CRON_JOBS: Record<string, Job> = {
  '* * * * *': 'tick',
  '*/10 * * * *': 'close_room',
  '17 * * * *': 'close_claim',
};

/** 1 signature · no priority fee (04 §4). */
const FEE_LAMPORTS = 5_000n;
/** a 0-byte system account must stay rent-exempt after paying fees (`solana rent 0 --url devnet` = 650,240). */
export const PAYER_RENT_FLOOR = 650_240n;
/** (50 subrequests − 4 fixed) / 2 per candidate */
export const MAX_CAP = 23;

export default {
  async scheduled(controller: ScheduledController, env: Env): Promise<void> {
    const job = CRON_JOBS[controller.cron];
    if (!job) return log({ level: 'error', msg: 'unknown cron', cron: controller.cron });
    await run(job, env); // awaited: the cron is reported finished only when the work is (P7 gate N5)
  },
  async fetch(): Promise<Response> {
    return new Response('lobby-worker: cron only', { status: 404 });
  },
};

/** Never throws — every failure ends as one structured log line. */
export async function run(job: Job, env: Env, now = Math.floor(Date.now() / 1000)): Promise<void> {
  try {
    await runJob(job, env, now);
  } catch (e) {
    log({ level: 'error', job, msg: 'run failed', ...errorFields(e) });
  }
}

async function runJob(job: Job, env: Env, now: number): Promise<void> {
  if (!env.RPC_URL) return log({ level: 'error', job, msg: 'RPC_URL is empty — set it in .dev.vars / wrangler secret' });
  if (!URL.canParse(env.RPC_URL)) return log({ level: 'error', job, msg: 'RPC_URL is not a valid URL' }); // never echo it
  const limit = Math.min(Number(env.MAX_CANDIDATES_PER_RUN), MAX_CAP);
  if (!Number.isInteger(limit) || limit < 1) return log({ level: 'error', job, msg: 'MAX_CANDIDATES_PER_RUN must be a positive integer' });
  const signer = await loadSigner(env.WORKER_SECRET_KEY);
  if (!signer) return log({ level: 'error', job, msg: 'WORKER_SECRET_KEY must be a JSON array of 64 bytes (solana-keygen file)' });

  const rpc = rpcFor(env.RPC_URL);
  const genesis = await rpc.getGenesisHash().send();
  if (genesis !== env.EXPECTED_GENESIS_HASH) {
    return log({ level: 'error', job, msg: 'wrong cluster — refusing to run', genesis });
  }
  const program = address(env.LOBBY_PROGRAM_ID);
  const balance = (await rpc.getBalance(signer.address).send()).value;
  if (balance < BigInt(env.LOW_BALANCE_LAMPORTS)) {
    log({ level: 'warn', job, msg: 'worker wallet low — top up', wallet: signer.address, lamports: balance.toString() });
  }

  const candidates = await findCandidates(job, rpc, program, now);
  const picked = candidates.slice(0, limit);
  const tally: Record<Outcome['kind'] | 'failed', number> = { sent: 0, noop: 0, rejected: 0, raced: 0, failed: 0 };
  if (picked.length > 0 && balance < PAYER_RENT_FLOOR + FEE_LAMPORTS * BigInt(picked.length)) {
    // with no fee money every simulate fails at fee-payer load and hides the real program answer
    return log({ level: 'error', job, msg: 'worker wallet cannot pay fees — skipped', wallet: signer.address, candidates: candidates.length });
  }
  if (picked.length > 0) {
    const blockhash = (await rpc.getLatestBlockhash({ commitment: 'confirmed' }).send()).value;
    for (const c of picked) {
      try {
        const out = await simulateThenSend(rpc, signer, blockhash, await c.ix());
        tally[out.kind]++;
        if (out.kind === 'sent') log({ level: 'info', job, account: c.address, outcome: 'sent', signature: out.signature });
        // raced = info whatever the code: the send carried the exact bytes the simulate just accepted, so a
        // program error at preflight means chain state moved in between (not a Worker bug)
        if (out.kind === 'raced') log({ level: 'info', job, account: c.address, outcome: 'raced', ...out.err });
        if (out.kind === 'rejected') {
          const expected = out.code !== null && EXPECTED_REJECTIONS.has(out.code);
          log({ level: expected ? 'info' : 'error', job, account: c.address, outcome: 'rejected', custom: out.code, err: stringify(out.err) });
        }
      } catch (e) {
        tally.failed++; // RPC / transport — not a program answer
        log({ level: 'error', job, account: c.address, outcome: 'failed', ...errorFields(e) });
      }
    }
  }
  log({ level: 'info', job, msg: 'done', candidates: candidates.length, deferred: candidates.length - picked.length, ...tally });
}

/** Parse errors are not rethrown: V8 quotes the input in JSON errors, which would leak the key into logs. */
async function loadSigner(secret: string): Promise<KeyPairSigner | null> {
  try {
    const bytes = JSON.parse(secret) as unknown;
    if (!Array.isArray(bytes) || bytes.length !== 64) return null;
    return await createKeyPairSignerFromBytes(Uint8Array.from(bytes as number[]));
  } catch {
    return null;
  }
}

interface Candidate {
  address: Address;
  dueAt: number; // earliest deadline first — real deadlines are never starved by later ones
  ix: () => Promise<Instruction> | Instruction;
}

async function findCandidates(job: Job, rpc: ReturnType<typeof rpcFor>, program: Address, now: number): Promise<Candidate[]> {
  const out: Candidate[] = [];
  if (job === 'close_claim') {
    for (const a of await fetchByDiscriminator(rpc, program, DISC.playerClaim, CLAIM.sliceLen)) {
      const c = decodeClaim(a.data);
      if (c && claimMaybeClosable(c, now)) out.push({ address: a.address, dueAt: c.lastJoinedAt, ix: () => closeClaimIx(program, a.address, c) });
    }
  } else if (job === 'tick') {
    // whole account (~351–491 B): seats are needed to know if a public room can really backfill
    for (const a of await fetchByDiscriminator(rpc, program, DISC.room)) {
      const r = decodeRoom(a.data);
      const at = r && tickDueAt(r);
      if (r && at !== null && at !== undefined && now >= at) out.push({ address: a.address, dueAt: at, ix: () => tickIx(program, a.address, r) });
    }
  } else {
    for (const a of await fetchByDiscriminator(rpc, program, DISC.room, ROOM.sliceLen)) {
      const r = decodeRoom(a.data);
      if (r && roomClosable(r, now)) out.push({ address: a.address, dueAt: r.phaseSince + r.ttl.retention, ix: () => closeRoomIx(program, a.address, r) });
    }
  }
  return out.sort((x, y) => x.dueAt - y.dueAt);
}

function stringify(v: unknown): string {
  return JSON.stringify(v, (_, x) => (typeof x === 'bigint' ? x.toString() : x));
}

function log(o: Record<string, unknown>): void {
  console.log(JSON.stringify(o));
}
