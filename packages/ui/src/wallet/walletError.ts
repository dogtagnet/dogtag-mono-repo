/**
 * What a wallet actually said, dug out of however deeply it was wrapped.
 *
 * TWO LIVE DEFECTS MADE THIS NECESSARY, and both are the same mistake: reading a wallet's answer at
 * one fixed depth, with one fixed code.
 *
 *   1. `useRoaxChain` added the ROAX chain only on code **4902**, which is MetaMask's spelling. A
 *      captain running a different wallet got `4100` instead, the add fallback never ran, and he was
 *      left adding the network by hand. A wallet that says "I do not know this chain" in its own
 *      dialect is still saying it.
 *   2. Providers wrap. viem re-throws with the original under `cause`, and several extensions nest
 *      theirs under `data.originalError`, so a top-level `err.code` read misses a code that is
 *      plainly present two levels down.
 *
 * So the codes are collected from the WHOLE chain rather than from the top, and unrecognized-chain
 * is matched on message as well - some wallets return `-32603` with the real reason only in text.
 *
 * Pure and separately tested, because every branch here is a wallet-specific failure that cannot be
 * reproduced in a browser without installing that wallet and putting it into that exact state.
 */

/** EIP-1193 / EIP-1474 codes this app distinguishes. */
export const WALLET_USER_REJECTED = 4001;
export const WALLET_UNAUTHORIZED = 4100;
export const WALLET_UNRECOGNIZED_CHAIN = 4902;

/**
 * Every `code` reachable from a thrown value, outermost first.
 *
 * Walks `cause`, `data`, `data.originalError` and `error`, which between them cover viem's wrapping
 * and the shapes the common extensions use. Cycle-guarded, because a provider error that references
 * itself would otherwise hang the handler rather than merely reporting badly.
 */
export function walletErrorCodes(err: unknown): number[] {
  const codes: number[] = [];
  const seen = new Set<unknown>();
  const walk = (v: unknown, depth: number) => {
    if (depth > 6 || v === null || typeof v !== "object" || seen.has(v)) return;
    seen.add(v);
    const o = v as Record<string, unknown>;
    if (typeof o.code === "number") codes.push(o.code);
    for (const key of ["cause", "data", "originalError", "error"]) walk(o[key], depth + 1);
  };
  walk(err, 0);
  return codes;
}

/** The first line of whatever the wallet said, for diagnosis. Never the whole stack. */
export function walletErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) return err.message.split("\n")[0]!.trim();
  if (typeof err === "string" && err.trim()) return err.split("\n")[0]!.trim();
  const m = (err as { message?: unknown } | null)?.message;
  if (typeof m === "string" && m.trim()) return m.split("\n")[0]!.trim();
  return "no reason given";
}

/**
 * Whether the wallet is saying it does not know this chain, in ANY of its dialects.
 *
 * Message matching sits beside the code because several wallets answer a generic `-32603` and put
 * the only usable information in the text. That is looser than a code compare and deliberately so:
 * the cost of a false positive here is one extra `wallet_addEthereumChain` prompt the user can
 * decline, and the cost of a false negative is what the captain hit - a dead end with a manual
 * network setup as the only way forward.
 */
export function isUnrecognizedChain(err: unknown): boolean {
  if (walletErrorCodes(err).includes(WALLET_UNRECOGNIZED_CHAIN)) return true;
  const text = walletErrorMessage(err).toLowerCase();
  return (
    text.includes("unrecognized chain") ||
    text.includes("unrecognised chain") ||
    text.includes("chain has not been added") ||
    text.includes("try adding the chain") ||
    text.includes("add the chain") ||
    text.includes("unknown chain")
  );
}

/** A deliberate refusal by the person, which must never be retried or reported as a fault. */
export function isUserRejection(err: unknown): boolean {
  if (walletErrorCodes(err).includes(WALLET_USER_REJECTED)) return true;
  const text = walletErrorMessage(err).toLowerCase();
  return text.includes("user rejected") || text.includes("user denied");
}

/**
 * The wallet declined to answer at all: EIP-1193 `4100 Unauthorized`.
 *
 * Worth its own predicate because of how it presented. It reads as "not authorized", which on a page
 * about provider authorization is the most misleading sentence a wallet could possibly return - a
 * captain read it as his provider record being unauthorized while the chain said the opposite. It is
 * a statement by a browser extension about a site, and it establishes NOTHING about the provider.
 */
export function isWalletUnauthorized(err: unknown): boolean {
  if (walletErrorCodes(err).includes(WALLET_UNAUTHORIZED)) return true;
  const text = walletErrorMessage(err).toLowerCase();
  return text.includes("has not been authorized by the user");
}

/**
 * What kind of fault a caught throw is, for a surface that must not render it as a verdict.
 *
 * `walletUnauthorized` and `walletRejected` are split from `walletFault` because their remedies
 * differ completely: one is "grant this site access, or check which wallet answered", the other is
 * "you pressed cancel", and a single "wallet error" would send the second user hunting through
 * extension settings.
 */
export type SurfaceFaultKind =
  | "walletRejected"
  | "walletUnauthorized"
  | "walletFault"
  | "surfaceFault";

export interface SurfaceFault {
  kind: SurfaceFaultKind;
  /** The wallet's own first line, kept so the fault is diagnosable. Never presented as a verdict. */
  detail: string;
  /** What this page could not establish. Always stated, so silence is never read as an answer. */
  established: string;
  nextStep: string;
}

const NOTHING_ESTABLISHED =
  "Nothing about your provider record was checked, so this says nothing about whether you are authorized.";

export function classifySurfaceFault(err: unknown): SurfaceFault {
  const detail = walletErrorMessage(err);
  if (isUserRejection(err)) {
    return {
      kind: "walletRejected",
      detail,
      established: NOTHING_ESTABLISHED,
      nextStep: "You cancelled the request in your wallet. Try again when you are ready.",
    };
  }
  if (isWalletUnauthorized(err)) {
    return {
      kind: "walletUnauthorized",
      detail,
      established: NOTHING_ESTABLISHED,
      // Names the multi-wallet case first, because that is what produces this without the user
      // having done anything wrong, and it is invisible from inside the page.
      nextStep:
        "Your wallet extension refused the request. If you have more than one wallet extension installed, check that the one you connected is the one that opened - and that this site is connected to the account shown above.",
    };
  }
  return {
    kind: "walletFault",
    detail,
    established: NOTHING_ESTABLISHED,
    nextStep:
      "Your wallet could not complete the request. Check that it is unlocked and connected to this site, then try again.",
  };
}
