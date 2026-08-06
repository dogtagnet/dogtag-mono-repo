/**
 * The provider's own half of the typed-resolver pair: SELECTING one.
 *
 * THE GAP THIS CLOSES, found by a captain walking the guide. Flows 3 and 4 of the provider page
 * could not be completed by anybody, because both are gated on a selection the provider had no way
 * to make. `ProviderRegistry.setDirectoryResolver` and `setDomainResolver` existed only in the
 * contracts and their forge tests - no product surface anywhere, in either portal or any backend
 * route - while the REGISTRAR's half (`setResolverApproved`) shipped on the admin Providers page. So
 * a vet could not publish a listing, which means they never appear in the directory and nothing a
 * pet owner searches can find them; and they could not claim a domain. Measured on the live set
 * before this shipped: both resolvers approved fleet-wide, and every provider's `directoryResolver`
 * and every service's `domainResolver` still the zero address.
 *
 * Same shape as the signer-whitelist gap: the mechanism existed, the door did not.
 *
 * THE WRITE IS A WALLET TRANSACTION AND THERE IS DELIBERATELY NO BACKEND ROUTE, for the same reason
 * `setIssuanceAllowed` has none. The core admits the provider's CONTROLLER (or a delegate holding
 * the matching permission bit); a backend is neither, and an operator session proves "staff of this
 * shop" rather than "controller of this provider record". A backend that could select would need
 * that authority itself, which hands one key both halves of a two-party arrangement.
 *
 * Every read goes through the injected {@link ProviderChainReader}, and the choice list comes from
 * the CHAIN (`resolverCount` / `resolverPage` / `isResolverApproved`) rather than from a bundled
 * address, so a resolver the registrar approves later needs no rebuild to become choosable.
 */

import {
  ResolverKind,
  Standing,
  ZERO_ADDR,
  type Address,
  type HexWord,
  type ProviderChainReader,
  type ResolverListing,
} from "./readers";
import {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type ProviderCheck,
  type ProviderVerdict,
} from "./types";

/**
 * Which of the two selections a plan is about.
 *
 * They are separate because they are KEYED differently, not merely because there are two contracts:
 * a directory resolver is chosen once per PROVIDER, because a provider has one listing however many
 * issuing contracts it holds, while a domain resolver is chosen per SERVICE, because a domain
 * belongs to one contract. Collapsing them would put the provider's address and logo in as many
 * copies as it has contracts, with nothing able to say which is real - the same reason
 * `ProviderDirectory`'s profile anchor deliberately carries no `name` field.
 */
export type ResolverSelectionScope = "directory" | "domain";

/** Human words for a scope, used in every sentence this module builds. */
const SCOPE_LABEL: Readonly<Record<ResolverSelectionScope, string>> = {
  directory: "provider directory",
  domain: "domain register",
};

/** Which `ResolverKind` a scope reads from. */
export function kindForScope(scope: ResolverSelectionScope): ResolverKind {
  return scope === "directory" ? ResolverKind.DIRECTORY : ResolverKind.DOMAIN;
}

/**
 * What the core currently records, as an explicit state.
 *
 * `selectedButPulled` IS REACHABLE AND MUST HAVE ITS OWN NAME. The core never clears a stored
 * selection when the registrar deapproves a resolver - `setResolverApproved` only flips the approval
 * mapping - and `ProviderRegistry.setDomainResolver`'s own doc comment says the stored selector is a
 * HISTORICAL record for exactly this reason. So a provider can be pointed at a register that answers
 * nothing, and that is a different fact from having selected nothing: one needs repairing, the other
 * has never been started. Rendering both as "not selected" is the same conflation
 * `DomainDisposition` was introduced to end one layer down.
 *
 * A read that FAILED is none of these three - it is the absence of this value entirely.
 */
export type ResolverSelectionState =
  | { kind: "none" }
  | { kind: "selected"; resolver: Address }
  | { kind: "selectedButPulled"; resolver: Address };

