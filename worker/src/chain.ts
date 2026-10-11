// RPC + tx plumbing. Stateless: every run re-reads the chain (04 §5).
import {
  type Address,
  type Base58EncodedBytes,
  type Instruction,
  type KeyPairSigner,
  appendTransactionMessageInstruction,
  createSolanaRpc,
  createTransactionMessage,
  getBase58Decoder,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  getSignatureFromTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
} from '@solana/kit';
import { customErrorCode, emittedEvent } from './plan';

export type Rpc = ReturnType<typeof createSolanaRpc>;
export type Blockhash = Awaited<ReturnType<ReturnType<Rpc['getLatestBlockhash']>['send']>>['value'];

export function rpcFor(url: string): Rpc {
  return createSolanaRpc(url);
}

/**
 * Accounts of one type (discriminator at offset 0) — sliced to the fixed-offset prefix, or whole when
 * `sliceLen` is omitted. ⚠️ unpaged: fine at devnet scale · measure before mainnet (P7 gate note).
 */
export async function fetchByDiscriminator(
  rpc: Rpc,
  program: Address,
  disc: Uint8Array,
  sliceLen?: number,
): Promise<{ address: Address; data: Uint8Array }[]> {
  const res = await rpc
    .getProgramAccounts(program, {
      encoding: 'base64',
      ...(sliceLen === undefined ? {} : { dataSlice: { offset: 0, length: sliceLen } }),
      filters: [{ memcmp: { offset: 0n, bytes: getBase58Decoder().decode(disc) as Base58EncodedBytes, encoding: 'base58' } }],
    })
    .send();
  const b64 = getBase64Encoder();
  return res.map((a) => ({ address: a.pubkey, data: Uint8Array.from(b64.encode(a.account.data[0])) }));
}

export type Outcome =
  | { kind: 'sent'; signature: string }
  | { kind: 'noop' } // simulate ok but nothing emitted (tick not due yet)
  | { kind: 'rejected'; code: number | null; err: unknown }; // program said no — skip quietly (04 §2)

/** Simulate first; send only when the simulation succeeds *and* changed state (04 §2). */
export async function simulateThenSend(
  rpc: Rpc,
  signer: KeyPairSigner,
  blockhash: Blockhash,
  ix: Instruction,
): Promise<Outcome> {
  const tx = await signTransactionMessageWithSigners(
    pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(signer, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
      (m) => appendTransactionMessageInstruction(ix, m),
    ),
  );
  const wire = getBase64EncodedWireTransaction(tx);
  const sim = await rpc.simulateTransaction(wire, { encoding: 'base64', sigVerify: false, commitment: 'confirmed' }).send();
  if (sim.value.err) return { kind: 'rejected', code: customErrorCode(sim.value.err), err: sim.value.err };
  if (!emittedEvent(sim.value.logs)) return { kind: 'noop' };
  // preflight stays on: a race between simulate and send fails here for free instead of on chain
  await rpc.sendTransaction(wire, { encoding: 'base64', preflightCommitment: 'confirmed' }).send();
  return { kind: 'sent', signature: getSignatureFromTransaction(tx) };
}
