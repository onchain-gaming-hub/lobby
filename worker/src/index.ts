// Lobby Worker — cron backstop for the permissionless lobby cranks (specs/designs/04-worker.md).
// No privileges, no cache: the program decides every transition; this only finds candidates.
import { type Address, address, createKeyPairSignerFromBytes, type Instruction } from '@solana/kit';
import { fetchByDiscriminator, rpcFor, simulateThenSend, type Outcome } from './chain';
import { CLAIM, DISC, ROOM, decodeClaim, decodeRoom } from './layout';
import { claimMaybeClosable, closeClaimIx, closeRoomIx, roomClosable, tickIx, tickMaybeDue } from './plan';

export interface Env {
  /** secret — fill in `.dev.vars` (local) / `wrangler secret put RPC_URL` (deployed) */
  RPC_URL: string;
  /** secret — JSON array of 64 bytes (solana-keygen file content) */
  WORKER_SECRET_KEY: string;
  LOBBY_PROGRAM_ID: string;
  /** refuse to run on any other cluster (Golden Rule: devnet only) */
  EXPECTED_GENESIS_HASH: string;
  /** cap per run — free plan allows 50 subrequests per invocation (2 per candidate + 4 fixed) */
  MAX_CANDIDATES_PER_RUN: string;
  LOW_BALANCE_LAMPORTS: string;
}

type Job = 'tick' | 'close_room' | 'close_claim';

/** Must match `[triggers] crons` in wrangler.toml. */
export const CRON_JOBS: Record<string, Job> = {
  '* * * * *': 'tick',
  '*/10 * * * *': 'close_room',
  '17 3 * * *': 'close_claim',
};

export default {
  async scheduled(controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    const job = CRON_JOBS[controller.cron];
    if (!job) return log({ level: 'error', msg: 'unknown cron', cron: controller.cron });
    ctx.waitUntil(run(job, env));
  },
  async fetch(): Promise<Response> {
    return new Response('lobby-worker: cron only', { status: 404 });
  },
};

export async function run(job: Job, env: Env, now = Math.floor(Date.now() / 1000)): Promise<void> {
  if (!env.RPC_URL) return log({ level: 'error', job, msg: 'RPC_URL is empty — set it in .dev.vars / wrangler secret' });
  const rpc = rpcFor(env.RPC_URL);
  const genesis = await rpc.getGenesisHash().send();
  if (genesis !== env.EXPECTED_GENESIS_HASH) {
    return log({ level: 'error', job, msg: 'wrong cluster — refusing to run', genesis });
  }
  const program = address(env.LOBBY_PROGRAM_ID);
  const signer = await createKeyPairSignerFromBytes(Uint8Array.from(JSON.parse(env.WORKER_SECRET_KEY) as number[]));
  const balance = (await rpc.getBalance(signer.address).send()).value;
  if (balance < BigInt(env.LOW_BALANCE_LAMPORTS)) {
    log({ level: 'warn', job, msg: 'worker wallet low — top up', wallet: signer.address, lamports: balance.toString() });
  }

  const candidates = await findCandidates(job, rpc, program, now);
  const limit = Number(env.MAX_CANDIDATES_PER_RUN);
  const picked = candidates.slice(0, limit);
  const tally: Record<Outcome['kind'], number> = { sent: 0, noop: 0, rejected: 0 };
  if (picked.length > 0) {
    const blockhash = (await rpc.getLatestBlockhash({ commitment: 'confirmed' }).send()).value;
    for (const c of picked) {
      try {
        const out = await simulateThenSend(rpc, signer, blockhash, await c.ix());
        tally[out.kind]++;
        if (out.kind !== 'noop') log({ level: 'info', job, account: c.address, ...describe(out) });
      } catch (e) {
        tally.rejected++;
        log({ level: 'error', job, account: c.address, msg: String(e) });
      }
    }
  }
  log({ level: 'info', job, msg: 'done', candidates: candidates.length, deferred: candidates.length - picked.length, ...tally });
}

interface Candidate {
  address: Address;
  sortKey: number; // oldest deadline first
  ix: () => Promise<Instruction> | Instruction;
}

async function findCandidates(job: Job, rpc: ReturnType<typeof rpcFor>, program: Address, now: number): Promise<Candidate[]> {
  const out: Candidate[] = [];
  if (job === 'close_claim') {
    for (const a of await fetchByDiscriminator(rpc, program, DISC.playerClaim, CLAIM.sliceLen)) {
      const c = decodeClaim(a.data);
      if (c && claimMaybeClosable(c, now)) out.push({ address: a.address, sortKey: c.lastJoinedAt, ix: () => closeClaimIx(program, a.address, c) });
    }
  } else {
    for (const a of await fetchByDiscriminator(rpc, program, DISC.room, ROOM.sliceLen)) {
      const r = decodeRoom(a.data);
      if (!r) continue;
      if (job === 'tick' && tickMaybeDue(r, now)) out.push({ address: a.address, sortKey: r.phaseSince, ix: () => tickIx(program, a.address, r) });
      if (job === 'close_room' && roomClosable(r, now)) out.push({ address: a.address, sortKey: r.phaseSince, ix: () => closeRoomIx(program, a.address, r) });
    }
  }
  return out.sort((x, y) => x.sortKey - y.sortKey);
}

function describe(o: Outcome): Record<string, unknown> {
  if (o.kind === 'sent') return { outcome: 'sent', signature: o.signature };
  if (o.kind === 'rejected') return { outcome: 'rejected', code: o.code, err: JSON.stringify(o.err, (_, v) => (typeof v === 'bigint' ? v.toString() : v)) };
  return { outcome: 'noop' };
}

function log(o: Record<string, unknown>): void {
  console.log(JSON.stringify(o));
}
