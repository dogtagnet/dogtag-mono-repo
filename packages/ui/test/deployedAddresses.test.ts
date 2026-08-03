import { describe, expect, it } from "vitest";
import { DEPLOYED_ADDRESSES, isConfiguredAddress } from "../src/wallet/contracts";
import { verifyCredentialOnchain, type IssuerChainReader } from "../src/wallet/verifyCredential";

/**
 * `DEPLOYED_ADDRESSES` is CONFIGURATION, not a table of literals - and an unconfigured address is a
 * refusal that names itself, never an address to dial.
 *
 * The property this file exists for is the one the captain asked for: a setup guide you can follow
 * from a clean slate with no hardcoded contract address anywhere. That decays silently, because a
 * pasted address compiles, typechecks and keeps working right up until the set is redeployed - at
 * which point it names a contract that decides nothing and every surface reporting it still looks
 * configured. `make check-addresses` guards the whole tree from the outside; these cases guard the
 * one module that used to hold the table, from the inside.
 */

const CLAIMED_ROOT = `0x${"11".repeat(32)}`;

/** A reader that must never be reached. Any call is the defect: an unconfigured client dialled out. */
function forbiddenReader(): IssuerChainReader {
  const forbid = (name: string) => () => {
    throw new Error(`the reader must not be reached with no factory configured (called ${name})`);
  };
  return {
    rootIssuer: forbid("rootIssuer"),
    recordType: forbid("recordType"),
    issuedAt: forbid("issuedAt"),
    isValid: forbid("isValid"),
    isRevoked: forbid("isRevoked"),
    issuedBy: forbid("issuedBy"),
    issuerRegistry: forbid("issuerRegistry"),
    rootIssuedAt: forbid("rootIssuedAt"),
    grantHistory: forbid("grantHistory"),
  } as unknown as IssuerChainReader;
}

function docNamingItsOwnStore(): Record<string, unknown> {
  return {
    version: "dogtag/1.0",
    data: {},
    signature: { type: "DogTagMerkleProof", proof: [], merkleRoot: CLAIMED_ROOT },
    privacy: { obfuscated: [] },
    // The attacker-chosen field the factory anchor exists to overrule. It must not become the
    // fallback when no factory is configured - that is the forgery this whole pillar removes.
    issuer: { name: "x", domain: "x", documentStore: `0x${"22".repeat(20)}`, recordType: "VACCINATION" },
  };
}

describe("DEPLOYED_ADDRESSES is configuration", () => {
  it("holds no address of its own - an unconfigured build reads the empty string", () => {
    // Vitest runs with no `VITE_*` address variables set, so this asserts exactly the property the
    // gate asserts from outside: nothing in this module supplies an address by itself. A literal
    // reintroduced as a default fails HERE, in the package's own suite, rather than at the next
    // redeploy.
    for (const [key, value] of Object.entries(DEPLOYED_ADDRESSES)) {
      expect(value, `${key} must come from configuration, not from a literal`).toBe("");
    }
  });

  it("carries no key for a contract that has no source", () => {
    // Each of these named a contract this repo does not build. Repointing one is worse than deleting
    // it: a key kept "just in case" is a key somebody wires up, and there is nothing to wire it to.
    for (const gone of [
      "CloneProvenanceRouter",
      "IssuerRegistry",
      "IssuerDomainRegistry",
      "Poseidon6",
    ]) {
      expect(Object.keys(DEPLOYED_ADDRESSES)).not.toContain(gone);
    }
  });

  it("tells an unset address apart from a configured one", () => {
    expect(isConfiguredAddress("")).toBe(false);
    expect(isConfiguredAddress("   ")).toBe(false);
    expect(isConfiguredAddress(undefined)).toBe(false);
    expect(isConfiguredAddress(null)).toBe(false);
    expect(isConfiguredAddress(`0x${"ab".repeat(20)}`)).toBe(true);
  });
});

describe("an unconfigured factory is a named refusal, never a pass and never a transport error", () => {
  it("refuses by name, and makes no chain read at all", async () => {
    await expect(
      verifyCredentialOnchain({
        wrappedDoc: docNamingItsOwnStore(),
        reader: forbiddenReader(),
        now: 1_700_000_000,
      }),
    ).rejects.toThrow(/VITE_DOGTAG_ISSUER_FACTORY_ADDR/);
  });

  it("does NOT fall back to the document's own documentStore", async () => {
    // The failure mode worth naming: "no factory configured" resolved to `issuer.documentStore`
    // would hand the attacker the anchor, which is precisely the forgery the factory read exists to
    // refuse. `forbiddenReader` proves it by throwing on any read; the assertion proves the refusal
    // came from the configuration check rather than from that throw.
    const err = await verifyCredentialOnchain({
      wrappedDoc: docNamingItsOwnStore(),
      reader: forbiddenReader(),
      now: 1_700_000_000,
    }).catch((e: unknown) => (e instanceof Error ? e.message : String(e)));
    expect(err).toContain("no DogTagIssuerFactory configured");
    expect(err).not.toContain("must not be reached");
  });

  it("an explicitly supplied factory still works - the refusal is about absence, not about the arg", async () => {
    const reads: string[] = [];
    const reader = {
      rootIssuer: async (factoryAddr: string) => {
        reads.push(factoryAddr);
        return "0x0000000000000000000000000000000000000000";
      },
    } as unknown as IssuerChainReader;
    const r = await verifyCredentialOnchain({
      wrappedDoc: docNamingItsOwnStore(),
      factoryAddr: `0x${"fa".repeat(20)}`,
      reader,
      now: 1_700_000_000,
    });
    expect(reads).toEqual([`0x${"fa".repeat(20)}`]);
    // No clone resolved, so the pillar is indeterminate - and an unanswered check is never a pass.
    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.verdict).toBe(false);
  });
});
