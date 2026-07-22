/**
 * Governance identities + the on-chain two-step / timelock model, grounded in the reconciled
 * deployment record (`contracts/deployments/roax.json` → `_governance`) and the deployed contracts.
 *
 * Phase-2 moved the governance/admin authority off the original deployer EOA to the governance signer
 * "signer-1" (`0x8E27…`). The console reads the LIVE authority map from the backend
 * (`GET /v1/admin/governance/authority`) and reconciles it against these known identities so the
 * page can say, at a glance, whether each authority currently resolves to signer-1.
 */

/**
 * The governance authority post-Phase-2. Per `roax.json._governance` it now holds the factory
 * `Ownable2Step` ownership, the IssuerRegistry `WHITELIST_ADMIN`, and the DEFAULT_ADMIN of the
 * IssuerRegistry + VerificationRegistry + DogTagSBT (each behind its ACDAR two-step timelock).
 */
export const GOVERNANCE_SIGNER_1 = "0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2";
export const GOVERNANCE_SIGNER_1_LABEL = "signer-1";
/**
 * The original deployer EOA. Phase-2 stripped it of the three governance authorities (factory owner,
 * WHITELIST_ADMIN, DEFAULT_ADMIN), so it must never be assumed to hold governance authority again.
 * It is NOT role-free: a 2026-07-16 audit found it still holds the retired owner-revealing SBT's
 * `ISSUER_ROLE` and known record-type whitelists, so it must not be reused as a neutral custodian.
 */
export const FORMER_DEPLOYER_EOA = "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96";

/** Case-insensitive address equality (both must be present + non-empty). */
export function sameAddr(a?: string | null, b?: string | null): boolean {
  return !!a && !!b && a.toLowerCase() === b.toLowerCase();
}
export function isGovernanceSigner1(addr?: string | null): boolean {
  return sameAddr(addr, GOVERNANCE_SIGNER_1);
}
export function isFormerDeployer(addr?: string | null): boolean {
  return sameAddr(addr, FORMER_DEPLOYER_EOA);
}

/** The two-step admin-transfer / timelock model for each governed contract, exactly as deployed. */
export interface TimelockModel {
  contract: string;
  /** The on-chain mechanism gating a change of authority. */
  mechanism: "AccessControlDefaultAdminRules" | "Ownable2Step";
  /** Human summary of the enforced delay (empty timelock for `Ownable2Step`). */
  delay: string;
  /** The two-step call sequence. */
  steps: string;
  /** What holding this authority lets you do. */
  capability: string;
}

/**
 * Grounded in the contracts:
 * - `IssuerRegistry.sol:26` — `AccessControlDefaultAdminRules(3 days, …)`.
 * - `VerificationRegistry.sol:114` — `AccessControlDefaultAdminRules(2 days, …)`, plus a separate
 *   2-day `ZK_TIMELOCK` (`:59`) for `proposeZkVerifier`/`proposeConsentKeys` → `execute…`.
 * - `DogTagSBT.sol:34,68` — `ADMIN_TRANSFER_DELAY = 3 days`.
 * - `DogTagIssuerFactory.sol:14` — `Ownable2Step` (two-step accept, no timelock).
 */
export const TIMELOCK_MODELS: TimelockModel[] = [
  {
    contract: "IssuerRegistry",
    mechanism: "AccessControlDefaultAdminRules",
    delay: "3-day timelock",
    steps: "beginDefaultAdminTransfer → wait 3 days → acceptDefaultAdminTransfer",
    capability: "adminRevoke, role admin, WHITELIST_ADMIN grant/revoke",
  },
  {
    contract: "VerificationRegistry",
    mechanism: "AccessControlDefaultAdminRules",
    delay: "2-day timelock",
    steps: "beginDefaultAdminTransfer → wait 2 days → acceptDefaultAdminTransfer",
    capability: "verifier + consent-key swaps (own 2-day ZK_TIMELOCK), relayer restriction",
  },
  {
    contract: "DogTagSBT",
    mechanism: "AccessControlDefaultAdminRules",
    delay: "3-day timelock",
    steps: "beginDefaultAdminTransfer → wait 3 days → acceptDefaultAdminTransfer",
    capability: "ISSUER_ROLE grant/revoke (mint rights)",
  },
  {
    contract: "DogTagIssuerFactory",
    mechanism: "Ownable2Step",
    delay: "no timelock (two-step accept only)",
    steps: "transferOwnership → acceptOwnership",
    capability: "createIssuer (deploy clones)",
  },
];

/** A live countdown to a unix-seconds ETA, computed against `nowMs` (Date.now()). */
export interface Countdown {
  /** the timelock has elapsed → the accept step is now callable. */
  elapsed: boolean;
  /** compact remaining label ("2d 4h 11m"), or "window open" once elapsed. */
  label: string;
}

/** Compact "2d 4h 11m" remaining until `unixSeconds`, computed against `nowMs`. */
export function countdown(unixSeconds: number, nowMs: number): Countdown {
  const remainingS = unixSeconds - Math.floor(nowMs / 1000);
  if (remainingS <= 0) return { elapsed: true, label: "window open" };
  const d = Math.floor(remainingS / 86400);
  const h = Math.floor((remainingS % 86400) / 3600);
  const m = Math.floor((remainingS % 3600) / 60);
  const parts = [d ? `${d}d` : "", h ? `${h}h` : "", `${m}m`].filter(Boolean);
  return { elapsed: false, label: parts.join(" ") };
}
