// Re-derives every raw offset / discriminator in src/layout.ts from the frozen IDL 0.1.0.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CLAIM, DISC, Phase, ROOM, SEAT, decodeClaim, decodeRoom } from '../src/layout';

type IdlType = string | { defined?: { name: string }; option?: unknown; vec?: unknown; array?: [IdlType, number] };
interface IdlTypeDef {
  name: string;
  type: { kind: 'struct' | 'enum'; fields?: { name: string; type: IdlType }[]; variants?: { name: string; fields?: unknown }[] };
}
const idl = JSON.parse(readFileSync(new URL('../../idls/lobby.json', import.meta.url), 'utf8')) as {
  version?: string;
  metadata?: { version: string };
  accounts: { name: string; discriminator: number[] }[];
  instructions: { name: string; discriminator: number[] }[];
  types: IdlTypeDef[];
};
const typeDef = (n: string) => idl.types.find((t) => t.name === n)!;

const PRIM: Record<string, number> = { u8: 1, bool: 1, u32: 4, i64: 8, u64: 8, pubkey: 32 };
function size(t: IdlType): number {
  if (typeof t === 'string') return PRIM[t] ?? fail(`unknown prim ${t}`);
  if (t.array) return size(t.array[0]) * t.array[1];
  if (t.defined) {
    const d = typeDef(t.defined.name);
    if (d.type.kind === 'enum') {
      if (d.type.variants!.some((v) => v.fields)) fail(`enum with data ${d.name}`);
      return 1;
    }
    return d.type.fields!.reduce((s, f) => s + size(f.type), 0);
  }
  return fail(`variable-size type before the fixed prefix ends: ${JSON.stringify(t)}`);
}
function fail(m: string): never {
  throw new Error(m);
}

/** offset (incl. 8-byte discriminator) of every field up to and including the first Option/Vec. */
function offsets(account: string): Record<string, number> {
  const out: Record<string, number> = {};
  let off = 8;
  for (const f of typeDef(account).type.fields!) {
    out[f.name] = off;
    const t = f.type;
    if (typeof t === 'object' && (t.option || t.vec)) break;
    off += size(t);
  }
  return out;
}
const bytes = (a: number[]) => Uint8Array.from(a);

describe('layout matches idls/lobby.json 0.1.0', () => {
  it('is IDL 0.1.0', () => {
    expect(idl.metadata?.version ?? idl.version).toBe('0.1.0');
  });

  it('discriminators', () => {
    const acc = (n: string) => bytes(idl.accounts.find((a) => a.name === n)!.discriminator);
    const ix = (n: string) => bytes(idl.instructions.find((i) => i.name === n)!.discriminator);
    expect(DISC.room).toEqual(acc('Room'));
    expect(DISC.playerClaim).toEqual(acc('PlayerClaim'));
    expect(DISC.tick).toEqual(ix('tick'));
    expect(DISC.closeRoom).toEqual(ix('close_room'));
    expect(DISC.closeClaim).toEqual(ix('close_claim'));
  });

  it('Room offsets', () => {
    const o = offsets('Room');
    expect(ROOM).toEqual({
      rentPayer: o.rent_payer,
      gameProgram: o.game_program,
      roomId: o.room_id,
      phase: o.phase,
      isPublic: o.is_public,
      minHumans: o.min_humans,
      botFill: o.bot_fill,
      ttl: o.ttl,
      phaseSince: o.phase_since,
      lastHumanActivityAt: o.last_human_activity_at,
      sliceLen: o.host,
    });
    const ttl = typeDef('RoomTtl').type.fields!.map((f) => f.name);
    expect(ttl).toEqual(['wait', 'ready', 'lobby', 'start', 'max_game', 'rematch_window', 'retention']);
    const phases = typeDef('Phase').type.variants!.map((v) => v.name);
    expect(phases).toEqual(Object.keys(Phase));
  });

  it('Seat layout + variable part order', () => {
    const f = typeDef('Seat').type.fields!;
    expect(f.map((x) => x.name)).toEqual(['kind', 'player', 'ready', 'left']);
    expect(size({ defined: { name: 'Seat' } })).toBe(SEAT.len);
    expect(1 + 32 + 1).toBe(SEAT.left);
    expect(typeDef('SeatKind').type.variants!.map((v) => v.name)).toEqual(['Empty', 'Human', 'Bot']);
    const names = typeDef('Room').type.fields!.map((x) => x.name);
    expect(names.slice(names.indexOf('host'))).toEqual(['host', 'rematch_room_id', 'seats', 'reserved']);
  });

  it('PlayerClaim offsets', () => {
    const o = offsets('PlayerClaim');
    expect(o.rent_payer).toBe(CLAIM.rentPayer);
    expect(o.player).toBe(CLAIM.player);
    expect(o.last_joined_at).toBe(CLAIM.lastJoinedAt);
    expect(o.current).toBe(CLAIM.current);
    // Option tag + RoomRef { game_program, room_id }
    expect(CLAIM.sliceLen).toBe(CLAIM.current + 1 + size({ defined: { name: 'RoomRef' } }));
    expect(typeDef('RoomRef').type.fields!.map((f) => f.name)).toEqual(['game_program', 'room_id']);
  });
});

