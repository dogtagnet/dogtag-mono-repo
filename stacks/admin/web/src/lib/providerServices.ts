/**
 * The pure decisions behind the service half of the registrar screen.
 *
 * Extracted from the page for the same reason `providerIdentity.ts` was: what a screen SAYS about a
 * lifecycle state is a rule worth pinning independently of how it is laid out, and a rule that only
 * exists inside JSX can only be tested by rendering it.
 */

import type {
  AttachPreflightResp,
  CurrentPointerRead,
  ProviderStanding,
  ServiceEffective,
} from "@dogtag/ui";

/**
 * The plan key for an attach: any change to it retires a reviewed preflight.
 *
 * The address is the ONLY input an admin types, but the preflight's answer also depends on the
 * provider it would be attached to, so both are in the key. Same rule as `registrationKey`: a send
 * addresses the values that were CHECKED, never whatever the form holds when the button is pressed.
 */
export function attachKey(providerId: string, serviceAddress: string): string {
  return `${providerId.trim().toLowerCase()}|${serviceAddress.trim().toLowerCase()}`;
}

/** One lifecycle term as the screen renders it. */
export interface EffectiveTerm {
  key: string;
  label: string;
  /** `null` is could-not-establish and is neither of the other two. */
  held: boolean | null;
  /** What to DO about it - different for every term, which is why they are never pre-ANDed. */
  remedy: string;
}

/**
 * The five terms `canIssue` folds, each with its own remedy.
 *
 * They are reported APART rather than as one "can issue" bool because each has a different fix, and
 * a single bool would tell an admin that something is wrong while withholding the only thing that
 * says what to do about it. `unavailable` yields all five as `null` - could-not-check, never "no".
 *
 * `currentPointer` is read for the LAST term's remedy only and is never folded into any term's
 * `held`: the chain already folds the pointer inside `hasActiveIssuer`, so ANDing it here a second
 * time would make one of the five stop meaning what the wire says it means.
 */
export function effectiveTerms(
  effective: ServiceEffective | { unavailable: string } | undefined,
  currentPointer?: CurrentPointerRead,
): EffectiveTerm[] {
  const unknown = effective === undefined || "unavailable" in effective;
  const e = unknown ? null : (effective as ServiceEffective);
  // By the time the last term is the FIRST failing one the other four hold, so the only two causes
  // it can be reporting are "nobody is granted" and "the provider has not repointed" - and only the
  // second has a remedy the registrar cannot perform. But the pointer read can itself have failed,
  // which is a THIRD state: naming either cause there would be a definite remedy derived from a
  // read that never happened.
  const pointer = currentPointer && currentPointer.state === "resolved" ? currentPointer : null;
  // The grant half is REGISTRY-WIDE: rights are keyed on the address, so "some key holds the issue
  // right" is a property of the whole registry rather than of this service. Once any address holds
  // it, this term can only be false because of the lifecycle half - so the grant remedy names the
  // right honestly (any address, no service) instead of implying a per-service grant that does not
  // exist and would change nothing here.
  const issuerRemedy = !pointer
    ? "Either no address anywhere holds the issue right, or the provider has not published this " +
      "contract as its current service - the pointer read did not complete, so which of the two " +
      "applies is not established here."
    : pointer.isCurrent
      ? "No address holds the issue right yet. Grant it to the key that will sign - the grant is " +
        "on that ADDRESS and names no service, so it reaches every service in standing."
      : "The provider has not published this contract as its current service for this record " +
        "type. Only the provider can repoint it, on their own portal - the registrar cannot do " +
        "it for them.";
  return [
    {
      key: "providerStanding",
      label: "Provider active",
      held: e ? e.providerStanding === "active" : null,
      remedy: "Set the provider's standing to Active.",
    },
    {
      key: "serviceStanding",
      label: "Service active",
      held: e ? e.serviceStanding === "active" : null,
      remedy: "Set this service's standing to Active - attaching leaves it Pending.",
    },
    {
      key: "factoryActive",
      label: "Factory generation live",
      held: e ? e.factoryActive : null,
      // Deprecation has no reactivation in the contract, so this one is terminal rather than a fix.
      remedy: "The generation that deployed this contract has been deprecated. This is terminal.",
    },
    {
      key: "ownerConfirmed",
      label: "Owner confirmed",
      held: e ? e.ownerConfirmed : null,
      remedy:
        "The contract's live owner differs from the confirmed one - a handover completed on the " +
        "clone and needs confirmServiceOwner before any write or issuance resumes.",
    },
    {
      key: "hasActiveIssuer",
      // NOT "has an issuer": the chain re-folds the owner, both standings and the provider's
      // current pointer into this one, so it answers "may somebody issue through this now".
      label: "An issuer may issue now",
      held: e ? e.hasActiveIssuer : null,
      remedy: issuerRemedy,
    },
  ];
}

/** Tone for a tri-state term. `null` is warning, never the failure red - it is not a refusal. */
export function termTone(held: boolean | null): "success" | "danger" | "warning" {
  if (held === null) return "warning";
  return held ? "success" : "danger";
}

/**
 * What is blocking this service from issuing, as ONE sentence, or null when nothing is.
 *
 * Reports the FIRST unheld term rather than folding them, so the sentence always names an actionable
 * remedy. A term that could not be established yields its own sentence and never a blocking claim.
 */
export function issuanceBlocker(terms: EffectiveTerm[]): string | null {
  const unknown = terms.find((t) => t.held === null);
  if (unknown) {
    return "Some lifecycle terms could not be read, so whether this service can issue is not established.";
  }
  const failing = terms.find((t) => t.held === false);
  return failing ? failing.remedy : null;
}

/**
 * Whether an attach preflight authorises a send.
 *
 * `couldNotRun` DOES authorise it: a read we could not complete is not evidence the contract would
 * refuse, and a preflight stricter than the chain is a worse defect than no preflight - it refuses
 * work the contract would have accepted, with no way for the admin to override.
 */
export function preflightPermitsSend(p: AttachPreflightResp | null): boolean {
  if (!p) return false;
  return p.verdict !== "refused";
}

/** The `generationId` a send must carry - present only when the probe actually resolved one. */
export function resolvedGenerationId(p: AttachPreflightResp | null): string | null {
  return p && p.generation.state === "resolved" ? p.generation.generationId : null;
}

/** The live `owner()` the guard is prefilled from - present only when the metadata read resolved. */
export function resolvedOwner(p: AttachPreflightResp | null): string | null {
  return p && p.metadata.state === "resolved" ? p.metadata.owner : null;
}

/**
 * Whether the ATTACH button may send at all.
 *
 * Both resolved values are REQUIRED even though `couldNotRun` permits a send in principle: the
 * calldata cannot be built without a generation id and an expected owner, and inventing either would
 * send a transaction addressing something nobody checked. So a `couldNotRun` whose missing half is
 * one of these two is offered as a RETRY rather than as a send.
 */
export function attachSendable(p: AttachPreflightResp | null): boolean {
  return preflightPermitsSend(p) && !!resolvedGenerationId(p) && !!resolvedOwner(p);
}

/** Standing badge tone, shared by provider and service rows - one standing vocabulary, one mapping. */
export const STANDING_TONE: Record<
  ProviderStanding,
  "neutral" | "warning" | "success" | "danger" | "outline"
> = {
  none: "outline",
  // PENDING is WARNING rather than neutral: it is what registration AND attachment produce, and a
  // record left there can do nothing at all. Painting it neutral is what makes it easy to miss.
  pending: "warning",
  active: "success",
  suspended: "danger",
  retired: "danger",
};
