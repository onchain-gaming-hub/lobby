import { readFileSync } from 'node:fs';
import { AccountRole, address, getAddressEncoder } from '@solana/kit';
import { describe, expect, it } from 'vitest';
import { CRON_JOBS } from '../src/index';
import { CLAIM_IDLE_SECS, DISC, Phase, type ClaimView, type RoomView } from '../src/layout';
import {
  claimMaybeClosable,
  claimPda,
  closeClaimIx,
  closeRoomIx,
  customErrorCode,
  emittedEvent,
  matchmakerPda,
  roomClosable,
  roomPda,
  tickIx,
  tickMaybeDue,
} from '../src/plan';

const PROGRAM = address('5zdBSvep3yV6v1SVD67uw9N5Wb3eT9Hp9D3eg4YSRqRu');
const GAME = address('5PjkqfpUic5nNYr5aN6nbGdHLSYQEEzK8wr7PDPv2aRw'); // mock-game
const enc = getAddressEncoder();
const game = Uint8Array.from(enc.encode(GAME));

const room = (o: Partial<RoomView> = {}): RoomView => ({
  rentPayer: Uint8Array.from(enc.encode(address('AvoF2kV5iiuu2bk7BAho98Nu7MQEtbhn8udzxVzb8wRE'))),
  gameProgram: game,
  roomId: 7n,
  phase: Phase.Waiting,
  isPublic: false,
  botFill: false,
  ttl: { wait: 60, ready: 30, lobby: 1800, start: 60, maxGame: 3600, retention: 600 },
  phaseSince: 1000,
  lastHumanActivityAt: 1000,
  ...o,
});

describe('PDAs match `solana find-program-derived-address`', () => {
  it('room / matchmaker / claim', async () => {
    expect(await roomPda(PROGRAM, game, 7n)).toBe('GHm6oPRecGxvuA8pao3gK3iQGVNWAgRW7MY8MtBDTHyc');
    expect(await matchmakerPda(PROGRAM, game)).toBe('8MKTm2rFxJhU2SMzykgnjDPWmEP3ef1AQZeQyXvdXeaE');
    const player = Uint8Array.from(enc.encode(address('AvoF2kV5iiuu2bk7BAho98Nu7MQEtbhn8udzxVzb8wRE')));
    expect(await claimPda(PROGRAM, player)).toBe('2uTKC4DQZkgmFhnzsEhgknbNTTv6xu8kAYPi95EXAzH8');
  });
});

describe('tickMaybeDue mirrors handle_tick deadlines', () => {
  it('Waiting: lobby TTL from last human activity', () => {
    expect(tickMaybeDue(room(), 1000 + 1799)).toBe(false);
    expect(tickMaybeDue(room(), 1000 + 1800)).toBe(true);
  });
  it('Waiting: public bot backfill after wait (only with bot_fill)', () => {
    expect(tickMaybeDue(room({ isPublic: true, botFill: true }), 1060)).toBe(true);
    expect(tickMaybeDue(room({ isPublic: true, botFill: false }), 1060)).toBe(false);
    expect(tickMaybeDue(room({ isPublic: false, botFill: true }), 1060)).toBe(false);
  });
  it('ReadyCheck / Starting / Playing', () => {
    expect(tickMaybeDue(room({ phase: Phase.ReadyCheck }), 1029)).toBe(false);
    expect(tickMaybeDue(room({ phase: Phase.ReadyCheck }), 1030)).toBe(true);
    expect(tickMaybeDue(room({ phase: Phase.Starting }), 1060)).toBe(true);
    expect(tickMaybeDue(room({ phase: Phase.Playing }), 4599)).toBe(false);
    expect(tickMaybeDue(room({ phase: Phase.Playing }), 4600)).toBe(true);
  });
  it('terminal rooms never tick', () => {
    expect(tickMaybeDue(room({ phase: Phase.Ended }), 1e12)).toBe(false);
    expect(tickMaybeDue(room({ phase: Phase.Cancelled }), 1e12)).toBe(false);
  });
});

