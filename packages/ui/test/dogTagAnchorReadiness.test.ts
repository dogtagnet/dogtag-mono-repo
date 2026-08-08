// The Register pet anchor gate's ONE decision. What these cases protect, in order of importance:
// only a DEFINITE backend answer may block the screen (could-not-check is never a refusal — and
// never a pass); the two missing-contract sentences point at DIFFERENT remedies, because sending
// the operator to the wrong screen is the defect the gate exists to remove; and the env variable
// is a parenthesised detail inside an instruction, never the instruction itself.
import { describe, expect, it } from "vitest";
import {
  ANCHOR_BLOCKED_HEADLINE,
  ANCHOR_MISSING_MINT_ROLE,
  ANCHOR_MISSING_PROFILE_ISSUER,
  ANCHOR_MISSING_SBT,
  ANCHOR_UNKNOWN_INCOHERENT,
  ANCHOR_UNKNOWN_MINT_ROLE,
  ANCHOR_UNKNOWN_NO_BLOCK,
  dogTagAnchorProbeFailure,
  dogTagAnchorReadiness,
} from "../src/issuance/anchorReadiness";
import type { HealthResp } from "../src/api/types";

function health(block?: HealthResp["dogTagIssuance"]): HealthResp {
  return block ? { status: "ok", dogTagIssuance: block } : { status: "ok" };
}

describe("dogTagAnchorReadiness", () => {
  it("is ready only when both anchor contracts are configured AND the mint role is held", () => {
    const r = dogTagAnchorReadiness(
      health({
        ready: true,
        profileIssuerConfigured: true,
        sbtConsentConfigured: true,
        mintRole: "held",
      }),
    );
    expect(r).toEqual({ state: "ready" });
  });

  it("blocks on a missing DOG_PROFILE issuer, pointing at the Provider page", () => {
    const r = dogTagAnchorReadiness(
      health({ ready: false, profileIssuerConfigured: false, sbtConsentConfigured: true }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.headline).toBe(ANCHOR_BLOCKED_HEADLINE);
    expect(r.problems).toEqual([ANCHOR_MISSING_PROFILE_ISSUER]);
    expect(r.problems[0]).toContain("DOG_PROFILE issuer contract");
    expect(r.problems[0]).toContain("Provider page");
    // A configured SBT must not be accused alongside it.
    expect(r.problems.join(" ")).not.toContain("DogTagSBTConsent");
  });

  it("blocks on a missing SBT, pointing at the deployment ledger — a different remedy", () => {
    const r = dogTagAnchorReadiness(
      health({ ready: false, profileIssuerConfigured: true, sbtConsentConfigured: false }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.problems).toEqual([ANCHOR_MISSING_SBT]);
    expect(r.problems[0]).toContain("DogTagSBTConsent");
    expect(r.problems[0]).toContain("deployment ledger");
    expect(r.problems[0]).not.toContain("Provider page");
  });

  it("names BOTH problems when both halves are missing", () => {
    const r = dogTagAnchorReadiness(
      health({ ready: false, profileIssuerConfigured: false, sbtConsentConfigured: false }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.problems).toEqual([ANCHOR_MISSING_PROFILE_ISSUER, ANCHOR_MISSING_SBT]);
  });

  it("the env variable rides as a parenthesised detail inside an instruction, never alone", () => {
    // Each sentence must instruct (deploy/configure/restart) and only MENTION the variable —
    // "PROFILE_ISSUER_ADDR is unset" alone is a config variable, not an instruction.
    for (const [sentence, envVar] of [
      [ANCHOR_MISSING_PROFILE_ISSUER, "PROFILE_ISSUER_ADDR"],
      [ANCHOR_MISSING_SBT, "SBT_CONSENT_ADDR"],
    ] as const) {
      expect(sentence).toContain(`(${envVar})`);
      expect(sentence).toContain("restart the backend");
      expect(sentence.replace(envVar, "").length).toBeGreaterThan(80);
    }
  });

  it("an ABSENT block is could-not-check — never blocked, never ready", () => {
    const r = dogTagAnchorReadiness(health(undefined));
    expect(r).toEqual({ state: "unknown", reason: ANCHOR_UNKNOWN_NO_BLOCK });
  });

  it("a not-ready answer naming nothing missing is could-not-check, not a guess", () => {
    const r = dogTagAnchorReadiness(
      health({ ready: false, profileIssuerConfigured: true, sbtConsentConfigured: true }),
    );
    expect(r).toEqual({ state: "unknown", reason: ANCHOR_UNKNOWN_INCOHERENT });
  });

  it("a DEFINITELY missing mint role blocks, preferring the backend's own remedy sentence", () => {
    const r = dogTagAnchorReadiness(
      health({
        ready: true,
        profileIssuerConfigured: true,
        sbtConsentConfigured: true,
        mintRole: "missing",
        mintRoleDetail: "the vet signing key 0xabc does not hold the dog-tag mint role — grant it",
      }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.problems).toEqual([
      "the vet signing key 0xabc does not hold the dog-tag mint role — grant it",
    ]);
  });

  it("a missing mint role with no backend detail still names the admin-portal card — a button, never a command", () => {
    const r = dogTagAnchorReadiness(
      health({
        ready: true,
        profileIssuerConfigured: true,
        sbtConsentConfigured: true,
        mintRole: "missing",
      }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.problems).toEqual([ANCHOR_MISSING_MINT_ROLE]);
    expect(r.problems[0]).toContain("Dog-tag mint role");
    expect(r.problems[0]).not.toContain("cast ");
  });

  it("the mint-role problem COMPOSES with a config problem rather than replacing it", () => {
    const r = dogTagAnchorReadiness(
      health({
        ready: false,
        profileIssuerConfigured: false,
        sbtConsentConfigured: true,
        mintRole: "missing",
      }),
    );
    if (r.state !== "blocked") throw new Error(`expected blocked, got ${r.state}`);
    expect(r.problems).toEqual([ANCHOR_MISSING_PROFILE_ISSUER, ANCHOR_MISSING_MINT_ROLE]);
  });

  it("an UNKNOWN mint role warns and never blocks — could-not-check is not a refusal", () => {
    const r = dogTagAnchorReadiness(
      health({
        ready: true,
        profileIssuerConfigured: true,
        sbtConsentConfigured: true,
        mintRole: "unknown",
        mintRoleDetail: "custody is locked, so the signing key's address cannot be resolved",
      }),
    );
    expect(r).toEqual({
      state: "unknown",
      reason: "custody is locked, so the signing key's address cannot be resolved",
    });
  });

  it("a backend too old to report the mint role at all is could-not-check, never silently green", () => {
    const r = dogTagAnchorReadiness(
      health({ ready: true, profileIssuerConfigured: true, sbtConsentConfigured: true }),
    );
    expect(r).toEqual({ state: "unknown", reason: ANCHOR_UNKNOWN_MINT_ROLE });
  });

  it("a FAILED probe is could-not-check carrying its own cause — an unreachable backend is not an unconfigured one", () => {
    const r = dogTagAnchorProbeFailure(new Error("fetch failed"));
    expect(r.state).toBe("unknown");
    if (r.state !== "unknown") throw new Error("unreachable");
    expect(r.reason).toContain("fetch failed");
  });

  it("a probe failure with no message still yields a stated reason, never an empty one", () => {
    const r = dogTagAnchorProbeFailure(new Error(""));
    if (r.state !== "unknown") throw new Error(`expected unknown, got ${r.state}`);
    expect(r.reason.length).toBeGreaterThan(0);
  });
});
