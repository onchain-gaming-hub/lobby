// Pure decisions + instruction builders. Every filter here is only a pre-filter to save RPC calls —
// the program decides (04 §2): a wrong guess costs one simulate, never a fee.
import {
  AccountRole,
  type Address,
  type Instruction,
  getAddressDecoder,
  getProgramDerivedAddress,
  getU64Encoder,
} from '@solana/kit';
import { CLAIM_IDLE_SECS, type ClaimView, DISC, Phase, type RoomView, SEED } from './layout';

const addr = (b: Uint8Array) => getAddressDecoder().decode(b);

/** Some deadline of this room may have passed (02 §4 · mirrors `handle_tick`'s due checks). */
export function tickMaybeDue(r: RoomView, now: number): boolean {
  const due = (anchor: number, ttl: number) => now >= anchor + ttl;
  switch (r.phase) {
    case Phase.Waiting:
      // backfill also needs humans >= min_humans (seats are past the raw slice) → simulate decides
      return due(r.lastHumanActivityAt, r.ttl.lobby) || (r.isPublic && r.botFill && due(r.phaseSince, r.ttl.wait));
    case Phase.ReadyCheck:
      return due(r.phaseSince, r.ttl.ready);
    case Phase.Starting:
      return due(r.phaseSince, r.ttl.start);
    case Phase.Playing:
      return due(r.phaseSince, r.ttl.maxGame);
    default:
      return false;
  }
}

export function roomClosable(r: RoomView, now: number): boolean {
  return (r.phase === Phase.Ended || r.phase === Phase.Cancelled) && now >= r.phaseSince + r.ttl.retention;
}

/** Liveness / kick cooldown are checked on chain (needs the pointed room) → simulate decides. */
export function claimMaybeClosable(c: ClaimView, now: number): boolean {
  return now >= c.lastJoinedAt + CLAIM_IDLE_SECS;
}

export async function roomPda(program: Address, gameProgram: Uint8Array, roomId: bigint): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: program,
    seeds: [SEED.room, gameProgram, getU64Encoder().encode(roomId)],
  });
  return pda;
}

export async function matchmakerPda(program: Address, gameProgram: Uint8Array): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({ programAddress: program, seeds: [SEED.matchmaker, gameProgram] });
  return pda;
}

export async function claimPda(program: Address, player: Uint8Array): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({ programAddress: program, seeds: [SEED.claim, player] });
  return pda;
}

/** Optional account left out = the program ID, read-only (Anchor `to_account_metas` for `None`). */
const placeholder = (program: Address) => ({ address: program, role: AccountRole.READONLY });

/** `tick`: [room (w), matchmaker (w) | placeholder]. Public rooms always get the matchmaker (02 §6). */
export async function tickIx(program: Address, room: Address, r: RoomView): Promise<Instruction> {
  return {
    programAddress: program,
    accounts: [
      { address: room, role: AccountRole.WRITABLE },
      r.isPublic ? { address: await matchmakerPda(program, r.gameProgram), role: AccountRole.WRITABLE } : placeholder(program),
    ],
    data: DISC.tick,
  };
}

/** `close_room`: [room (w), rent_payer (w)] — refund pinned by `has_one` (R3). */
export function closeRoomIx(program: Address, room: Address, r: RoomView): Instruction {
  return {
    programAddress: program,
    accounts: [
      { address: room, role: AccountRole.WRITABLE },
      { address: addr(r.rentPayer), role: AccountRole.WRITABLE },
    ],
    data: DISC.closeRoom,
  };
}

/** `close_claim`: [claim (w), rent_payer (w), prev_room = room `current` points at | placeholder]. */
export async function closeClaimIx(program: Address, claim: Address, c: ClaimView): Promise<Instruction> {
  return {
    programAddress: program,
    accounts: [
      { address: claim, role: AccountRole.WRITABLE },
      { address: addr(c.rentPayer), role: AccountRole.WRITABLE },
      c.current
        ? { address: await roomPda(program, c.current.gameProgram, c.current.roomId), role: AccountRole.READONLY }
        : placeholder(program),
    ],
    data: DISC.closeClaim,
  };
}

/** Did the simulated tx change anything? Every lobby state change emits an event (`Program data:`). */
export function emittedEvent(logs: readonly string[] | null): boolean {
  return (logs ?? []).some((l) => l.startsWith('Program data: '));
}

/** Custom program error code from a simulate `err`, if any. */
export function customErrorCode(err: unknown): number | null {
  const ie = (err as { InstructionError?: [unknown, unknown] } | null)?.InstructionError;
  const custom = (ie?.[1] as { Custom?: number | bigint } | undefined)?.Custom;
  return custom === undefined ? null : Number(custom);
}
