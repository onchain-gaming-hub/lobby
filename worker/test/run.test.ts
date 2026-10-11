import { generateKeyPairSync } from 'node:crypto';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type Env, run } from '../src/worker';

/** a real ed25519 keypair in solana-keygen form: secret (32) ‖ public (32) */
function keygenJson(): string {
  const jwk = generateKeyPairSync('ed25519').privateKey.export({ format: 'jwk' });
  return JSON.stringify([...Buffer.from(jwk.d!, 'base64url'), ...Buffer.from(jwk.x!, 'base64url')]);
}

const env = (o: Partial<Env>): Env => ({
  RPC_URL: 'http://127.0.0.1:1', // never reached: config errors return before any RPC call
  WORKER_SECRET_KEY: keygenJson(),
  LOBBY_PROGRAM_ID: '5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu',
  EXPECTED_GENESIS_HASH: 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
  MAX_CANDIDATES_PER_RUN: '20',
  LOW_BALANCE_LAMPORTS: '50000000',
  ...o,
});

function logs(): string[] {
  return vi.mocked(console.log).mock.calls.map((c) => String(c[0]));
}

describe('run() config errors: one fixed log line, no throw, no secret', () => {
  afterEach(() => vi.restoreAllMocks());

  it('base58 key pasted instead of the JSON array → fixed message, key not echoed', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const secret = '5Kd3NBUAdUnhyzenEwVLy9pBKxSwXvE9FMPyR4UKZvpe6E3AgLr3vQqzJDkGfMcHmyfNKPXXJdZd8jt8';
    await run('tick', env({ WORKER_SECRET_KEY: secret }));
    expect(logs()).toHaveLength(1);
    expect(logs()[0]).toContain('WORKER_SECRET_KEY must be a JSON array');
    expect(logs()[0]).not.toContain(secret.slice(0, 6));
  });

  it('bad cap / empty RPC', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    await run('tick', env({ MAX_CANDIDATES_PER_RUN: '' }));
    await run('tick', env({ RPC_URL: '' }));
    expect(logs()[0]).toContain('MAX_CANDIDATES_PER_RUN');
    expect(logs()[1]).toContain('RPC_URL is empty');
  });

  it('unreachable RPC → logged, not thrown', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    await expect(run('tick', env({}))).resolves.toBeUndefined();
    expect(logs()[0]).toContain('"msg":"run failed"');
  });
});
