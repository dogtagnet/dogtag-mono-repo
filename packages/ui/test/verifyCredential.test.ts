// Hermetic coverage for the direct-to-RPC web verify path (wallet/verifyCredential.ts). No network:
// integrity is the real offline `@dogtag/standard` recompute, and the on-chain `view` reads are an
// injected fake `IssuerChainReader`. These assert the web panel classifies a credential identically
// to the operator-gated vet-api handler it replaces, using the SAME reads the mobile apps do.
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";
import { describe, expect, it } from "vitest";
import {
  DEPLOYED_ADDRESSES,
  recordTypeKey,
  UNPOSITIONED_LOG,
  type LogPoint,
  type UnpositionedLog,
  type WhitelistGrantEvent,
} from "../src/wallet/contracts";
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
/**
 * The registry the CLONE names - `DogTagIssuer.registry()`, the only authority whose `_wl` log can
 * answer for that contract's issuances. Deliberately NOT `DEPLOYED_ADDRESSES.IssuerRegistry`: the
 * pillar must ask this one, so a fake that used the configured address could not tell the two apart.
 */
const GOVERNING_REGISTRY = "0x00000000000000000000000000000000005e6157";
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

const ZERO_ADDR = "0x0000000000000000000000000000000000000000";
const VACCINATION_KEY = recordTypeKey("VACCINATION");

interface ReaderCfg {
  /** `rootIssuer(root)`; the zero address models a root no factory clone ever issued. */
  rootIssuer?: string;
  /** The RESOLVED clone's own `recordType()`; the zero word models an uninitialized contract. */
  recordType?: string;
  issuedAt?: bigint;
  isValid?: boolean;
  isRevoked?: boolean;
  /** `issuedBy(root)`; the zero address models a clone that never issued this root. */
  issuedBy?: string;
  /** The clone's own `registry()` - the authority whose grant log answers for its issuances. */
  issuerRegistry?: string;
  /**
   * Where the clone's `RootIssued` for this root sits; `null` models a contract that emitted none, and
   * `UNPOSITIONED_LOG` one that emitted a log the node gave no `(blockNumber, logIndex)`.
   */
  rootIssuedAt?: LogPoint | null | UnpositionedLog;
  /**
   * The governing registry's grant log for this pair. An EMPTY array is a real answer;
   * `UNPOSITIONED_LOG` models a grant the node returned with no position, which cannot be ordered.
   */
  grantHistory?: WhitelistGrantEvent[] | UnpositionedLog;
}

/** The honest ordering: granted, then the root anchored, and never withdrawn. */
const GRANTED: LogPoint = { blockNumber: 100n, logIndex: 0 };
const ANCHORED: LogPoint = { blockNumber: 200n, logIndex: 3 };

/**
 * Defaults describe a GENUINE credential: the factory names the clone the document claims, that clone
 * declares the document's record type, it issued the root, and its governing registry's log shows the
 * signer holding that capability at the anchoring point.
 */
