/**
 * The provider self-service surface's shared vocabulary (registry-plan S-15).
 *
 * Every check this surface performs reports `pass | fail | could-not-run`, and a could-not-run
 * states its reason. That is not decoration: this codebase has four separate recorded instances of
 * an unanswerable question being collapsed into a definite verdict, the worst of which declared
 * genuine credentials forged on an unauthenticated endpoint. A provider surface has the same shape
 * of hazard pointed at the provider's own property - "we could not read the chain" rendered as
 * "your contract is not genuine" is an accusation nobody was in a position to make.
 *
 * {@link CheckOutcome} is imported rather than restated so the bench, the verifier and this surface
 * cannot drift into three spellings of the same three states. The import is type-only, so nothing
 * from the bench's module graph is pulled in at runtime.
 */

import type { CheckOutcome } from "../wallet/verificationBench";

export type { CheckOutcome };

/**
 * Where a candidate contract stands relative to this provider, as a single explicit state.
 *
 * The lifecycle is modelled because the repoint flow is NOT one provider action, and a surface that
 * assumed it was would describe a journey nobody can walk. `attachService` is `onlyOwner` on the
 * core, so the real sequence is:
 *
 *   provider deploys (`createIssuer`) -> REGISTRAR attaches (`attachService`) -> provider repoints
 *
 * `deployed` is therefore a first-class waiting state with a real next step that belongs to someone
 * else, not an error and not a half-attached limbo.
 *
 * `unknown` is the could-not-run of the lifecycle itself and must never be rendered as `notAClone`:
 * one says the chain refused the address, the other says we failed to ask.
 */
export type CloneLifecycle =
  /** A read failed, so nothing about this address was established. */
  | "unknown"
  /** The configured factory did not deploy this address. A definite refusal. */
  | "notAClone"
  /** Genuinely ours, and not yet attached to any provider. The registrar's move is next. */
  | "deployed"
  /** Attached to this provider, but not the provider's current pointer for its record type. */
  | "attached"
  /** Attached and selected: new credentials of this record type anchor here. */
  | "current"
  /** Genuine and attached, but to a DIFFERENT provider. Provenance is not attribution. */
  | "foreign";

export type ProviderCheckId =
  /** Did our own configured factory deploy this address? */
  | "clone-provenance"
  /**
   * WHO owns the clone right now. A REPORT, not an authorization - see `clone-write-authority`.
   * Owning the contract and being allowed to select it are different facts, and the chain admits a
   * delegate who owns nothing.
   */
  | "clone-control"
  /** Would the CHAIN accept a repoint from this key? Composed from `canWriteService`, never re-derived. */
  | "clone-write-authority"
  /** Is the clone attached to this provider in the core? */
  | "clone-attachment"
  /** Does the clone's lifecycle currently permit writes at all? */
  | "clone-standing"
  /** Is the provider itself cleared to act? */
  | "provider-standing"
  /** May this key deploy a clone for this provider and record type? */
  | "deploy-authority"
  /** Is the domain resolver approved fleet-wide and selected by this service? */
  | "domain-resolver-live"
  /**
   * Which typed resolvers may this provider choose from? Read from the chain's own allowlist, never
   * from a bundled address, so one approved later becomes choosable with no rebuild.
   */
  | "resolver-choices"
  /** Which resolver is selected right now - including the withdrawn-underneath case? */
  | "resolver-selection-current"
  /** Is the resolver the provider picked one the registry would accept? */
  | "resolver-choice-approved"
  /** Would a selection from this key be accepted? Composed from `canWriteProvider`/`canWriteService`. */
  | "resolver-write-authority"
  /** Would this selection change anything, or would the registry refuse it as a no-op? */
  | "resolver-selection-change"
  /** Would a domain write from this key be accepted? */
  | "domain-write-authority"
  /** Is the directory resolver approved fleet-wide and selected by this provider? */
  | "directory-resolver-live"
  /** Is the location input usable, or deliberately absent? */
  | "directory-location"
  /** What is already published, so a re-publish can UPDATE rather than append a second pin. */
  | "directory-listing-state"
  /** Is there anything to publish at all? */
  | "directory-contents";

/**
 * One check's answer.
 *
 * `couldNotRunReason` is present if and ONLY if `outcome === "could-not-run"` - the same invariant
 * `BenchCheck` carries, so a renderer cannot show a reason beside a pass or omit one beside a
 * could-not-run. {@link providerCheck} is the only sanctioned constructor and enforces it.
 */
export interface ProviderCheck {
  id: ProviderCheckId;
  /** The check as a plain-language question, not a jargon label. */
  question: string;
  outcome: CheckOutcome;
  /** What was observed. Always present, for every outcome. */
  finding: string;
  /** Why the check could not run. Present iff `outcome === "could-not-run"`. */
  couldNotRunReason?: string;
}

/** Build a check with the reason invariant enforced rather than remembered. */
export function providerCheck(
  id: ProviderCheckId,
  question: string,
  outcome: CheckOutcome,
  finding: string,
  couldNotRunReason?: string,
): ProviderCheck {
  if (outcome === "could-not-run") {
    if (!couldNotRunReason) {
      throw new Error(`provider check ${id}: a could-not-run must state its reason`);
    }
    return { id, question, outcome, finding, couldNotRunReason };
  }
  if (couldNotRunReason) {
    throw new Error(`provider check ${id}: only a could-not-run may carry a reason`);
  }
  return { id, question, outcome, finding };
}

/**
 * The verdict over a set of checks.
 *
 * `refused` outranks `indeterminate`, deliberately: a definite failure is not softened by a
 * neighbouring non-answer. The inverse - letting a could-not-run raise severity to `refused` - is
 * equally forbidden, which is why `indeterminate` exists as its own value rather than folding into
 * either neighbour.
 */
export type ProviderVerdict = "ready" | "refused" | "indeterminate";

export function foldVerdict(checks: readonly ProviderCheck[]): ProviderVerdict {
  if (checks.some((c) => c.outcome === "fail")) return "refused";
  if (checks.some((c) => c.outcome === "could-not-run")) return "indeterminate";
  return "ready";
}

/** Normalize an unknown thrown value into a one-line reason a could-not-run can carry. */
export function reasonFrom(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    // Chain-client errors are routinely paragraphs. One line is what a check row can render, and
    // the untruncated value is never the thing a reader needs here.
    const firstLine = error.message.split("\n")[0]!.trim();
    return firstLine.length > 200 ? `${firstLine.slice(0, 197)}...` : firstLine;
  }
  return fallback;
}
