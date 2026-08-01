/**
 * Flow 3: the provider claims a domain through the service-domain resolver (registry-plan S-15).
 *
 * The captain's ruling this implements: DOMAIN STATE IS TYPED, not an overloaded empty string.
 * "Nobody has said", "there is deliberately no domain" and "a claim was withdrawn" are three
 * different facts, and the predecessor could express only the first of them - as `""`, which every
 * consumer then rendered as "this issuer has published no domain on-chain", a publication decision
 * the issuer may never have made.
 *
 * So this module never returns a bare string. {@link describeDisposition} is the only sanctioned way
 * to turn a record into words, and it has a distinct sentence for each of the four states.
 */

import {
  DomainDisposition,
  type Address,
  type DomainClaimStanding,
  type ProviderChainReader,
} from "./readers";
import {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type ProviderCheck,
  type ProviderVerdict,
} from "./types";

/** `ServiceDomainResolver.MAX_DOMAIN_LENGTH`. */
export const MAX_DOMAIN_LENGTH = 253;

export type DomainValidation = { ok: true; domain: string } | { ok: false; reason: string };

/**
 * The client-side mirror of `ServiceDomainResolver._requireValidDomain`.
 *
 * It REJECTS rather than normalizes, exactly as the contract does, so what a resolver reads is
 * exactly what the writer intended. This is deliberately not a convenience lowercaser: silently
 * "fixing" `Seaport-Vet.SG` would publish a claim the provider did not type, and the contract would
 * accept it, so the provider would never learn their input was altered.
 *
 * It is a preflight, not a gate - the contract re-checks every rule. What it buys is the message
 * beside the field instead of an opaque `BadDomain` revert.
 */
export function validateDomain(input: string): DomainValidation {
  const domain = input.trim();
  if (domain.length === 0) return { ok: false, reason: "Enter a domain." };
  if (domain.length > MAX_DOMAIN_LENGTH) {
    return { ok: false, reason: `A domain may be at most ${MAX_DOMAIN_LENGTH} characters.` };
  }
  if (domain !== domain.toLowerCase()) {
    return {
      ok: false,
      reason: "Domains must be lowercase. Type it in lowercase rather than relying on it being corrected.",
    };
  }

  const labels = domain.split(".");
  if (labels.length < 2) {
    return { ok: false, reason: "Use a full domain such as clinic.example.sg, not a single word." };
  }
  for (const label of labels) {
    if (label.length === 0) {
      return { ok: false, reason: "A domain cannot have an empty part, a leading dot or a trailing dot." };
    }
    if (label.length > 63) return { ok: false, reason: "Each part of a domain may be at most 63 characters." };
    if (label.startsWith("-") || label.endsWith("-")) {
      return { ok: false, reason: "No part of a domain may start or end with a hyphen." };
    }
    if (!/^[a-z0-9-]+$/.test(label)) {
      return { ok: false, reason: "A domain may contain only lowercase letters, digits, dots and hyphens." };
    }
  }
  return { ok: true, domain };
}

/**
 * How a disposition reads, in the provider's own terms.
 *
 * Four distinct sentences for four distinct facts. Collapsing any two of them re-creates exactly
 * the defect the typed enum was introduced to remove.
 */
export function describeDisposition(disposition: DomainDisposition, domain: string): string {
  switch (disposition) {
    case DomainDisposition.UNSET:
      return "No domain has been published for this contract, and none has been declined either. Nobody has said.";
    case DomainDisposition.NO_DOMAIN:
      return "This contract deliberately publishes no domain. That is a positive statement, not an omission.";
    case DomainDisposition.CLAIMED:
      return `This contract claims ${domain}.`;
    case DomainDisposition.CLEARED:
      return "A domain was published for this contract and has since been withdrawn.";
  }
}

export interface DomainClaimAssessment {
  checks: ProviderCheck[];
  verdict: ProviderVerdict;
  /**
   * The current record, when it could be read. `undefined` means the read failed - which is not
   * `UNSET`. A surface that defaulted to `UNSET` here would state "nobody has said" on the strength
   * of a question nobody asked, which is the very conflation this flow exists to end.
   */
  standing?: DomainClaimStanding;
  /** How the current record reads. `undefined` for the same reason as `standing`. */
  description?: string;
  canWrite: boolean;
  nextStep: string;
}

