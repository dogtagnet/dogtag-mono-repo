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
 * FOUR STATES, and the fourth is the one that is easy to collapse:
 *
 *   submitted - the wallet returned a hash. Nothing about the outcome is known YET.
 *   succeeded - a receipt was read and its status is success.
 *   reverted  - a receipt was read and its status is reverted. A definite failure.
 *   unknown   - the receipt could not be read (timeout, RPC failure). NEITHER neighbour: a
 *               transaction we could not follow is not one that failed, and it is certainly not one
 *               that worked. It carries its reason, exactly like a `could-not-run` check.
 *
 * `submitted` must never render as "published"/"deployed", and `unknown` must never render as either
 * of the two definite states.
 */

import { txExplorerHref } from "../chain/provenance";

export type SendState = "submitted" | "succeeded" | "reverted" | "unknown";

export interface SendRecord {
  /** The transaction hash the wallet returned. */
  hash: string;
  /** What was being attempted, in the provider's terms. Never a claim that it happened. */
  what: string;
  state: SendState;
  /** Why the outcome could not be established. Present if and ONLY if `state === "unknown"`. */
  unknownReason?: string;
}

/**
 * Build a record with the reason invariant enforced rather than remembered.
 *
 * Same rule and same reason as `providerCheck`: a renderer must not be able to show a reason beside
 * a settled outcome, or omit one beside an unsettled one.
 */
export function sendRecord(
  hash: string,
  what: string,
  state: SendState,
  unknownReason?: string,
): SendRecord {
  if (state === "unknown") {
    if (!unknownReason) throw new Error("a send record with an unknown outcome must state its reason");
    return { hash, what, state, unknownReason };
  }
  if (unknownReason) throw new Error("only an unknown outcome may carry a reason");
  return { hash, what, state };
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
  // Never "sent" alone - the point of the word is that the outcome is still open.
  submitted: "submitted, outcome not yet known",
  succeeded: "succeeded",
  reverted: "reverted on chain",
  unknown: "outcome could not be established",
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
  return txExplorerHref({ txHash: record.hash });
}