export interface ResolverSelectionPlan {
  scope: ResolverSelectionScope;
  /**
   * What the write addresses - a `bytes20` provider id for `directory`, a contract address for
   * `domain`. Captured from the request, so a send acts on what was CHECKED rather than on whatever
   * the form holds by the time the button is pressed.
   */
  subject: HexWord;
  /**
   * The register the provider chose, captured for the same reason.
   *
   * `ZERO_ADDR` means "stop using one", which the chain always accepts - `setDirectoryResolver`
   * checks approval only for a non-zero resolver.
   */
  chosen: Address;
  checks: ProviderCheck[];
  verdict: ProviderVerdict;
  /**
   * What may be chosen: APPROVED entries only.
   *
   * `undefined` means the list could not be read, which is not the same as an empty list and must
   * never be rendered as one - an empty array says the registrar has approved nothing, a statement
   * about DogTag that a failed read is in no position to make.
   */
  choices?: readonly Address[];
  /**
   * Entries the registry lists whose approval has been withdrawn.
   *
   * Carried so a short choice list is legible rather than mysterious, and never offered as a choice:
   * selecting one reverts `ResolverNotApproved`.
   */
  withdrawn?: readonly Address[];
  /** The current selection. `undefined` means the read failed - which is NOT `{ kind: "none" }`. */
  selection?: ResolverSelectionState;
  /** How the current selection reads. `undefined` for the same reason as `selection`. */
  description?: string;
  canSelect: boolean;
  nextStep: string;
}

/**
 * How a selection state reads, in the provider's own terms.
 *
 * Three distinct sentences for three distinct facts, and the third is the one that would otherwise
 * be invisible: a register whose approval was pulled underneath a live selection.
 */
export function describeSelection(
  scope: ResolverSelectionScope,
  state: ResolverSelectionState,
): string {
  const label = SCOPE_LABEL[scope];
  switch (state.kind) {
    case "none":
      return `No ${label} is selected, so nothing you publish through it would be read.`;
    case "selected":
      return `${shortHex(state.resolver)} is selected, and DogTag still approves it.`;
    case "selectedButPulled":
      return (
        `${shortHex(state.resolver)} is selected, and DogTag has since withdrawn its approval - so it ` +
        `currently answers nothing. Your published content is not deleted; selecting an approved ` +
        `${label} is what makes it readable again.`
      );
  }
}

/** First and last four bytes, for a sentence that has to name an address without being all address. */
function shortHex(value: string): string {
  return value.length > 14 ? `${value.slice(0, 10)}…${value.slice(-4)}` : value;
}

/**
 * Whether the chain would refuse this write as a no-op.
 *
 * `setDirectoryResolver` and `setDomainResolver` both `revert NoChange()` when the value is already
 * stored, so offering the button anyway produces a revert the provider cannot interpret - the same
 * reason the withdraw-domain button is gated on a claim existing. Returns the sentence to show, or
 * `null` when the write really would change something.
 *
 * `undefined` for `selection` means the current value was not established, and then this cannot
 * answer either: refusing on the strength of a read that failed would block a write the chain would
 * have accepted.
 */
export function selectionChangeRefusal(
  scope: ResolverSelectionScope,
  selection: ResolverSelectionState | undefined,
  chosen: Address,
): string | null {
  if (!selection) return null;
  const current = selection.kind === "none" ? ZERO_ADDR : selection.resolver;
  if (!sameAddress(current, chosen)) return null;
  const label = SCOPE_LABEL[scope];
  return selection.kind === "none"
    ? `No ${label} is selected already, so there is nothing to stop using.`
    : `${shortHex(chosen)} is already your ${label}. The registry refuses a write that would change nothing, so there is nothing to send.`;
}

