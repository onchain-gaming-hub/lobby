// Raw account layout of lobby IDL 0.1.0 (specs/designs/01-accounts.md · 04 §2).
// Only fields before the first Option/Vec are read raw — later offsets move (01 §0).
// test/layout.test.ts re-derives every number here from idls/lobby.json.

export const DISC = {
  room: hex('9cc7431bde17b95e'),
  playerClaim: hex('9d44604094406bfe'),
  tick: hex('5c4f2c0865503f0f'),
  closeRoom: hex('98c558c062c533d3'),
  closeClaim: hex('2ab1a523d5b3d313'),
} as const;

export const SEED = {
  room: 'room',
  matchmaker: 'matchmaker',
  claim: 'claim',
} as const;

/** program constant `CLAIM_IDLE_SECS` (L13 (c)) — Worker uses it only as a pre-filter. */
export const CLAIM_IDLE_SECS = 30 * 86_400;

export const Phase = { Waiting: 0, ReadyCheck: 1, Starting: 2, Playing: 3, Ended: 4, Cancelled: 5 } as const;
export type Phase = (typeof Phase)[keyof typeof Phase];

export const ROOM = {
  rentPayer: 10,
  gameProgram: 42,
  roomId: 74,
  phase: 82,
  isPublic: 84,
  minHumans: 86,
  botFill: 87,
  ttl: 89, // RoomTtl: wait, ready, lobby, start, max_game, rematch_window, retention (u32 each)
  phaseSince: 117,
  lastHumanActivityAt: 125,
  sliceLen: 133, // first variable field (`host`) starts here
} as const;

/** Seat = kind u8 (Empty 0 · Human 1 · Bot 2) + player 32 + ready u8 + left u8. */
export const SEAT = { len: 35, kindHuman: 1, left: 34 } as const;

export const CLAIM = {
  rentPayer: 10,
  player: 42,
  lastJoinedAt: 74,
  current: 82, // Option<RoomRef>: tag u8, then game_program (32) + room_id (u64)
  sliceLen: 123,
} as const;

export interface RoomView {
  rentPayer: Uint8Array;
  gameProgram: Uint8Array;
  roomId: bigint;
  phase: Phase;
  isPublic: boolean;
  minHumans: number;
  botFill: boolean;
  /** active humans (`Human && !left`) — null when decoded from the fixed-prefix slice only */
  humans: number | null;
  ttl: { wait: number; ready: number; lobby: number; start: number; maxGame: number; retention: number };
  phaseSince: number;
  lastHumanActivityAt: number;
}

export interface ClaimView {
  rentPayer: Uint8Array;
  player: Uint8Array;
  lastJoinedAt: number;
  current: { gameProgram: Uint8Array; roomId: bigint } | null;
}

export function decodeRoom(d: Uint8Array): RoomView | null {
  if (d.length < ROOM.sliceLen || !startsWith(d, DISC.room)) return null;
  const v = view(d);
  const t = ROOM.ttl;
  return {
    rentPayer: d.slice(ROOM.rentPayer, ROOM.rentPayer + 32),
    gameProgram: d.slice(ROOM.gameProgram, ROOM.gameProgram + 32),
    roomId: v.getBigUint64(ROOM.roomId, true),
    phase: d[ROOM.phase] as Phase,
    isPublic: d[ROOM.isPublic] === 1,
    minHumans: d[ROOM.minHumans],
    botFill: d[ROOM.botFill] === 1,
    humans: countHumans(d),
    ttl: {
      wait: v.getUint32(t, true),
      ready: v.getUint32(t + 4, true),
      lobby: v.getUint32(t + 8, true),
      start: v.getUint32(t + 12, true),
      maxGame: v.getUint32(t + 16, true),
      retention: v.getUint32(t + 24, true),
    },
    phaseSince: Number(v.getBigInt64(ROOM.phaseSince, true)),
    lastHumanActivityAt: Number(v.getBigInt64(ROOM.lastHumanActivityAt, true)),
  };
}

export function decodeClaim(d: Uint8Array): ClaimView | null {
  if (d.length < CLAIM.sliceLen || !startsWith(d, DISC.playerClaim)) return null;
  const v = view(d);
  const c = CLAIM.current;
  return {
    rentPayer: d.slice(CLAIM.rentPayer, CLAIM.rentPayer + 32),
    player: d.slice(CLAIM.player, CLAIM.player + 32),
    lastJoinedAt: Number(v.getBigInt64(CLAIM.lastJoinedAt, true)),
    current: d[c] === 1 ? { gameProgram: d.slice(c + 1, c + 33), roomId: v.getBigUint64(c + 33, true) } : null,
  };
}

/** Walks host: Option<Pubkey> · rematch_room_id: Option<u64> · seats: Vec<Seat> (borsh: None = 1 byte). */
function countHumans(d: Uint8Array): number | null {
  let o = ROOM.sliceLen;
  if (d.length < o + 1) return null;
  o += d[o] === 1 ? 33 : 1;
  if (d.length < o + 1) return null;
  o += d[o] === 1 ? 9 : 1;
  if (d.length < o + 4) return null;
  const n = view(d).getUint32(o, true);
  o += 4;
  if (d.length < o + n * SEAT.len) return null;
  let humans = 0;
  for (let i = 0; i < n; i++, o += SEAT.len) if (d[o] === SEAT.kindHuman && d[o + SEAT.left] === 0) humans++;
  return humans;
}

function view(d: Uint8Array): DataView {
  return new DataView(d.buffer, d.byteOffset, d.byteLength);
}

function startsWith(d: Uint8Array, p: Uint8Array): boolean {
  return p.every((b, i) => d[i] === b);
}

function hex(s: string): Uint8Array {
  return Uint8Array.from(s.match(/../g)!.map((b) => parseInt(b, 16)));
}
