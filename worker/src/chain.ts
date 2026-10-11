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
  | { kind: 'raced'; err: ErrorFields }; // simulate ok, the program said no at preflight: state changed in between

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
    // "state changed since simulate" only when preflight failed *inside the program* (an InstructionError).
    // BlockhashNotFound (lagging node) · 429 / 5xx / transport → rethrown → caller logs error (P7 gate F2).
    if (isSolanaError(e, SOLANA_ERROR__JSON_RPC__SERVER_ERROR_SEND_TRANSACTION_PREFLIGHT_FAILURE) && isInstructionError(e.cause)) {
      return { kind: 'raced', err: errorFields(e) };
    }
    throw e;
  }
  return { kind: 'sent', signature: getSignatureFromTransaction(tx) };
}

/** kit's InstructionError codes are 4615xxx (`SOLANA_ERROR__INSTRUCTION_ERROR__*`). */
function isInstructionError(e: unknown): boolean {
  return isSolanaError(e) && Math.floor(e.context.__code / 1000) === 4615;
}

export interface ErrorFields {
  code?: number; // kit SolanaError code (decode: npx @solana/errors decode -- <code>)
  status?: number; // HTTP status for transport errors
  cause?: number; // kit code of the cause (e.g. 4615026 = program Custom error)
  custom?: number; // program custom error number when the cause carries one
  server?: string; // JSON-RPC server message, when kit kept one
  error?: string; // non-kit errors only (key not `msg`: spread into log lines that have their own msg)
}

/**
 * Structured fields instead of `String(e)`: deployed (NODE_ENV=production) kit drops its messages and
 * prints only "Solana error #<code>" (P7 gate F1) — the codes survive, the text does not.
 */
export function errorFields(e: unknown): ErrorFields {
  if (!isSolanaError(e)) return { error: redactUrls(String(e).slice(0, 1000)).slice(0, 200) }; // bound regex input (O(n²) on long text)
  const ctx = e.context as { __code: number; statusCode?: number; __serverMessage?: string };
  const out: ErrorFields = { code: ctx.__code };
  if (ctx.statusCode !== undefined) out.status = ctx.statusCode;
  if (typeof ctx.__serverMessage === 'string') out.server = redactUrls(ctx.__serverMessage.slice(0, 1000)).slice(0, 200);
  if (isSolanaError(e.cause)) {
    const cctx = e.cause.context as { __code: number; code?: number | bigint };
    out.cause = cctx.__code;
    if (cctx.code !== undefined) out.custom = Number(cctx.code);
  }
  return out;
}

/** Non-kit error text (e.g. workerd fetch errors) can quote the RPC URL with its API key — drop any URL. */
export function redactUrls(s: string): string {
  return s.replace(/[a-z][a-z0-9+.-]*:\/\/\S+/gi, '<url>');
}