function fakeReader(cfg: ReaderCfg) {
  const calls = {
    rootIssuer: [] as Array<[string, string]>,
    recordType: [] as string[],
    issuedAt: [] as Array<[string, string]>,
    isValid: [] as Array<[string, string]>,
    isRevoked: [] as Array<[string, string]>,
    issuedBy: [] as Array<[string, string]>,
    issuerRegistry: [] as string[],
    rootIssuedAt: [] as Array<[string, string]>,
    grantHistory: [] as Array<[string, string, string]>,
  };
  const reader: IssuerChainReader = {
    async rootIssuer(factory, root) {
      calls.rootIssuer.push([factory, root]);
      return cfg.rootIssuer ?? ISSUER.documentStore;
    },
    async recordType(addr) {
      calls.recordType.push(addr);
      return cfg.recordType ?? VACCINATION_KEY;
    },
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
    async issuedBy(addr, root) {
      calls.issuedBy.push([addr, root]);
      return cfg.issuedBy ?? SIGNER;
    },
    async issuerRegistry(addr) {
      calls.issuerRegistry.push(addr);
      return cfg.issuerRegistry ?? GOVERNING_REGISTRY;
    },
    async rootIssuedAt(addr, root) {
      calls.rootIssuedAt.push([addr, root]);
      return cfg.rootIssuedAt === undefined ? ANCHORED : cfg.rootIssuedAt;
    },
    async grantHistory(registry, key, signer) {
      calls.grantHistory.push([registry, key, signer]);
      return cfg.grantHistory ?? [{ kind: "whitelisted", ...GRANTED }];
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
    // The signer is RESOLVED from the chain, not supplied by the caller.
    expect(r.signerAddr).toBe(SIGNER);
    expect(r.fragments).toEqual({
      integrity: true,
      onchain: true,
      issued: true,
      revoked: false,
      issuerWhitelisted: true,
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
      // The governing registry's log records no grant to this signer for this record type, ever.
      grantHistory: [],
    });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.status).toBe("valid"); // on-chain state is valid…
    expect(r.verdict).toBe(false); // …but an unauthorised issuer signer fails the verdict.
    expect(r.signerAddr).toBe(SIGNER);
    expect(r.fragments.issuerWhitelisted).toBe(false);
    // The grant history is read from the GOVERNING registry - the one the CLONE names - with the
    // record-type key the CHAIN reported and the CHAIN-resolved signer. Never anything supplied by
    // the caller or the document, and never this client's own configured registry.
    expect(calls.grantHistory).toEqual([[GOVERNING_REGISTRY, VACCINATION_KEY, SIGNER]]);
    expect(calls.grantHistory[0][0]).not.toBe(DEPLOYED_ADDRESSES.IssuerRegistry);
  });

  it("delisting is FORWARD-ONLY: after the anchoring it verifies, before it does not", async () => {
    const doc = validDoc();
    const granted = { kind: "whitelisted", ...GRANTED } as const;

    // Delisted AFTER the anchoring - a key rotation. The credential stays genuine.
    const after = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      now: NOW,
      reader: fakeReader({
        issuedAt: 1_699_000_000n,
        isValid: true,
        grantHistory: [
          granted,
          { kind: "delisted", blockNumber: ANCHORED.blockNumber + 500n, logIndex: 0 },
        ],
      }).reader,
    });
    expect(after.fragments.issuerWhitelisted).toBe(true);
    expect(after.verdict).toBe(true);

    // Delisted BEFORE it - an anchoring `onlyWhitelisted` could not have permitted.
    const before = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      now: NOW,
      reader: fakeReader({
        issuedAt: 1_699_000_000n,
        isValid: true,
        grantHistory: [
          granted,
          { kind: "delisted", blockNumber: ANCHORED.blockNumber - 1n, logIndex: 0 },
        ],
      }).reader,
    });
    expect(before.fragments.issuerWhitelisted).toBe(false);
    expect(before.verdict).toBe(false);
  });

  it("could-not-determine is neither a pass nor an accusation", async () => {
    const doc = validDoc();
    // (a) the clone names no governing registry - there is no authority whose log could answer.
    const noAuthority = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      now: NOW,
      reader: fakeReader({ issuedAt: 1_699_000_000n, isValid: true, issuerRegistry: ZERO_ADDR })
        .reader,
    });
    expect(noAuthority.fragments.issuerWhitelisted).toBeNull();
    expect(noAuthority.verdict).toBe(false);

    // (b) the contract emitted no anchoring event - there is no moment to ask the history about.
    const noAnchoring = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      rootIssuedAt: null,
    });
    const unanchored = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      now: NOW,
      reader: noAnchoring.reader,
    });
    expect(unanchored.fragments.issuerWhitelisted).toBeNull();
    expect(unanchored.verdict).toBe(false);
    // And the grant log is not even consulted: there is nothing to sequence it against.
    expect(noAnchoring.calls.grantHistory).toEqual([]);
  });

  /**
   * The two unpositioned-log cases, deliberately SEPARATE tests rather than one.
   *
   * The grant-history arm is only reachable once the anchoring arm has returned a real point, so a
   * single combined case would still pass with either guard deleted - which is exactly the shape that
   * let the original coercion survive review.
   */
  it("an anchoring log with no position is undetermined, never placed at genesis", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      // The node returned a `RootIssued` it considers PENDING: no blockNumber, no logIndex. Coercing
      // it to `(0n, 0)` would anchor this root before every grant and refuse a genuine credential;
      // skipping it would let a later sibling move the anchoring past a delisting. Neither is a
      // reading we have, so the pillar reports it could not run.
      rootIssuedAt: UNPOSITIONED_LOG,
    });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.verdict).toBe(false);
    // Nothing to sequence against, so the grant log is not consulted at all.
    expect(calls.grantHistory).toEqual([]);
  });

  it("a grant log with no position is undetermined, and is NOT read as an empty history", async () => {
    const doc = validDoc();
    const { reader } = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      grantHistory: UNPOSITIONED_LOG,
    });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    // `null`, NOT the `false` an empty history earns above: the registry's log could not be ordered,
    // which is a fact about our reading rather than evidence about the credential. The three outcomes
    // stay apart - definite refusal, undetermined, and a read that threw.
    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.verdict).toBe(false);
    expect(r.status).toBe("valid"); // the on-chain state is untouched by an unreadable grant log
  });

  it("resolves the signer from the chain, so the pillar runs with no operator input at all", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(calls.issuedBy).toEqual([[ISSUER.documentStore, doc.signature.merkleRoot]]);
    expect(r.fragments.issuerWhitelisted).toBe(true);
    expect(r.verdict).toBe(true);
  });

  it("an UNRESOLVED whitelist pillar is never a pass (the relabelled-issuer fail-open)", async () => {
    // `issuedBy` == the zero address: the clone the document names never issued this root. Before the
    // pillar was mandatory this returned `verdict: true` — which is how a credential relabelled to a
    // different issuing authority still verified.
    const doc = validDoc();
    const { reader, calls } = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      issuedBy: ZERO_ADDR,
    });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.fragments.issuerWhitelisted).toBeNull(); // indeterminate…
    expect(r.verdict).toBe(false); // …and therefore NOT a pass.
    expect(r.signerAddr).toBeNull();
    // Never ask the registry whether the zero address is whitelisted: that answers the wrong question.
    expect(calls.grantHistory).toEqual([]);
  });

  // The forgery this pillar exists for: point `issuer.documentStore` at a contract you control that
  // answers `isValid = true` and names a genuinely whitelisted signer. Every question the verifier
  // knows how to ask is then answered by the suspect. What refuses it is refusing to take the
  // document's word for WHICH contract to ask.
  it("a documentStore swap is refused: reads go to the clone the FACTORY names", async () => {
    const doc = validDoc();
    const hostile = "0x00000000000000000000000000000000deadbeef";
    const forged = {
      ...doc,
      issuer: { ...doc.issuer, documentStore: hostile, name: "Ministry of Health" },
    };
    // The factory still names the REAL clone — the hostile contract can never enter that index.
    const { reader, calls } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(forged as WrappedDoc),
      reader,
      now: NOW,
    });

    expect(r.fragments.issuerWhitelisted).toBe(false);
    expect(r.verdict).toBe(false);
    // Every read went to the resolved clone, and the hostile contract was never consulted.
    expect(r.issuerAddr).toBe(ISSUER.documentStore);
    expect(calls.isValid[0]?.[0]).toBe(ISSUER.documentStore);
    expect(calls.grantHistory).toEqual([]);
  });

  it("a root no factory clone ever issued is indeterminate, and no reads are made", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({ rootIssuer: ZERO_ADDR, isValid: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.fragments.onchain).toBe(false);
    expect(r.status).toBe("not_issued");
    expect(r.verdict).toBe(false);
    expect(calls.isValid).toEqual([]);
    expect(calls.issuedBy).toEqual([]);
  });

  it("a record-type relabel is refused by the resolved clone's own recordType()", async () => {
    // `issuer.recordType` picks WHICH whitelist question gets asked and lives in the same
    // root-uncovered block as documentStore, so an authority holding two record types could otherwise
    // carry a credential relabelled from one to the other.
    const doc = validDoc();
    const relabelled = {
      ...doc,
      issuer: { ...doc.issuer, recordType: "TRAVEL_CLEARANCE" },
    };
    const { reader, calls } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(relabelled as WrappedDoc),
      reader,
      now: NOW,
    });

    expect(r.fragments.issuerWhitelisted).toBe(false);
    expect(r.verdict).toBe(false);
    expect(calls.grantHistory).toEqual([]);
  });

  it("a clone reporting no record type leaves the pillar indeterminate, not passed", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({
      issuedAt: 1_699_000_000n,
      isValid: true,
      recordType: `0x${"0".repeat(64)}`,
    });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.verdict).toBe(false);
    expect(calls.grantHistory).toEqual([]);
  });

  it("an explicit expected signer only tightens the pillar - a mismatch fails it", async () => {
    const doc = validDoc();
    const { reader } = fakeReader({ issuedAt: 1_699_000_000n, isValid: true });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      signerAddr: "0x00000000000000000000000000000000000000ff",
      reader,
      now: NOW,
    });

    expect(r.fragments.issuerWhitelisted).toBe(false);
    expect(r.verdict).toBe(false);
    expect(r.signerAddr).toBe(SIGNER); // reported value stays the chain's answer
  });

  it("reads the CLAIMED root + the resolved clone, and recomputedRoot matches on a valid doc", async () => {
    const doc = validDoc();
    const { reader, calls } = fakeReader({ issuedAt: 1n, isValid: true });
    const r = await verifyCredentialOnchain({ wrappedDoc: asRecord(doc), reader, now: NOW });

    const claimedRoot = doc.signature.merkleRoot;
    expect(calls.rootIssuer).toEqual([
      [DEPLOYED_ADDRESSES.DogTagIssuerFactory, claimedRoot],
    ]);
    for (const c of [calls.issuedAt[0], calls.isValid[0], calls.isRevoked[0]]) {
      expect(c).toEqual([ISSUER.documentStore, claimedRoot]);
    }
    // integrity recompute reconciles to the claimed root for an untampered doc.
    expect(r.root).toBe(claimedRoot);
    expect(r.recomputedRoot).toBe(claimedRoot);
  });

  it("an explicit issuerAddr only tightens - it never selects which contract answers", async () => {
    // Were the override allowed to pick the read target, the whole forgery would just move to this
    // field: name an obliging contract and the factory anchor is bypassed without touching
    // `documentStore`. So it asserts an expectation, and a disagreeing one fails the pillar.
    const doc = validDoc();
    const hostile = "0x000000000000000000000000000000000000beef";
    const { reader, calls } = fakeReader({ issuedAt: 1n, isValid: true });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      issuerAddr: hostile,
      reader,
      now: NOW,
    });

    expect(calls.isValid[0]?.[0]).toBe(ISSUER.documentStore); // the FACTORY's clone, not the override
    expect(r.issuerAddr).toBe(ISSUER.documentStore);
    expect(r.fragments.issuerWhitelisted).toBe(false);
    expect(r.verdict).toBe(false);

    // An override that AGREES with the factory leaves a genuine credential passing.
    const agreeing = fakeReader({ issuedAt: 1n, isValid: true });
    const ok = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      issuerAddr: ISSUER.documentStore.toUpperCase().replace("0X", "0x"),
      reader: agreeing.reader,
      now: NOW,
    });
    expect(ok.fragments.issuerWhitelisted).toBe(true);
    expect(ok.verdict).toBe(true);
  });

  it("an override cannot resurrect a root no factory clone ever issued", async () => {
    // The sharpest shape of the same bypass: with nothing in the factory index there is no clone at
    // all, so an override naming an obliging contract must not become one.
    const doc = validDoc();
    const { reader, calls } = fakeReader({ rootIssuer: ZERO_ADDR, isValid: true, issuedAt: 1n });
    const r = await verifyCredentialOnchain({
      wrappedDoc: asRecord(doc),
      issuerAddr: "0x000000000000000000000000000000000000beef",
      reader,
      now: NOW,
    });

    expect(r.fragments.issuerWhitelisted).toBeNull();
    expect(r.verdict).toBe(false);
    expect(calls.isValid).toEqual([]);
    expect(calls.issuedBy).toEqual([]);
    expect(calls.grantHistory).toEqual([]);
  });

  it("fails closed: an RPC read error rejects rather than resolving as valid", async () => {
    const doc = validDoc();
    const reader: IssuerChainReader = {
      async rootIssuer() {
        return ISSUER.documentStore;
      },
      async recordType() {
        return VACCINATION_KEY;
      },
      async issuedAt() {
        return 1n;
      },
      async isValid() {
        throw new Error("rpc 502");
      },
      async isRevoked() {
        return false;
      },
      async issuedBy() {
        return SIGNER;
      },
      async issuerRegistry() {
        return GOVERNING_REGISTRY;
      },
      async rootIssuedAt() {
        return ANCHORED;
      },
      async grantHistory() {
        return [{ kind: "whitelisted" as const, ...GRANTED }];
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
