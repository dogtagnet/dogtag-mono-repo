// Hermetic coverage for the direct-to-RPC web verify path (wallet/verifyCredential.ts). No network:
// integrity is the real offline `@dogtag/standard` recompute, and the on-chain `view` reads are an
// injected fake `IssuerChainReader`. These assert the web panel classifies a credential identically
// to the operator-gated vet-api handler it replaces, using the SAME reads the mobile apps do.
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";
import { describe, expect, it } from "vitest";
import { DEPLOYED_ADDRESSES } from "../src/wallet/contracts";
import {
  verifyCredentialOnchain,
  type IssuerChainReader,
} from "../src/wallet/verifyCredential";

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: "0x0000000000000000000000000000000000000001",
  recordType: "VACCINATION",
};
const SIGNER = "0xabc0000000000000000000000000000000000abc";
const NOW = 1_700_000_000;

/** A wrapped doc with dogTagId == "42"; deterministic salts so the root is stable across runs. */
function validDoc(): WrappedDoc {
  let seq = 0;
  const fixedSalt = () => new Uint8Array(16).fill(++seq);
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: { tag: TypeTag.Integer, value: "42" },
        name: { tag: TypeTag.String, value: "Rex" },
      },
    },
    ISSUER,
    fixedSalt,
  );
}

/** Tamper a packed value while preserving salt/tag so integrity recomputes to INVALID. */
function tamperIntegrity(doc: WrappedDoc): WrappedDoc {
  const data = JSON.parse(JSON.stringify(doc.data));
  const packed: string = data.credentialSubject.name;
  const [salt, tag] = packed.split(":");
  data.credentialSubject.name = `${salt}:${tag}:Fido`;
  return { ...doc, data };
}

interface ReaderCfg {
  issuedAt?: bigint;
  isValid?: boolean;
  isRevoked?: boolean;
  isWhitelistedFor?: boolean;
}

function fakeReader(cfg: ReaderCfg) {
  const calls = {
    issuedAt: [] as Array<[string, string]>,
    isValid: [] as Array<[string, string]>,
    isRevoked: [] as Array<[string, string]>,
    isWhitelistedFor: [] as Array<[string, string, string]>,
  };
  const reader: IssuerChainReader = {
    async issuedAt(addr, root) {
      calls.issuedAt.push([addr, root]);
      return cfg.issuedAt ?? 0n;
    },
    async isValid(addr, root) {
      calls.isValid.push([addr, root]);
      return cfg.isValid ?? false;
    },
    async isRevoked(addr, root) {
      calls.isRevoked.push([addr, root]);
      return cfg.isRevoked ?? false;
    },
    async isWhitelistedFor(registry, recordType, signer) {
      calls.isWhitelistedFor.push([registry, recordType, signer]);
      return cfg.isWhitelistedFor ?? false;
    },
  };
  return { reader, calls };
}

const asRecord = (d: WrappedDoc) => d as unknown as Record<string, unknown>;

describe("verifyCredentialOnchain - classification parity with the vet-api handler", () => {
  it("valid: issued, not revoked, isValid → verdict pass / status valid", async () => {
    const doc = validDoc();
    const { reader } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true, isRevoked: false });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.verdict).toBe(true);
    expect(r.status).toBe("valid");
    expect(r.recordType).toBe("VACCINATION");
    expect(r.issuedAt).toBe("1699000000");
    expect(r.checkedAt).toBe(NOW);
    expect(r.signerAddr).toBeNull();
    expect(r.fragments).toEqual({
      integrity: true,
      onchain: true,
      issued: true,
      revoked: false,
      issuerWhitelisted: null,
    });
  });

  it("revoked: issued but revokedAt set → verdict fail / status revoked", async () => {
    const doc = validDoc();
    const { reader } = fakeReader({ issuedAt: 1_699_000_000n, isValid: false, isRevoked: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.status).toBe("revoked");
    expect(r.verdict).toBe(false);
    expect(r.fragments.revoked).toBe(true);
    expect(r.fragments.issued).toBe(true);
  });

  it("not_issued: issuedAt == 0 → status not_issued (distinct from revoked)", async () => {
    const doc = validDoc();
    const { reader } = fakeReader({ issuedAt: 0n, isValid: false, isRevoked: false });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.status).toBe("not_issued");
    expect(r.verdict).toBe(false);
    expect(r.issuedAt).toBe("0");
    expect(r.fragments.issued).toBe(false);
  });

  it("integrity_failed: tampered doc → status integrity_failed even when chain says valid", async () => {
    const doc = tamperIntegrity(validDoc());
    const { reader } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true, isRevoked: false });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.status).toBe("integrity_failed");
    expect(r.verdict).toBe(false);
    expect(r.fragments.integrity).toBe(false);
  });

  it("whitelist pillar gates the verdict but not the status", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      isRevoked: false,
      isWhitelistedFor: false,
    });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      signerAddr: SIGNER,
      reader,
      now: NOW,
    });

    expect(r.status).toBe("valid"); // on-chain state is valid…
    expect(r.verdict).toBe(false); // …but a non-whitelisted issuer signer fails the verdict.
    expect(r.signerAddr).toBe(SIGNER);
    expect(r.fragments.issuerWhitelisted).toBe(false);
    // whitelist read uses the deployed IssuerRegistry, the human record type, and the signer.
    expect(calls.isWhitelistedFor).toEqual([
      [DEPLOYED_ADDRESSES.IssuerRegistry, "VACCINATION", SIGNER],
    ]);
  });

  it("skips the whitelist read (issuerWhitelisted = null) when no signer is given", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(calls.isWhitelistedFor).toEqual([]);
  });

  it("reads the CLAIMED root + issuer documentStore, and recomputedRoot matches on a valid doc", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({ issuedAt: 1n, isValid: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    const claimedRoot = doc.signature.merkleRoot;
    for (const c of [calls.issuedAt[0], calls.isValid[0], calls.isRevoked[0]]) {
      expect(c).toEqual([ISSUER.documentStore, claimedRoot]);
    }
    // integrity recompute reconciles to the claimed root for an untampered doc.
    expect(r.root).toBe(claimedRoot);
    expect(r.recomputedRoot).toBe(claimedRoot);
  });

  it("honors an explicit issuerAddr override for the chain reads", async () => {
    const doc = validDoc();
    const override = "0x000000000000000000000000000000000000beef";
    const { reader, calls } = fakeReader({ issuedAt: 1n, isValid: true });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      issuerAddr: override,
      reader,
      now: NOW,
    });

    expect(r.issuerAddr).toBe(override);
    expect(calls.isValid[0]?.[0]).toBe(override);
  });

  it("fails closed: an RPC read error rejects rather than resolving as valid", async () => {
    const doc = validDoc();
    const reader: IssuerChainReader = {
      async issuedAt() {
        return 1n;
      },
      async isValid() {
        throw new Error("rpc 502");
      },
      async isRevoked() {
        return false;
      },
      async isWhitelistedFor() {
        return true;
      },
    };
    await expect(
      verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW }),
    ).rejects.toThrow("rpc 502");
  });

  it("rejects a structurally malformed document (missing issuer/signature)", async () => {
    const { reader } = fakeReader({ isValid: true });
    await expect(
      verifyCredentialOnchain({ wrappedDoc: { foo: "bar" }, reader, now: NOW }),
    ).rejects.toThrow(/issuer\/signature/);
  });
});