export async function assessDomainClaim(
  service: Address,
  caller: Address,
  reader: ProviderChainReader,
): Promise<DomainClaimAssessment> {
  const checks: ProviderCheck[] = [];
  let standing: DomainClaimStanding | undefined;

  try {
    standing = await reader.domainClaimStanding(service);
    // The three resolver terms are reported as ONE check because they share a remedy the provider
    // cannot act on - all three are DogTag's to fix - while being kept nameable in the finding so a
    // support conversation can start from the right one.
    const live =
      standing.lineageRecognizesService &&
      standing.registryApprovesThisResolver &&
      standing.coreSelectsThisResolver;
    checks.push(
      providerCheck(
        "domain-resolver-live",
        "Is the domain register live for this contract?",
        live ? "pass" : "fail",
        live
          ? "The domain register is approved and selected for this contract."
          : [
              standing.lineageRecognizesService ? null : "the contract is not recognized by the issuer lineage",
              standing.registryApprovesThisResolver ? null : "the domain register is not approved",
              standing.coreSelectsThisResolver ? null : "this contract has not selected the domain register",
            ]
              .filter(Boolean)
              .join("; "),
      ),
    );

    // The FOURTH term, reported separately on purpose. It is outside the resolver verdict because a
    // frozen contract is a different situation with a different remedy, and because a `CLAIMED`
    // record beside a frozen contract is history rather than a live claim.
    if (!standing.serviceStandingEffective) {
      checks.push(
        providerCheck(
          "clone-standing",
          "Does this contract's standing still allow changes?",
          "fail",
          "This contract is frozen, so its domain record can no longer be changed. Any domain shown for "
            + "it is history, not a current claim.",
        ),
      );
    }
  } catch (error) {
    checks.push(
      providerCheck(
        "domain-resolver-live",
        "Is the domain register live for this contract?",
        "could-not-run",
        "The domain record could not be read.",
        reasonFrom(error, "the claimStanding read failed"),
      ),
    );
  }

  try {
    const canWrite = await reader.canWriteDomain(service, caller);
    checks.push(
      providerCheck(
        "domain-write-authority",
        "May your key publish a domain for this contract?",
        canWrite ? "pass" : "fail",
        canWrite
          ? "Your key may publish a domain for this contract."
          : "Your key may not publish a domain for this contract. The owner on file, or a delegate they "
            + "authorised, may.",
      ),
    );
  } catch (error) {
    checks.push(
      providerCheck(
        "domain-write-authority",
        "May your key publish a domain for this contract?",
        "could-not-run",
        "Write authority could not be read.",
        reasonFrom(error, "the canWriteDomain read failed"),
      ),
    );
  }

  const verdict = foldVerdict(checks);
  return {
    checks,
    verdict,
    ...(standing ? { standing } : {}),
    ...(standing ? { description: describeDisposition(standing.disposition, standing.domain) } : {}),
    canWrite: verdict === "ready",
    nextStep: domainNextStep(verdict, standing),
  };
}

function domainNextStep(verdict: ProviderVerdict, standing?: DomainClaimStanding): string {
  if (verdict === "indeterminate") {
    return "Some checks could not run, so the current domain record is not known. Try again once the connection is back.";
  }
  if (verdict === "refused") return "The domain record cannot be changed right now. The failed checks above say why.";
  if (standing?.disposition === DomainDisposition.CLAIMED) {
    return "You can publish a different domain, re-affirm this one, or withdraw the claim.";
  }
  return "You can publish a domain, or state that this contract deliberately has none.";
}

/**
 * Whether `clearDomain` is available.
 *
 * The contract refuses a withdrawal unless a claim actually exists, because `CLEARED` asserts that
 * one WAS withdrawn - recording a withdrawal from `UNSET` would be a false statement. Offering the
 * button anyway would produce a revert the provider cannot interpret.
 */
export function canWithdraw(standing: DomainClaimStanding | undefined): boolean {
  return standing?.disposition === DomainDisposition.CLAIMED;
}

export { DomainDisposition };
