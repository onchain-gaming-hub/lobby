// The main run() loop against a mocked JSON-RPC (P7 gate N9): cap, deadline order, noop ⇒ no send, balance skip.
import { generateKeyPairSync } from 'node:crypto';
import { address, getAddressEncoder, getBase58Decoder } from '@solana/kit';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type Env, PAYER_RENT_FLOOR, run } from '../src/index';
import { DISC, Phase, ROOM } from '../src/layout';
import { roomPda } from '../src/plan';

const PROGRAM = address('5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu');
const GAME = Uint8Array.from(getAddressEncoder().encode(address('5PjkqfpUic5nNYr5aN6nbGdHLSYQEEzK8wr7PDPv2aRw')));
const NOW = 100_000;

function keygenJson(): string {
  const jwk = generateKeyPairSync('ed25519').privateKey.export({ format: 'jwk' });
  return JSON.stringify([...Buffer.from(jwk.d!, 'base64url'), ...Buffer.from(jwk.x!, 'base64url')]);
}

/** Whole private Room account in ReadyCheck (no host · no rematch · 4 empty seats). */
function readyCheckRoom(roomId: bigint, phaseSince: number): string {
  const d = new Uint8Array(ROOM.sliceLen + 1 + 1 + 4 + 4 * 35 + 32);
  const v = new DataView(d.buffer);
  d.set(DISC.room);
  d.set(GAME, ROOM.gameProgram);
  v.setBigUint64(ROOM.roomId, roomId, true);
  d[ROOM.phase] = Phase.ReadyCheck;
  v.setUint32(ROOM.ttl + 4, 30, true); // ready
  v.setBigInt64(ROOM.phaseSince, BigInt(phaseSince), true);
  v.setBigInt64(ROOM.lastHumanActivityAt, BigInt(phaseSince), true);
  v.setUint32(ROOM.sliceLen + 2, 4, true);
  return Buffer.from(d).toString('base64');
}

interface Mock {
  balance: number;
  rooms: { id: bigint; phaseSince: number }[];
  /** room ids whose simulation emits an event (others are no-ops) */
  emits: Set<bigint>;
  sendStatus?: number;
  sendPreflightFails?: boolean;
}

async function mockRpc(m: Mock) {
  const calls: { method: string; params: unknown[] }[] = [];
  const pdaToId = new Map<string, bigint>();
  const accounts = await Promise.all(
    m.rooms.map(async (r) => {
      const pda = await roomPda(PROGRAM, GAME, r.id);
      pdaToId.set(pda, r.id);
      return { pubkey: pda, account: { data: [readyCheckRoom(r.id, r.phaseSince), 'base64'], executable: false, lamports: 1, owner: PROGRAM, rentEpoch: 0, space: 0 } };
    }),
  );
  const roomOfTx = (b64: string): bigint => {
    const bytes = Buffer.from(b64, 'base64');
    for (const [pda, id] of pdaToId) if (bytes.includes(Buffer.from(getAddressEncoder().encode(address(pda))))) return id;
    throw new Error('tx without a known room');
  };
  const ctx = { slot: 1 };
  vi.stubGlobal('fetch', async (_url: string, init: RequestInit) => {
    const req = JSON.parse(String(init.body)) as { id: number; method: string; params: unknown[] };
    calls.push({ method: req.method, params: req.params });
    if (req.method === 'sendTransaction' && m.sendStatus) return new Response('', { status: m.sendStatus });
    if (req.method === 'sendTransaction' && m.sendPreflightFails) {
      const error = { code: -32002, message: 'Transaction simulation failed', data: { err: { InstructionError: [0, { Custom: 3012 }] }, logs: [], accounts: null, unitsConsumed: 0, returnData: null } };
      return new Response(JSON.stringify({ jsonrpc: '2.0', id: req.id, error }), { headers: { 'content-type': 'application/json' } });
    }
    const result = (() => {
      switch (req.method) {
        case 'getGenesisHash':
          return 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG';
        case 'getBalance':
          return { context: ctx, value: m.balance };
        case 'getProgramAccounts':
          return accounts;
        case 'getLatestBlockhash':
          return { context: ctx, value: { blockhash: getBase58Decoder().decode(new Uint8Array(32).fill(3)), lastValidBlockHeight: 1000 } };
        case 'simulateTransaction': {
          const id = roomOfTx(req.params[0] as string);
          const logs = m.emits.has(id) ? ['Program data: spJD3tAk1ds='] : ['Program log: Instruction: Tick'];
          return { context: ctx, value: { err: null, logs, accounts: null, unitsConsumed: 7000, returnData: null } };
        }
        case 'sendTransaction':
          return getBase58Decoder().decode(new Uint8Array(64).fill(9));
        default:
          throw new Error(`unexpected ${req.method}`);
      }
    })();
    return new Response(JSON.stringify({ jsonrpc: '2.0', id: req.id, result }), { headers: { 'content-type': 'application/json' } });
  });
  const sent = () => calls.filter((c) => c.method === 'sendTransaction').map((c) => roomOfTx(c.params[0] as string));
  const simulated = () => calls.filter((c) => c.method === 'simulateTransaction').map((c) => roomOfTx(c.params[0] as string));
  return { calls, sent, simulated };
}

