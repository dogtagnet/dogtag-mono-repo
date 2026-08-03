/**
 * What actually happened to a transaction this surface sent (registry-plan S-15).
 *
 * THE DEFECT THIS EXISTS TO REMOVE. The flows used to record `Deployed contract: <hash>` straight
 * from `writeContractAsync`, which resolves as soon as the wallet hands back a hash. A transaction
 * that then mined with status 0 left a message asserting the action had succeeded, and the provider
 * had no way to tell. That is the same shape as checking one address and writing to another: a
 * verdict stated before the fact is established - on the one surface whose entire design principle
 * is that an unestablished fact is never stated as a verdict.
 *
 * It matters here more than on a surface with a real gate, because the preflight explicitly is NOT
 * one: the plan can be ready and the write still revert - a standing change between Check and Send,
 * a delegate expiring, a registrar repointing in the same block.
 *
 * SIX STATES, and the two that are easy to collapse are the two that mean "we cannot tell":
 *
 *   awaitingWallet - the request has gone to the wallet and it has not answered. NO HASH EXISTS YET.
 *   submitted      - the wallet returned a hash. Nothing about the outcome is known YET.
 *   succeeded      - a receipt was read and its status is success.
 *   reverted       - a receipt was read and its status is reverted. A definite failure.
 *   unknown        - a hash exists and the RECEIPT could not be read. Neither neighbour: a
 *                    transaction we could not follow is not one that failed.
 *   walletSilent   - the WALLET never answered, so no hash exists and it is not known whether
 *                    anything was broadcast at all.
 *
 * **`awaitingWallet` AND `walletSilent` BOTH EXIST BECAUSE OF A TRANSACTION THAT MINED AND WAS LOST.**
 * A captain pressed Deploy, confirmed in his wallet, and the page showed nothing - ever. The deploy
 * mined; the clone is on chain; his account nonce was 1, so it was the only transaction he had ever
 * sent. `sendAndFollow` awaited the wallet FIRST and recorded the attempt only after that promise
 * resolved, so the entire wallet window had no on-screen state, and a promise that never resolved
 * left the page blank indefinitely. The receipt path had a timeout and an explicit could-not-tell
 * state; the send path - the one place a promise can hang forever - had neither.
 *
 * `unknown` and `walletSilent` must NOT be merged, because their remedies differ completely: one has
 * a hash and the user checks the explorer, the other has none and the user checks their wallet and
 * re-runs the preflight. Merging them would tell somebody to look up a hash that does not exist.
 *
 * `submitted` must never render as "published"/"deployed", and neither could-not-tell state may ever
 * render as one of the definite ones.
 */

import { txExplorerHref } from "../chain/provenance";

export type SendState =
  | "awaitingWallet"
  | "submitted"
  | "succeeded"
  | "reverted"
  | "unknown"
  | "walletSilent";

/** The states in which no hash exists, because the wallet has not returned one. */
export function hasNoHash(state: SendState): boolean {
  return state === "awaitingWallet" || state === "walletSilent";
}

/** The states that report an inability to tell, rather than an outcome. */
export function isUnsettled(state: SendState): boolean {
  return state === "unknown" || state === "walletSilent";
}

export interface SendRecord {
  /**
   * Stable identity for this attempt, whether or not a hash ever arrives.
   *
   * The row is created BEFORE the wallet is asked, so it cannot be keyed on the hash - which is
   * exactly what forced the old code to record nothing until the wallet had already answered.
   */
  id: string;
  /** The transaction hash, once the wallet returned one. Absent while it has not. */
  hash?: string;
  /** What was being attempted, in the provider's terms. Never a claim that it happened. */
  what: string;
  state: SendState;
  /** Why the outcome could not be established. Present if and ONLY if the state is unsettled. */
  unknownReason?: string;
}

/**
 * Build a record with the reason invariant enforced rather than remembered.
 *
 * Same rule and same reason as `providerCheck`: a renderer must not be able to show a reason beside
 * a settled outcome, or omit one beside an unsettled one.
 */
export function sendRecord(
  id: string,
  what: string,
  state: SendState,
  opts: { hash?: string; unknownReason?: string } = {},
): SendRecord {
  const { hash, unknownReason } = opts;
  if (hasNoHash(state) && hash) {
    throw new Error("a send record with no wallet answer cannot carry a transaction hash");
  }
  if (!hasNoHash(state) && !hash) {
    throw new Error("a settled or submitted send record must carry its transaction hash");
  }
  if (isUnsettled(state)) {
    if (!unknownReason) throw new Error("a send record with an unknown outcome must state its reason");
    return hash ? { id, hash, what, state, unknownReason } : { id, what, state, unknownReason };
  }
  if (unknownReason) throw new Error("only an unknown outcome may carry a reason");
  return hash ? { id, hash, what, state } : { id, what, state };
}

/**
 * Fold a viem receipt's status into a settled state.
 *
 * `writeContractAsync` does NOT throw on a reverted transaction, so this is the only place the
 * difference is observed - and a caller that ignored it would send step 2 of a multi-step
 * publication after step 1 had reverted.
 */
export function outcomeFromReceiptStatus(status: string): SendState {
  return status === "success" ? "succeeded" : "reverted";
}

/** Whether the sequence may continue. Only an established success licenses the next transaction. */
export function mayContinueAfter(state: SendState): boolean {
  return state === "succeeded";
}

const STATE_LABEL: Readonly<Record<SendState, string>> = {
  awaitingWallet: "waiting for your wallet to respond",
  // Never "sent" alone - the point of the word is that the outcome is still open.
  submitted: "submitted, outcome not yet known",
  succeeded: "succeeded",
  reverted: "reverted on chain",
  unknown: "outcome could not be established",
  // Says MAY, because it may. The wallet not answering is not the transaction not happening.
  walletSilent: "your wallet did not respond - this may still have been sent",
};

export function sendStateLabel(state: SendState): string {
  return STATE_LABEL[state];
}

/**
 * The explorer link for a send, or `null` when the hash cannot address a transaction.
 *
 * An explorer link is a CLAIM, so it is offered only for a well-formed 32-byte hash - the repo's
 * standing rule, and the shared `txExplorerHref` is where it lives rather than a URL composed here.
 * A wallet that returned something malformed gets its value rendered inert instead of a dead link.
 */
export function sendExplorerHref(record: SendRecord): string | null {
  // No hash, no link. An explorer URL built from a missing hash would be a claim that a transaction
  // exists to look at, which is precisely what these two states cannot say.
  if (!record.hash) return null;
  return txExplorerHref({ txHash: record.hash });
}
