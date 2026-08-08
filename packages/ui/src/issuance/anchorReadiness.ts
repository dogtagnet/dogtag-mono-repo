// Dog-tag ANCHOR readiness: the ONE decision behind the vet portal's Register pet gate and the
// Setup page's persistent warning.
//
// The defect this closes (measured on a live walk, 2026-08-07): with `PROFILE_ISSUER_ADDR` unset
// the portal allocated a tag, drew a QR, and had a second human scan it — and the refusal the
// backend had known since boot surfaced as a 503 on the OWNER'S PHONE, where nobody can act on it.
// The boot-time warning had scrolled past in a terminal the operator never reads again. So the
// same config facts ride on `GET /health` (`HealthResp.dogTagIssuance`), and this module turns
// them into the three-state answer both screens render.
//
// Three states, and the split is load-bearing:
// - `blocked` is a DEFINITE backend answer ("not configured"), and it is the only state that may
//   replace the Register pet form with a refusal. Its copy names the missing contract in the
//   operator's vocabulary and points at the fixing step; the env variable rides in parentheses as
//   the detail, never as the whole instruction.
// - `unknown` (the probe failed, or the backend sent no block — an older build, or a groomer
//   answering for a portal misconfigured against it) is could-not-check. It must NOT block: a
//   refusal needs a definite answer, and the backend's own session-start gate enforces the real
//   rule regardless — an attempt against an unanchorable backend is refused with this same remedy,
//   with nothing allocated. The surface states that it could not check, and no more.
// - `ready` claims CONFIGURATION, never function: the backend shape-checks the addresses and makes
//   no chain read for this.

import type { HealthResp } from "../api/types";

export type DogTagAnchorReadiness =
  | { state: "ready" }
  | { state: "blocked"; headline: string; problems: readonly string[] }
  | { state: "unknown"; reason: string };

/** The refusal card's headline. A claim about the BACKEND's configuration, not about the chain. */
export const ANCHOR_BLOCKED_HEADLINE =
  "This backend cannot anchor a dog tag on-chain yet";

/**
 * The DOG_PROFILE half. Its remedy is the Provider page, because the clone is something the clinic
 * deploys itself — pointing at the ledger here would send the operator somewhere the address does
 * not exist.
 */
export const ANCHOR_MISSING_PROFILE_ISSUER =
  "The DOG_PROFILE issuer contract — the contract every dog tag is anchored through — is not set. " +
  "Deploy or select the clinic's DOG_PROFILE contract on the Provider page, configure the backend " +
  "with its address (PROFILE_ISSUER_ADDR), and restart the backend.";

/**
 * The SBT half. Its remedy is the deployment ledger, because the SBT is deployed once per
 * protocol deployment, never per clinic.
 */
export const ANCHOR_MISSING_SBT =
  "The DogTag SBT contract — where a dog tag's profile root is sealed — is not set. Configure the " +
  "backend with the DogTagSBTConsent address from the deployment ledger, " +
  "contracts/deployments/roax.json (SBT_CONSENT_ADDR), and restart the backend.";

/** The backend answered but carried no readiness block (older build, or a non-issuing role). */
export const ANCHOR_UNKNOWN_NO_BLOCK =
  "this backend did not report whether it can anchor dog tags";

/** The backend said "not ready" without naming either half — a shape this build cannot read. */
export const ANCHOR_UNKNOWN_INCOHERENT =
  "this backend reported it cannot anchor dog tags but did not say which contract is missing";

/**
 * The MINT-ROLE half, when the backend answered "missing" WITHOUT its own remedy sentence (the
 * normal missing answer carries `mintRoleDetail`, which names the signer — prefer it). The remedy
 * is a BUTTON on the admin portal, never a command.
 */
export const ANCHOR_MISSING_MINT_ROLE =
  "The vet signing key does not hold the dog-tag mint role (DogTagSBTConsent ISSUER_ROLE), so " +
  "every mint reverts before broadcasting. On the ADMIN portal's Providers page, use the " +
  "\"Dog-tag mint role\" card to grant it to this backend's signing key.";

/** The backend reported it could not check the mint role and gave no cause. */
export const ANCHOR_UNKNOWN_MINT_ROLE =
  "this backend could not check whether its signing key holds the dog-tag mint role";

/**
 * The could-not-check sentence both screens render around the `unknown` reason. It says what was
 * not established and what still stands — never a verdict in either direction.
 */
export const ANCHOR_UNKNOWN_NOTICE =
  "Could not check whether this backend can anchor dog tags";

/** Classify a health response. An ABSENT block is could-not-check, never either verdict. */
export function dogTagAnchorReadiness(health: HealthResp): DogTagAnchorReadiness {
  const block = health.dogTagIssuance;
  if (!block) return { state: "unknown", reason: ANCHOR_UNKNOWN_NO_BLOCK };
  const problems: string[] = [];
  if (!block.ready) {
    if (!block.profileIssuerConfigured) problems.push(ANCHOR_MISSING_PROFILE_ISSUER);
    if (!block.sbtConsentConfigured) problems.push(ANCHOR_MISSING_SBT);
    if (problems.length === 0) {
      // "Not ready" with both halves claiming configured is not a shape this build understands, and
      // a refusal built on it could name nothing to fix. Could-not-check, not a guess.
      return { state: "unknown", reason: ANCHOR_UNKNOWN_INCOHERENT };
    }
  }
  // The mint role — a DEFINITE "missing" blocks exactly as a missing contract does (the SBT would
  // refuse every mint from this signer), preferring the backend's own remedy sentence, which names
  // the signer. It composes with the config problems rather than replacing them.
  if (block.mintRole === "missing") {
    problems.push(block.mintRoleDetail || ANCHOR_MISSING_MINT_ROLE);
  }
  if (problems.length > 0) {
    return { state: "blocked", headline: ANCHOR_BLOCKED_HEADLINE, problems };
  }
  // Config ready and the role not definitely missing: "held" is the full green; "unknown" — or a
  // backend too old to report the field at all — is could-not-check, which WARNS and never blocks
  // (the backend refuses an unmintable start itself, allocating nothing).
  if (block.mintRole === "held") return { state: "ready" };
  return {
    state: "unknown",
    reason: block.mintRoleDetail || ANCHOR_UNKNOWN_MINT_ROLE,
  };
}

/** Classify a FAILED probe. A backend we could not reach is never accused of being unconfigured. */
export function dogTagAnchorProbeFailure(err: unknown): DogTagAnchorReadiness {
  const detail = err instanceof Error ? err.message : String(err);
  return { state: "unknown", reason: detail || "the readiness probe failed" };
}
