// RPC + tx plumbing. Stateless: every run re-reads the chain (04 §5).
import {
  type Address,
  type Base58EncodedBytes,
  type Instruction,
  type KeyPairSigner,
  SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE,
  appendTransactionMessageInstruction,
  compileTransaction,
  createSolanaRpc,
  createTransactionMessage,
  getBase58Decoder,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  getSignatureFromTransaction,
  isSolanaError,
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
  | { kind: 'rejected'; code: number | null; err: unknown } // program said no — skip quietly (04 §2)
  | { kind: 'raced'; err: string }; // simulate ok, preflight of the send failed: state changed in between

/**
 * Simulate first (unsigned — `sigVerify: false`, saves CPU on the many no-op ticks); sign and send only
 * when the simulation succeeds *and* changed state (04 §2).
 */
export async function simulateThenSend(
  rpc: Rpc,
  signer: KeyPairSigner,
  blockhash: Blockhash,
  ix: Instruction,
): Promise<Outcome> {
  const message = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(signer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstruction(ix, m),
  );
  const unsigned = getBase64EncodedWireTransaction(compileTransaction(message));
  const sim = await rpc.simulateTransaction(unsigned, { encoding: 'base64', sigVerify: false, commitment: 'confirmed' }).send();
  if (sim.value.err) return { kind: 'rejected', code: customErrorCode(sim.value.err), err: sim.value.err };
  if (!emittedEvent(sim.value.logs)) return { kind: 'noop' };
  const tx = await signTransactionMessageWithSigners(message);
  // preflight stays on: a race between simulate and send fails here for free instead of on chain.
  // Accepted (P7 gate N2): if someone else ticks in between, our tick becomes a no-op that still
  // passes preflight and costs one fee (5,000 lamports) — bounded by how often races happen.
  try {
    await rpc.sendTransaction(getBase64EncodedWireTransaction(tx), { encoding: 'base64', preflightCommitment: 'confirmed' }).send();
  } catch (e) {
    // only a preflight rejection means "state changed since simulate"; 429 / 5xx / transport → caller logs error
    if (isSolanaError(e, SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE)) return { kind: 'raced', err: String(e) };
    throw e;
  }
  return { kind: 'sent', signature: getSignatureFromTransaction(tx) };
}