/** Case-insensitive address comparison. Hex casing is a checksum, never an identity. */
function sameAddress(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

/** Whether "stop using one" is on offer: only when something is actually selected. */
export function canStopUsing(selection: ResolverSelectionState | undefined): boolean {
  return selection !== undefined && selection.kind !== "none";
}

interface SelectionRequest {
  scope: ResolverSelectionScope;
  subject: HexWord;
  chosen: Address;
  caller: Address;
  reader: ProviderChainReader;
}

/**
 * Preflight one resolver selection.
 *
 * The check ORDER is not cosmetic: standing is read and reported BEFORE authority, because
 * `canWriteProvider` returns false on `standing != ACTIVE` before it looks at the caller at all, and
 * `canWriteService` does the same on ineffective standing or an unconfirmed owner. So a bare `false`
 * from either says nothing about the operator's key - and for a provider record that is the state
 * EVERY new provider starts in, since `registerProvider` writes `PENDING` and activation is a
 * separate registrar call. Reporting that as "your key may not" is an accusation nobody was in a
 * position to make.
 */
export async function assessResolverSelection(
  request: SelectionRequest,
): Promise<ResolverSelectionPlan> {
  const { scope, subject, chosen, caller, reader } = request;
  const label = SCOPE_LABEL[scope];
  const checks: ProviderCheck[] = [];
  const stopping = sameAddress(chosen, ZERO_ADDR);

  // ---- what may be chosen -----------------------------------------------------------------------
  let listings: readonly ResolverListing[] | undefined;
  try {
    listings = await reader.approvedResolvers(kindForScope(scope));
    const approved = listings.filter((l) => l.approved);
    const pulled = listings.length - approved.length;
    checks.push(
      providerCheck(
        "resolver-choices",
        `Which ${label} may you choose from?`,
        "pass",
        approved.length === 0
          ? `DogTag has approved no ${label}${
              pulled > 0 ? `, and has withdrawn ${pulled} it previously approved` : ""
            }. There is nothing to select until it approves one.`
          : `DogTag approves ${approved.length} ${label}${approved.length === 1 ? "" : "s"}${
              pulled > 0 ? `, and lists ${pulled} more whose approval it has withdrawn` : ""
            }.`,
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "resolver-choices",
        `Which ${label} may you choose from?`,
        "could-not-run",
        `The list of ${label}s you may choose from could not be read.`,
        reasonFrom(error, "the resolver list read failed"),
      ),
    );
  }

  // ---- what is selected now ---------------------------------------------------------------------
  //
  // Read from the core, and its approval read from the SAME listing above rather than from a second
  // question: a resolver is pushed onto the list the first time it is approved and never removed, so
  // an address absent from the list has never been approved and cannot be selected. Deriving it here
  // therefore agrees with `isResolverApproved` by construction, for every state the contract can
  // reach.
  let selection: ResolverSelectionState | undefined;
  try {
    const stored =
      scope === "directory"
        ? (await reader.provider(subject)).directoryResolver
        : (await reader.service(subject as Address)).domainResolver;
    selection = classifySelection(stored, listings);
    if (selection) {
      checks.push(
        providerCheck(
          "resolver-selection-current",
          `Which ${label} is selected now?`,
          "pass",
          describeSelection(scope, selection),
        ),
      );
    } else {
      // The selector READ succeeded and names a register, but whether that register is still approved
      // could not be established - so the state is genuinely between `selected` and
      // `selectedButPulled`, and reporting either would be inventing one. The address is named
      // anyway, because it is the one thing we did learn.
      checks.push(
        providerCheck(
          "resolver-selection-current",
          `Which ${label} is selected now?`,
          "could-not-run",
          `${shortHex(stored)} is selected, and whether DogTag still approves it is not established.`,
          `the list of approved ${label}s could not be read, so a selected register cannot be told apart from a withdrawn one`,
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "resolver-selection-current",
        `Which ${label} is selected now?`,
        "could-not-run",
        `The ${label} currently selected could not be read.`,
        reasonFrom(error, "the selection read failed"),
      ),
    );
  }

  // ---- is the choice one the chain would accept -------------------------------------------------
  if (stopping) {
    // The contract checks approval only for a NON-ZERO resolver, so stopping is always permitted.
    // Reported as a real row rather than skipped: a check that is not pushed cannot be reasoned about
    // by a reader, and its absence would read as an oversight.
    checks.push(
      providerCheck(
        "resolver-choice-approved",
        "Is what you picked something you may select?",
        "pass",
        `Stopping is always allowed. The registry only checks approval for a ${label} you switch TO.`,
      ),
    );
  } else if (listings === undefined) {
    checks.push(
      providerCheck(
        "resolver-choice-approved",
        "Is what you picked something you may select?",
        "could-not-run",
        "Whether the register you picked may be selected is not established.",
        `the list of approved ${label}s could not be read, so this cannot be answered either way`,
      ),
    );
  } else {
    const entry = listings.find((l) => sameAddress(l.resolver, chosen));
    if (entry?.approved) {
      checks.push(
        providerCheck(
          "resolver-choice-approved",
          "Is what you picked something you may select?",
          "pass",
          `${shortHex(chosen)} is approved by DogTag, so the registry will accept it.`,
        ),
      );
    } else {
      // TWO DIFFERENT REFUSALS, and they send the provider to different places. A listed-but-pulled
      // address was approved once and DogTag withdrew it; an unlisted one was never approved at all,
      // which is what a hand-typed or copy-pasted address looks like.
      checks.push(
        providerCheck(
          "resolver-choice-approved",
          "Is what you picked something you may select?",
          "fail",
          entry
            ? `${shortHex(chosen)} was approved by DogTag and its approval has been withdrawn, so the registry would refuse it. Pick one from the approved list.`
            : `${shortHex(chosen)} is not a ${label} DogTag has approved, so the registry would refuse it. Pick one from the approved list.`,
        ),
      );
    }
  }

  // ---- standing, BEFORE authority ---------------------------------------------------------------
  const standing = await assessStanding(scope, subject, reader, checks);

  // ---- authority --------------------------------------------------------------------------------
  //
  // THE SAME TRI-STATE RULE THE DOMAIN AND REPOINT ROWS FOLLOW. Both `canWrite*` predicates fold the
  // standing terms above and short-circuit on them, so when standing does not hold they answer
  // `false` for the controller, for every delegate and for everybody else alike. Read as a fact
  // about the key, that says "your key may not" to somebody whose key is perfectly correct, about a
  // refusal only DogTag can clear.
  try {
    const canWrite =
      scope === "directory"
        ? await reader.canWriteProviderDirectoryResolver(subject, caller)
        : await reader.canWriteServiceDomainResolver(subject as Address, caller);
    if (canWrite) {
      checks.push(
        providerCheck(
          "resolver-write-authority",
          `May your key choose the ${label}?`,
          "pass",
          `Your key may choose the ${label}.`,
        ),
      );
    } else if (standing === true) {
      checks.push(
        providerCheck(
          "resolver-write-authority",
          `May your key choose the ${label}?`,
          "fail",
          scope === "directory"
            ? "Your key may not choose it. The controller on your provider record, or a delegate they authorised for this, may."
            : "Your key may not choose it. The owner of this contract, or a delegate they authorised for this, may.",
        ),
      );
    } else {
      checks.push(
        providerCheck(
          "resolver-write-authority",
          `May your key choose the ${label}?`,
          "could-not-run",
          "Whether your key may choose it is not established yet.",
          standing === false
            ? "the registry refuses this write from ANY key while the standing above does not hold, so this answer says nothing about your key - the row above it is the one to act on"
            : "the standing this write depends on could not be read, and it folds into this same answer, so a refusal here cannot be attributed to your key",
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "resolver-write-authority",
        `May your key choose the ${label}?`,
        "could-not-run",
        "Write authority could not be read.",
        reasonFrom(error, "the canWrite read failed"),
      ),
    );
  }

  // ---- would this change anything ---------------------------------------------------------------
  const noChange = selectionChangeRefusal(scope, selection, chosen);
  if (noChange) {
    checks.push(
      providerCheck(
        "resolver-selection-change",
        "Would this change what is selected?",
        "fail",
        noChange,
      ),
    );
  }

  const verdict = foldVerdict(checks);
  const approvedChoices = listings?.filter((l) => l.approved).map((l) => l.resolver);
  const withdrawnChoices = listings?.filter((l) => !l.approved).map((l) => l.resolver);
  return {
    scope,
    subject,
    chosen,
    checks,
    verdict,
    ...(approvedChoices ? { choices: approvedChoices } : {}),
    ...(withdrawnChoices ? { withdrawn: withdrawnChoices } : {}),
    ...(selection ? { selection } : {}),
    ...(selection ? { description: describeSelection(scope, selection) } : {}),
    canSelect: verdict === "ready",
    nextStep: selectionNextStep(scope, verdict, selection, listings),
  };
}

/**
 * Turn a stored selector into a state, using the listing to decide whether it is still approved.
 *
 * `listings === undefined` means we could not establish approval, so a NON-ZERO selector cannot be
 * classified as either `selected` or `selectedButPulled` - and this returns `undefined` rather than
 * guessing, which is what makes the caller report a could-not-run. A ZERO selector needs no listing:
 * nothing is selected, whatever the registry approves.
 */
function classifySelection(
  stored: Address,
  listings: readonly ResolverListing[] | undefined,
): ResolverSelectionState | undefined {
  if (sameAddress(stored, ZERO_ADDR)) return { kind: "none" };
  if (listings === undefined) return undefined;
  const entry = listings.find((l) => sameAddress(l.resolver, stored));
  return entry?.approved
    ? { kind: "selected", resolver: stored }
    : { kind: "selectedButPulled", resolver: stored };
}

/**
 * The standing terms this write folds, reported as their own rows.
 *
 * Returns `true` when standing holds, `false` when it definitely does not, and `undefined` when it
 * could not be read - the three values the authority row above needs to know which of its own three
 * answers it is entitled to give.
 */
async function assessStanding(
  scope: ResolverSelectionScope,
  subject: HexWord,
  reader: ProviderChainReader,
  checks: ProviderCheck[],
): Promise<boolean | undefined> {
  if (scope === "directory") {
    try {
      const record = await reader.provider(subject);
      const active = record.standing === Standing.ACTIVE;
      checks.push(
        providerCheck(
          "provider-standing",
          "Is your provider record cleared to act?",
          active ? "pass" : "fail",
          active
            ? "Your provider record is active."
            : record.standing === Standing.PENDING
              ? "Your provider record is registered and still pending activation. DogTag activates it; nothing you have set up is wrong, and there is nothing here for you to change."
              : `Your provider record is ${STANDING_WORD[record.standing]}, so the registry accepts no writes into it.`,
        ),
      );
      return active;
    } catch (error) {
      checks.push(
        providerCheck(
          "provider-standing",
          "Is your provider record cleared to act?",
          "could-not-run",
          "Your provider record could not be read.",
          reasonFrom(error, "the provider read failed"),
        ),
      );
      return undefined;
    }
  }

  try {
    const e = await reader.effectiveService(subject as Address);
    // FOUR TERMS, NOT FIVE. `canWriteService` folds provider standing, service standing, factory
    // generation and the confirmed-owner match - and deliberately NOT `hasActiveIssuer`, which is
    // about whether the contract can anchor credentials and has nothing to do with choosing a
    // register. Folding it in would refuse a selection the chain would accept.
    const blockers = [
      e.providerStanding === Standing.ACTIVE ? null : `your provider record is ${STANDING_WORD[e.providerStanding]}`,
      e.serviceStanding === Standing.ACTIVE ? null : `this contract is ${STANDING_WORD[e.serviceStanding]}`,
      e.factoryActive ? null : "the generation that deployed this contract has been retired",
      e.ownerConfirmed ? null : "this contract's owner on file does not match its live owner, so it is held pending confirmation",
    ].filter((b): b is string => b !== null);
    const ok = blockers.length === 0;
    checks.push(
      providerCheck(
        "clone-standing",
        "Does this contract's standing allow it to choose one?",
        ok ? "pass" : "fail",
        ok
          ? "This contract's standing allows the choice."
          : // "held pending confirmation" is NOT "frozen", and the difference is what a provider
            // needs: a pending owner confirmation is cleared by `confirmServiceOwner`, while a
            // retired generation or a retired contract is cleared by nothing.
            `The registry accepts no register choice for this contract: ${blockers.join("; ")}.`,
      ),
    );
    return ok;
  } catch (error) {
    checks.push(
      providerCheck(
        "clone-standing",
        "Does this contract's standing allow it to choose one?",
        "could-not-run",
        "This contract's standing could not be read.",
        reasonFrom(error, "the effectiveService read failed"),
      ),
    );
    return undefined;
  }
}

/** A standing value as one word, for a sentence that has to name it. */
const STANDING_WORD: Readonly<Record<Standing, string>> = {
  [Standing.NONE]: "not registered",
  [Standing.PENDING]: "pending activation",
  [Standing.ACTIVE]: "active",
  [Standing.SUSPENDED]: "suspended",
  [Standing.RETIRED]: "retired",
};

function selectionNextStep(
  scope: ResolverSelectionScope,
  verdict: ProviderVerdict,
  selection: ResolverSelectionState | undefined,
  listings: readonly ResolverListing[] | undefined,
): string {
  const label = SCOPE_LABEL[scope];
  if (verdict === "indeterminate") {
    return `Some checks could not run, so what is selected is not known. Try again once the connection is back.`;
  }
  if (verdict === "refused") {
    return `This choice cannot be made right now. The failed checks above say why.`;
  }
  if (selection?.kind === "selectedButPulled") {
    return `Selecting an approved ${label} replaces the withdrawn one and makes your published content readable again.`;
  }
  const approvedCount = listings?.filter((l) => l.approved).length ?? 0;
  if (selection?.kind === "none" && approvedCount > 0) {
    return `Selecting a ${label} is what turns the rest of this flow on.`;
  }
  return `You can switch to a different ${label}, or stop using one.`;
}

export { ResolverKind };
