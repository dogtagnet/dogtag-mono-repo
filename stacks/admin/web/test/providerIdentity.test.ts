/**
 * The registrar's provider id and identity statement.
 *
 * These are the decisions the screen must be able to show an admin BEFORE a send that cannot be
 * undone: `providerId` is permanent and unreassignable, and the identity anchor's revision only ever
 * moves forward.
 */

import { describe, expect, it } from "vitest";
import {
  canonicalIdentityStatement,
  checkRegistration,
  EMPTY_IDENTITY_STATEMENT,
  generateProviderId,
  identityDigest,
  identityStatementProblem,
  isValidProviderId,
  registrationKey,
  type IdentityStatement,
} from "../src/lib/providerIdentity";

const STATEMENT: IdentityStatement = {
  legalName: "Seaport Veterinary Clinic Pte Ltd",
  jurisdiction: "Singapore",
  registrationNumber: "202312345K",
  verifiedOn: "2026-08-02",
  notes: "ACRA bizfile + practising licence checked",
};

describe("provider id", () => {
  it("is generated from the CSPRNG and nothing else, at exactly bytes20", () => {
    const id = generateProviderId();
    expect(id).toMatch(/^0x[0-9a-f]{40}$/);
    expect(isValidProviderId(id)).toBe(true);
  });

  /**
   * `ProviderRegistry.sol`'s own header forbids deriving the id from a name, domain, controller,
   * signer, clone, salt or generation. Two ids generated back to back must therefore share nothing.
   */
  it("does not repeat, so it cannot be carrying meaning", () => {
    const ids = new Set(Array.from({ length: 64 }, () => generateProviderId()));
    expect(ids.size).toBe(64);
  });

  /** Zero is the one id the contract refuses (`ZeroProviderId()`), so the generator retries past it. */
  it("never yields the zero id even when the source hands it zeros", () => {
    let call = 0;
    const source = (n: number) => {
      call += 1;
      // First two draws are all-zero; the third is usable.
      return call <= 2 ? new Uint8Array(n) : new Uint8Array(n).fill(7);
    };
    const id = generateProviderId(source);
    expect(id).toBe(`0x${"07".repeat(20)}`);
    expect(call).toBe(3);
  });

  it("gives up rather than returning a zero id if the source never yields one", () => {
    expect(() => generateProviderId((n) => new Uint8Array(n))).toThrow(/non-zero/);
  });

  it("rejects a malformed or zero id", () => {
    expect(isValidProviderId("0x" + "0".repeat(40))).toBe(false);
    expect(isValidProviderId("0xdeadbeef")).toBe(false);
    expect(isValidProviderId("deadbeef".repeat(5))).toBe(false);
    expect(isValidProviderId("0x" + "g".repeat(40))).toBe(false);
    expect(isValidProviderId("0x" + "a".repeat(40))).toBe(true);
  });
});