const env = (o: Partial<Env> = {}): Env => ({
  RPC_URL: 'http://rpc.test',
  WORKER_SECRET_KEY: keygenJson(),
  LOBBY_PROGRAM_ID: PROGRAM,
  EXPECTED_GENESIS_HASH: 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
  MAX_CANDIDATES_PER_RUN: '2',
  LOW_BALANCE_LAMPORTS: '0',
  ...o,
});

const lines = () => vi.mocked(console.log).mock.calls.map((c) => JSON.parse(String(c[0])) as Record<string, unknown>);

describe('run() tick loop (mocked RPC)', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('earliest deadline first · cap respected · noop simulated but never sent', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const rpc = await mockRpc({
      balance: 1_000_000_000,
      // due: 1 (deadline NOW-70), 2 (NOW-470), 3 (NOW-270) · 4 not due yet
      rooms: [
        { id: 1n, phaseSince: NOW - 100 },
        { id: 2n, phaseSince: NOW - 500 },
        { id: 3n, phaseSince: NOW - 300 },
        { id: 4n, phaseSince: NOW - 10 },
      ],
      emits: new Set([2n]),
    });
    await run('tick', env(), NOW);
    expect(rpc.simulated()).toEqual([2n, 3n]); // oldest deadlines, cap 2
    expect(rpc.sent()).toEqual([2n]); // 3 simulated as no-op → not sent
    const done = lines().find((l) => l.msg === 'done')!;
    expect(done).toMatchObject({ candidates: 3, deferred: 1, sent: 1, noop: 1, rejected: 0, raced: 0, failed: 0 });
  });

  it('wallet below rent floor + fees → no simulate, no send, one error · exactly enough → runs', async () => {
    const two = [{ id: 1n, phaseSince: 0 }, { id: 2n, phaseSince: 0 }];
    const need = Number(PAYER_RENT_FLOOR) + 2 * 5_000;
    vi.spyOn(console, 'log').mockImplementation(() => {});
    let rpc = await mockRpc({ balance: need - 1, rooms: two, emits: new Set([1n, 2n]) });
    await run('tick', env(), NOW);
    expect(rpc.simulated()).toEqual([]);
    expect(rpc.sent()).toEqual([]);
    expect(lines().some((l) => l.level === 'error' && String(l.msg).includes('cannot pay fees'))).toBe(true);
    rpc = await mockRpc({ balance: need, rooms: two, emits: new Set([1n, 2n]) });
    await run('tick', env(), NOW);
    expect(rpc.sent()).toEqual([1n, 2n]);
  });

  it('send failing for a non-race reason (HTTP 429) is an error, not `raced`', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    await mockRpc({ balance: 1_000_000_000, rooms: [{ id: 1n, phaseSince: 0 }], emits: new Set([1n]), sendStatus: 429 });
    await run('tick', env(), NOW);
    expect(lines().some((l) => l.outcome === 'raced')).toBe(false);
    expect(lines().some((l) => l.level === 'error' && String(l.msg).includes('429'))).toBe(true);
    expect(lines().find((l) => l.msg === 'done')).toMatchObject({ sent: 0, raced: 0, failed: 1 });
  });

  it('preflight rejection after a good simulate → `raced` at info', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    await mockRpc({ balance: 1_000_000_000, rooms: [{ id: 1n, phaseSince: 0 }], emits: new Set([1n]), sendPreflightFails: true });
    await run('tick', env(), NOW);
    expect(lines().find((l) => l.outcome === 'raced')).toMatchObject({ level: 'info' });
    expect(lines().find((l) => l.msg === 'done')).toMatchObject({ sent: 0, raced: 1, failed: 0 });
  });

  it('cap is clamped to the subrequest budget', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const rooms = Array.from({ length: 30 }, (_, i) => ({ id: BigInt(i + 1), phaseSince: 0 }));
    const rpc = await mockRpc({ balance: 1_000_000_000, rooms, emits: new Set(rooms.map((r) => r.id)) }); // worst case: all send
    await run('tick', env({ MAX_CANDIDATES_PER_RUN: '1000' }), NOW);
    expect(rpc.simulated()).toHaveLength(23);
    expect(rpc.sent()).toHaveLength(23);
    expect(rpc.calls.length).toBe(50);
  });

  it('malformed RPC_URL is not echoed', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    await run('tick', env({ RPC_URL: 'not a url?api-key=SECRET123' }), NOW);
    expect(JSON.stringify(lines())).not.toContain('SECRET123');
  });
});