describe('close filters', () => {
  it('room: terminal + retention', () => {
    expect(roomClosable(room({ phase: Phase.Ended }), 1599)).toBe(false);
    expect(roomClosable(room({ phase: Phase.Ended }), 1600)).toBe(true);
    expect(roomClosable(room({ phase: Phase.Cancelled }), 1600)).toBe(true);
    expect(roomClosable(room({ phase: Phase.Playing }), 1e12)).toBe(false);
  });
  it('claim: 30 idle days', () => {
    const c = { lastJoinedAt: 1000 } as ClaimView;
    expect(claimMaybeClosable(c, 1000 + CLAIM_IDLE_SECS - 1)).toBe(false);
    expect(claimMaybeClosable(c, 1000 + CLAIM_IDLE_SECS)).toBe(true);
  });
});

describe('instruction builders follow the IDL account order', () => {
  const ROOM_ADDR = address('GHm6oPRecGxvuA8pao3gK3iQGVNWAgRW7MY8MtBDTHyc');
  it('tick private → placeholder = program ID read-only', async () => {
    const ix = await tickIx(PROGRAM, ROOM_ADDR, room());
    expect(ix.data).toEqual(DISC.tick);
    expect(ix.accounts).toEqual([
      { address: ROOM_ADDR, role: AccountRole.WRITABLE },
      { address: PROGRAM, role: AccountRole.READONLY },
    ]);
  });
  it('tick public → matchmaker PDA writable', async () => {
    const ix = await tickIx(PROGRAM, ROOM_ADDR, room({ isPublic: true }));
    expect(ix.accounts![1]).toEqual({ address: '8MKTm2rFxJhU2SMzykgnjDPWmEP3ef1AQZeQyXvdXeaE', role: AccountRole.WRITABLE });
  });
  it('close_room → rent_payer from the account', () => {
    const ix = closeRoomIx(PROGRAM, ROOM_ADDR, room());
    expect(ix.data).toEqual(DISC.closeRoom);
    expect(ix.accounts).toEqual([
      { address: ROOM_ADDR, role: AccountRole.WRITABLE },
      { address: 'AvoF2kV5iiuu2bk7BAho98Nu7MQEtbhn8udzxVzb8wRE', role: AccountRole.WRITABLE },
    ]);
  });
  it('close_claim → prev_room = room `current` points at, else placeholder', async () => {
    const CLAIM_ADDR = address('2uTKC4DQZkgmFhnzsEhgknbNTTv6xu8kAYPi95EXAzH8');
    const base: ClaimView = { rentPayer: room().rentPayer, player: room().rentPayer, lastJoinedAt: 0, current: null };
    const none = await closeClaimIx(PROGRAM, CLAIM_ADDR, base);
    expect(none.data).toEqual(DISC.closeClaim);
    expect(none.accounts![2]).toEqual({ address: PROGRAM, role: AccountRole.READONLY });
    const some = await closeClaimIx(PROGRAM, CLAIM_ADDR, { ...base, current: { gameProgram: game, roomId: 7n } });
    expect(some.accounts![2]).toEqual({ address: ROOM_ADDR, role: AccountRole.READONLY });
  });
});

describe('simulation reading', () => {
  it('only `Program data:` (an emitted event) counts as a change', () => {
    expect(emittedEvent(['Program log: Instruction: Tick', 'Program 5zdB… success'])).toBe(false);
    expect(emittedEvent(['Program data: spJD3tAk1ds='])).toBe(true);
    expect(emittedEvent(null)).toBe(false);
  });
  it('custom error code', () => {
    expect(customErrorCode({ InstructionError: [0, { Custom: 6025 }] })).toBe(6025);
    expect(customErrorCode({ InstructionError: [0, { Custom: 3012n }] })).toBe(3012);
    expect(customErrorCode('AccountNotFound')).toBeNull();
  });
});

describe('config', () => {
  it('wrangler.toml crons == CRON_JOBS', () => {
    const toml = readFileSync(new URL('../wrangler.toml', import.meta.url), 'utf8');
    const crons = JSON.parse(toml.match(/^crons = (\[.*\])$/m)![1]) as string[];
    expect(crons.sort()).toEqual(Object.keys(CRON_JOBS).sort());
  });
  it('vars point at the deployed program on devnet', () => {
    const toml = readFileSync(new URL('../wrangler.toml', import.meta.url), 'utf8');
    expect(toml).toContain(`LOBBY_PROGRAM_ID = "${PROGRAM}"`);
    expect(toml).toContain('EXPECTED_GENESIS_HASH = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"');
  });
});