describe("identity statement", () => {
  /**
   * The digest commits to the ASSERTION, not to a formatting accident. Two registrars entering the
   * same facts with different spacing must land on the same anchor.
   */
  it("canonicalises whitespace and unicode form, so the digest is about the facts", () => {
    const spaced: IdentityStatement = {
      ...STATEMENT,
      legalName: "  Seaport   Veterinary  Clinic Pte Ltd\t",
    };
    expect(canonicalIdentityStatement(spaced)).toBe(canonicalIdentityStatement(STATEMENT));
    expect(identityDigest(spaced)).toBe(identityDigest(STATEMENT));
  });

  it("changes the digest when any asserted fact changes", () => {
    const base = identityDigest(STATEMENT);
    expect(identityDigest({ ...STATEMENT, legalName: "Other Clinic" })).not.toBe(base);
    expect(identityDigest({ ...STATEMENT, jurisdiction: "Malaysia" })).not.toBe(base);
    expect(identityDigest({ ...STATEMENT, registrationNumber: "999" })).not.toBe(base);
    expect(identityDigest({ ...STATEMENT, verifiedOn: "2026-08-03" })).not.toBe(base);
    expect(identityDigest({ ...STATEMENT, notes: "nothing checked" })).not.toBe(base);
  });

  it("is a well-formed non-zero bytes32 — the contract refuses a zero digest", () => {
    const d = identityDigest(STATEMENT);
    expect(d).toMatch(/^0x[0-9a-f]{64}$/);
    expect(d).not.toBe(`0x${"0".repeat(64)}`);
  });

  /**
   * An anchor over an empty statement asserts nothing while looking exactly like one that asserts
   * something — and it would still be a perfectly valid non-zero digest, so the contract cannot
   * catch it.
   */
  it("refuses a statement with nothing asserted", () => {
    expect(identityStatementProblem(EMPTY_IDENTITY_STATEMENT)).toMatch(/legal entity name/i);
    expect(identityStatementProblem({ ...STATEMENT, jurisdiction: "" })).toMatch(/jurisdiction/i);
    expect(identityStatementProblem({ ...STATEMENT, verifiedOn: "yesterday" })).toMatch(/YYYY-MM-DD/);
    // The optional fields really are optional — not every jurisdiction issues a number.
    expect(identityStatementProblem({ ...STATEMENT, registrationNumber: "", notes: "" })).toBeNull();
  });
});

describe("the checked registration", () => {
  it("carries the exact values it checked, so a send addresses those", () => {
    const r = checkRegistration("0x" + "a".repeat(40), "0x" + "b".repeat(40), STATEMENT);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.checked.providerId).toBe("0x" + "a".repeat(40));
    expect(r.checked.controller).toBe("0x" + "b".repeat(40));
    expect(r.checked.digest).toBe(identityDigest(STATEMENT));
    expect(r.checked.canonical).toBe(canonicalIdentityStatement(STATEMENT));
  });

  /**
   * The staleness key is what retires a plan when an input moves. Every input the plan depends on
   * must be in it, or a send could carry a value nobody reviewed — permanently, since a providerId
   * cannot be reassigned.
   */
  it("changes its key when ANY input it depends on changes", () => {
    const pid = "0x" + "a".repeat(40);
    const ctrl = "0x" + "b".repeat(40);
    const base = registrationKey(pid, ctrl, STATEMENT);
    expect(registrationKey("0x" + "c".repeat(40), ctrl, STATEMENT)).not.toBe(base);
    expect(registrationKey(pid, "0x" + "d".repeat(40), STATEMENT)).not.toBe(base);
    for (const field of [
      "legalName",
      "jurisdiction",
      "registrationNumber",
      "verifiedOn",
      "notes",
    ] as const) {
      expect(
        registrationKey(pid, ctrl, { ...STATEMENT, [field]: `${STATEMENT[field]} changed` }),
      ).not.toBe(base);
    }
  });

  it("is stable under case and surrounding whitespace, which change nothing asserted", () => {
    const pid = "0x" + "A".repeat(40);
    const base = registrationKey("0x" + "a".repeat(40), "0x" + "b".repeat(40), STATEMENT);
    expect(registrationKey(pid, " 0x" + "B".repeat(40) + " ", STATEMENT)).toBe(base);
  });

  it("refuses the inputs the contract would refuse, before any transaction", () => {
    const good = "0x" + "b".repeat(40);
    expect(checkRegistration("0x" + "0".repeat(40), good, STATEMENT)).toMatchObject({ ok: false });
    expect(checkRegistration("0xdead", good, STATEMENT)).toMatchObject({ ok: false });
    expect(checkRegistration("0x" + "a".repeat(40), "0x" + "0".repeat(40), STATEMENT)).toMatchObject({
      ok: false,
    });
    expect(checkRegistration("0x" + "a".repeat(40), "0xdead", STATEMENT)).toMatchObject({ ok: false });
    expect(
      checkRegistration("0x" + "a".repeat(40), good, EMPTY_IDENTITY_STATEMENT),
    ).toMatchObject({ ok: false });
  });
});