describe('decode', () => {
  it('Room prefix', () => {
    const d = new Uint8Array(ROOM.sliceLen);
    d.set(DISC.room);
    const v = new DataView(d.buffer);
    d.fill(7, ROOM.rentPayer, ROOM.rentPayer + 32);
    v.setBigUint64(ROOM.roomId, 42n, true);
    d[ROOM.phase] = Phase.ReadyCheck;
    d[ROOM.isPublic] = 1;
    v.setUint32(ROOM.ttl + 4, 30, true); // ready
    v.setUint32(ROOM.ttl + 24, 600, true); // retention
    v.setBigInt64(ROOM.phaseSince, 1_000n, true);
    const r = decodeRoom(d)!;
    expect(r.rentPayer).toEqual(new Uint8Array(32).fill(7));
    expect(r.roomId).toBe(42n);
    expect(r.phase).toBe(Phase.ReadyCheck);
    expect(r.isPublic).toBe(true);
    expect(r.ttl.ready).toBe(30);
    expect(r.ttl.retention).toBe(600);
    expect(r.phaseSince).toBe(1000);
  });

  it('counts active humans from the whole account (host / rematch Some or None)', () => {
    const seat = (kind: number, left = 0) => [kind, ...new Array(32).fill(1), 0, left];
    const seats = [seat(1), seat(1, 1), seat(2), seat(0), seat(1)]; // human · left human · bot · empty · human
    for (const [host, rematch] of [[[0], [0]], [[1, ...new Array(32).fill(5)], [1, 9, 0, 0, 0, 0, 0, 0, 0]]]) {
      const tail = [...host, ...rematch, seats.length, 0, 0, 0, ...seats.flat(), ...new Array(32).fill(0)];
      const d = new Uint8Array(ROOM.sliceLen + tail.length);
      d.set(DISC.room);
      d[ROOM.minHumans] = 2;
      d.set(tail, ROOM.sliceLen);
      const r = decodeRoom(d)!;
      expect(r.humans).toBe(2);
      expect(r.minHumans).toBe(2);
    }
  });

  it('slice only → humans unknown', () => {
    const d = new Uint8Array(ROOM.sliceLen);
    d.set(DISC.room);
    expect(decodeRoom(d)!.humans).toBeNull();
  });

  it('rejects other accounts and short slices', () => {
    const d = new Uint8Array(ROOM.sliceLen);
    d.set(DISC.playerClaim);
    expect(decodeRoom(d)).toBeNull();
    expect(decodeClaim(new Uint8Array(10))).toBeNull();
  });

  it('PlayerClaim current Some / None', () => {
    const d = new Uint8Array(CLAIM.sliceLen);
    d.set(DISC.playerClaim);
    expect(decodeClaim(d)!.current).toBeNull();
    d[CLAIM.current] = 1;
    d.fill(9, CLAIM.current + 1, CLAIM.current + 33);
    new DataView(d.buffer).setBigUint64(CLAIM.current + 33, 5n, true);
    expect(decodeClaim(d)!.current).toEqual({ gameProgram: new Uint8Array(32).fill(9), roomId: 5n });
  });
});
