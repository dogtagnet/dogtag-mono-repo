// Hermetic coverage for the admin verification bench (wallet/verificationBench.ts + benchMutations.ts).
//
// No network. Integrity is the real offline `@dogtag/standard` recompute, and the chain is an
// ADDRESS-KEYED fake. That keying is not incidental: a fake that ignores which contract it is asked
// about cannot represent "the hostile contract answers true while the clone the factory named answers
// false", so every forged-issuer test written against one passes for the wrong reason. This repo has
// been bitten by exactly that three times (see the `MockChain` note in
// `crates/dogtag-standard-rs/src/verify.rs`), so every read below takes its contract argument
// seriously and each test breaks exactly ONE thing off a genuine baseline.
import { TypeTag, wrapDocument, type IssuerMeta, type WrappedDoc } from "@dogtag/standard";
import { describe, expect, it } from "vitest";
import { DEPLOYED_ADDRESSES, recordTypeKey } from "../src/wallet/contracts";
import type { IssuerChainReader } from "../src/wallet/verifyCredential";
import {
  docFromShareResponse,
  runVerificationBench,
  validUntilOf,
  VALID_UNTIL_KEYPATHS,
  type BenchCheck,
  type BenchCheckId,
  type BenchReport,
  type DomainClaimReader,
  type GrantHistoryReader,
  type IssuerAuthorityReader,
} from "../src/wallet/verificationBench";
import type { LogPoint, WhitelistGrantEvent } from "../src/wallet/contracts";
import {
  BENCH_MUTATIONS,
  expireValidityWindow,
  extendValidityWindow,
  relabelIssuerName,
  repointDocumentStore,
  tamperCoveredField,
} from "../src/wallet/benchMutations";

/** The clone the FACTORY names for the root, and the address an honest document also names. */
const CLONE = "0x0000000000000000000000000000000000000001";
/** A contract the factory never deployed, deployed by whoever forged the document. */
const HOSTILE = "0x000000000000000000000000000000000000ba0b";
/** The signer that actually issued R on-chain (== clone.issuedBy[R]). */
const SIGNER = "0xabc0000000000000000000000000000000000abc";
const ZERO_ADDR = "0x0000000000000000000000000000000000000000";
const DOMAIN_REGISTRY = "0x00000000000000000000000000000000000d0a17";
const VACCINATION_KEY = recordTypeKey("VACCINATION");
const NOW = 1_700_000_000;
const TODAY = "2023-11-14";
/** Where the genuine baseline anchors its root, and where the signer's grant was recorded. */
const ISSUED_AT_LOG = { blockNumber: 900_000n, logIndex: 4 };
const GRANTED_AT_LOG = { blockNumber: 800_000n, logIndex: 1 };

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: CLONE,
  recordType: "VACCINATION",
};

/** Deterministic salts, so a fixture's root is stable across runs. */
function fixedSalts() {
  let seq = 0;
  return () => new Uint8Array(16).fill(++seq);
}

/**
 * TIER 1 of the canonical validUntil chain: TRAVEL_CLEARANCE's nested CDC `validity` block.
 *
 * The fixture deliberately carries a REAL issuer's shape. It used to write a bare
 * `credentialSubject.validUntil`, which no issuer emits - so the suite agreed with the bench about a
 * keyPath that exists nowhere else, and could not have noticed the chain being wrong.
 */
function validDoc(validUntil = "2027-01-11"): WrappedDoc {
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: { tag: TypeTag.Integer, value: "42" },
        name: { tag: TypeTag.String, value: "Rex" },
        validity: { validUntil: { tag: TypeTag.String, value: validUntil } },
      },
    },
    ISSUER,
    fixedSalts(),
  );
}

/** TIER 2: EU_HEALTH_CERT's flat Annex-IV leaf. This record type has NO `validity` block at all. */
function euHealthCertDoc(rabiesValidUntil = "2029-01-14"): WrappedDoc {
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: { tag: TypeTag.Integer, value: "42" },
        species: { tag: TypeTag.String, value: "dog" },
        rabiesValidUntil: { tag: TypeTag.String, value: rabiesValidUntil },
      },
    },
    ISSUER,
    fixedSalts(),
  );
}

/**
 * TIER 3: VACCINATION's schema field is DOTLESS, so the leaf lands as a SIBLING of
 * `credentialSubject` at the top level of `data` - never as a child of it.
 */
function vaccinationDoc(validUntil = "2027-06-30"): WrappedDoc {
  return wrapDocument(
    {
      credentialSubject: { dogTagId: { tag: TypeTag.Integer, value: "42" } },
      validUntil: { tag: TypeTag.String, value: validUntil },
    },
    ISSUER,
    fixedSalts(),
  );
}

/** Overwrite a packed leaf's VALUE, keeping its salt and type tag - a blank window, not a broken leaf. */
function setValidUntilValue(doc: WrappedDoc, keyPath: string, value: string): Record<string, unknown> {
  const d = JSON.parse(JSON.stringify(doc));
  const parts = keyPath.split(".");
  let node = d.data;
  for (const p of parts.slice(0, -1)) node = node[p];
  const leaf = parts[parts.length - 1] as string;
  const packed = node[leaf] as string;
  const [saltHex, tag] = packed.split(":");
  node[leaf] = `${saltHex}:${tag}:${value}`;
  return d;
}

const asRecord = (d: WrappedDoc | Record<string, unknown>) => d as unknown as Record<string, unknown>;

// ── the address-keyed fake chain ────────────────────────────────────────────────────────────────

/** Every read the bench can make, including the two `eth_getLogs` it performs on its own account. */
type FailableRead =
  | keyof IssuerChainReader
  | "issuerRegistry"
  | "rootIssuedLog"
  | "whitelistHistory";

interface ChainCfg {
  /** factory rootIssuer index: root -> clone. Absent == the factory has NO record of that root. */
  rootIssuers?: Record<string, string>;
  /** (contract|root) -> isValid. Absent == false. */
  isValid?: Record<string, boolean>;
  /** (contract|root) -> issuedAt. Absent == 0n (never issued HERE). */
  issuedAt?: Record<string, bigint>;
  /** (contract|root) -> isRevoked. */
  isRevoked?: Record<string, boolean>;
  /** (contract|root) -> issuedBy. Absent == the zero address. */
  issuedBy?: Record<string, string>;
  /** contract -> its immutable recordType(). */
  recordTypes?: Record<string, string>;
  /** whitelisted "key|signer" pairs - the CURRENT state, as `isWhitelistedFor` answers it. */
  whitelist?: Set<string>;
  /** contract -> the registry its own `onlyWhitelisted` consults. Absent == the zero address. */
  issuerRegistries?: Record<string, string>;
  /** (contract|root) -> where `RootIssued` was emitted. Absent == no anchoring log. */
  rootIssuedLogs?: Record<string, LogPoint>;
  /** ("key|signer") -> the registry's grant history, oldest first. Absent == none ever recorded. */
  grants?: Record<string, WhitelistGrantEvent[]>;
  /** Read kinds forced to fail, modelling a transient RPC error. */
  failing?: Set<FailableRead>;
}

const k = (a: string, b: string) => `${a.toLowerCase()}|${b.toLowerCase()}`;

/**
 * A chain on which `doc` is exactly what it claims to be: the factory names CLONE for the root, CLONE
 * issued it from SIGNER, declares record type VACCINATION, and SIGNER is whitelisted for that type.
 * Every test starts here and breaks ONE thing, so a red test names the term it pins.
 */
function genuineChain(doc: WrappedDoc): ChainCfg {
  const root = doc.signature.merkleRoot;
  return {
    rootIssuers: { [root.toLowerCase()]: CLONE },
    isValid: { [k(CLONE, root)]: true },
    issuedAt: { [k(CLONE, root)]: 1_699_000_000n },
    isRevoked: { [k(CLONE, root)]: false },
    issuedBy: { [k(CLONE, root)]: SIGNER },
    recordTypes: { [CLONE.toLowerCase()]: VACCINATION_KEY },
    whitelist: new Set([k(VACCINATION_KEY, SIGNER)]),
    // The clone is governed by the registry this client is configured with - the matched pair a
    // factory guarantees, since `registry` is written once from the factory's own immutable.
    issuerRegistries: { [CLONE.toLowerCase()]: DEPLOYED_ADDRESSES.IssuerRegistry },
    // ...and the log agrees with the getters: granted at 800_000, anchored at 900_000.
    rootIssuedLogs: { [k(CLONE, root)]: ISSUED_AT_LOG },
    grants: { [k(VACCINATION_KEY, SIGNER)]: [{ kind: "whitelisted", ...GRANTED_AT_LOG }] },
  };
}

/** Install a contract the factory never deployed, answering with attacker-chosen values. */
function withHostile(cfg: ChainCfg, doc: WrappedDoc): ChainCfg {
  const root = doc.signature.merkleRoot;
  return {
    ...cfg,
    isValid: { ...cfg.isValid, [k(HOSTILE, root)]: true },
    issuedAt: { ...cfg.issuedAt, [k(HOSTILE, root)]: 1_699_000_000n },
    issuedBy: { ...cfg.issuedBy, [k(HOSTILE, root)]: SIGNER },
    recordTypes: { ...cfg.recordTypes, [HOSTILE.toLowerCase()]: VACCINATION_KEY },
  };
}

function fakeReader(cfg: ChainCfg): IssuerChainReader {
  const fail = (m: keyof IssuerChainReader) => {
    if (cfg.failing?.has(m)) throw new Error(`${m} read failed`);
  };
  return {
    async rootIssuer(_factory, root) {
      fail("rootIssuer");
      return cfg.rootIssuers?.[root.toLowerCase()] ?? ZERO_ADDR;
    },
    async recordType(addr) {
      fail("recordType");
      return cfg.recordTypes?.[addr.toLowerCase()] ?? `0x${"0".repeat(64)}`;
    },
    async issuedAt(addr, root) {
      fail("issuedAt");
      return cfg.issuedAt?.[k(addr, root)] ?? 0n;
    },
    async isValid(addr, root) {
      fail("isValid");
      return cfg.isValid?.[k(addr, root)] ?? false;
    },
    async isRevoked(addr, root) {
      fail("isRevoked");
      return cfg.isRevoked?.[k(addr, root)] ?? false;
    },
    async issuedBy(addr, root) {
      fail("issuedBy");
      return cfg.issuedBy?.[k(addr, root)] ?? ZERO_ADDR;
    },
    async isWhitelistedFor(_registry, key, signer) {
      fail("isWhitelistedFor");
      return cfg.whitelist?.has(k(key, signer)) ?? false;
    },
  };
}

/** `DogTagIssuer.registry()` - which authority actually gates the clone's `onlyWhitelisted`. */
function fakeAuthorityReader(cfg: ChainCfg): IssuerAuthorityReader {
  return {
    async registryOf(cloneAddr) {
      if (cfg.failing?.has("issuerRegistry")) throw new Error("registry read failed");
      return cfg.issuerRegistries?.[cloneAddr.toLowerCase()] ?? ZERO_ADDR;
    },
  };
}

/** The two `eth_getLogs` the bench makes on its own account, keyed exactly as the real ones are. */
function fakeGrantHistoryReader(cfg: ChainCfg): GrantHistoryReader {
  return {
    async rootIssuedAt(cloneAddr, root) {
      if (cfg.failing?.has("rootIssuedLog")) throw new Error("RootIssued log read failed");
      return cfg.rootIssuedLogs?.[k(cloneAddr, root)] ?? null;
    },
    async grants(_registryAddr, key, signer) {
      if (cfg.failing?.has("whitelistHistory")) throw new Error("grant history read failed");
      return cfg.grants?.[k(key, signer)] ?? [];
    },
  };
}

const domainReader = (domain: string | null): DomainClaimReader => ({
  async claim() {
    return domain === null ? null : { domain, updatedAtBlock: 4242n };
  },
});

const failingDomainReader: DomainClaimReader = {
  async claim() {
    throw new Error("registry unreachable");
  },
};

/**
 * Every reader is injected HERE, including the two log readers.
 *
 * Not a convenience: `runVerificationBench` defaults each reader to a live viem one, so a suite that
 * injected only `reader` would send real `eth_call`/`eth_getLogs` at ROAX from a unit test - slow,
 * flaky, and (for a bench whose whole subject is what a chain answered) capable of going green against
 * state nobody in this file wrote.
 */
async function bench(
  doc: WrappedDoc | Record<string, unknown>,
  cfg: ChainCfg,
  extra: Partial<Parameters<typeof runVerificationBench>[0]> = {},
): Promise<BenchReport> {
  return runVerificationBench({
    wrappedDoc: asRecord(doc),
    reader: fakeReader(cfg),
    authorityReader: fakeAuthorityReader(cfg),
    grantHistoryReader: fakeGrantHistoryReader(cfg),
    blockNumber: 900_100n,
    today: TODAY,
    now: NOW,
    ...extra,
  });
}

function check(report: BenchReport, id: BenchCheckId): BenchCheck {
  const c = report.checks.find((x) => x.id === id);
  if (!c) throw new Error(`no check ${id} in report`);
  return c;
}

const outcomes = (r: BenchReport) =>
  Object.fromEntries(r.checks.map((c) => [c.id, c.outcome])) as Record<BenchCheckId, string>;

// ── the invariant the whole surface is held to ──────────────────────────────────────────────────

describe("the three-state contract", () => {
  it("attaches a reason to every could-not-run and to nothing else", async () => {
    // Sweep several genuinely different worlds so this covers could-not-run rows produced by each
    // distinct path (unreadable chain, absent config, absent field), not just one.
    const doc = validDoc();
    const reports = [
      await bench(doc, genuineChain(doc)),
      await bench(doc, { ...genuineChain(doc), failing: new Set(["rootIssuer"] as const) }),
      await bench(doc, { ...genuineChain(doc), rootIssuers: {} }),
      await bench(asRecord({ nonsense: true }), genuineChain(doc)),
      await bench(stripValidUntil(doc), genuineChain(doc)),
    ];
    let couldNotRunSeen = 0;
    for (const r of reports) {
      for (const c of r.checks) {
        if (c.outcome === "could-not-run") {
          couldNotRunSeen++;
          expect(c.couldNotRunReason, `${c.id} must say why it could not run`).toBeTruthy();
        } else {
          expect(c.couldNotRunReason, `${c.id} is ${c.outcome} and must carry no reason`).toBeUndefined();
        }
        // A finding is always present, whatever the outcome.
        expect(c.finding).toBeTruthy();
      }
    }
    expect(couldNotRunSeen).toBeGreaterThan(0);
  });

  it("still answers integrity when the chain is unreachable, and formats it identically either way", async () => {
    // Integrity is PURE and offline, so an unreachable chain must not take it down with the rest.
    const doc = validDoc();
    const answered = check(await bench(doc, genuineChain(doc)), "integrity");
    const fallback = check(
      await bench(doc, { ...genuineChain(doc), failing: new Set(["rootIssuer"] as const) }),
      "integrity",
    );
    expect(fallback.outcome).toBe("pass");
    expect(fallback.outcome).not.toBe("could-not-run");
    const rootOf = (c: BenchCheck) => c.evidence.find((e) => e.label === "Recomputed root")?.value;
    // The two paths must be indistinguishable by formatting: a decimal on one and 0x-hex on the other
    // reads as two different values for the same root.
    expect(rootOf(fallback)).toBe(rootOf(answered));
    expect(rootOf(fallback)).toMatch(/^0x[0-9a-f]{64}$/);
  });

  it("REPORTS a malformed record rather than throwing - a dead button tells an operator nothing", async () => {
    // The page is "throw any record at the system", so a JSON primitive is a record too. `null` parses,
    // reached `doc.issuer` inside the engine, and threw a TypeError out of the call - discarded by the
    // page's `void runOn(...)`, leaving the PREVIOUS report on screen as though it applied to this input.
    const cfg = genuineChain(validDoc());
    for (const junk of [null, 42, "a string", [1, 2, 3], true]) {
      const r = await runVerificationBench({
        wrappedDoc: junk as unknown as Record<string, unknown>,
        reader: fakeReader(cfg),
        blockNumber: 900_100n,
        today: TODAY,
        now: NOW,
      });
      expect(r.verdict).toBeNull();
      expect(r.verifierError, `${JSON.stringify(junk)} must be reported, not swallowed`).toBeTruthy();
      expect(check(r, "integrity").outcome).toBe("could-not-run");
      expect(r.checks.every((c) => c.outcome !== "pass")).toBe(true);
    }
  });

  it("reports no verdict at all - never `false` - when the verifier could not produce one", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), failing: new Set(["isValid"] as const) });
    // The distinction this pins: an unreachable chain is not a refused credential.
    expect(r.verdict).toBeNull();
    expect(r.verdict).not.toBe(false);
    expect(r.verifierError).toBeTruthy();
  });
});

function stripValidUntil(doc: WrappedDoc): Record<string, unknown> {
  const d = JSON.parse(JSON.stringify(doc));
  delete d.data.credentialSubject.validity;
  return d;
}

// ── the baseline ────────────────────────────────────────────────────────────────────────────────

describe("a genuine credential", () => {
  it("passes every check that can run, and the bench copies the real verifier's verdict", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader(ISSUER.domain),
    });
    expect(r.verdict).toBe(true);
    expect(outcomes(r)).toMatchObject({
      integrity: "pass",
      "issuer-descends-from-factory": "pass",
      "document-names-issuing-contract": "pass",
      "issuer-whitelisted": "pass",
      "anchored-on-chain": "pass",
      "not-revoked": "pass",
      "not-expired": "pass",
      "issuer-domain-claim": "pass",
    });
  });

  it("pins its evidence to the block the reads were made at", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc));
    expect(r.blockNumber).toBe(900_100n);
    const anchor = check(r, "issuer-descends-from-factory");
    expect(anchor.evidence.some((e) => e.label === "Block" && e.value === "900100")).toBe(true);
  });

  it("says so plainly when the reads were NOT pinned to a block", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), { blockNumber: undefined });
    expect(r.blockNumber).toBeNull();
    const blockLine = check(r, "issuer-descends-from-factory").evidence.find((e) => e.label === "Block");
    // It must not name a height it never read, and must not silently omit the caveat either.
    expect(blockLine?.value).toContain("unanchored");
  });
});

// ── the recorder: evidence is OBSERVED, not re-derived ──────────────────────────────────────────

describe("the recording reader", () => {
  it("records which contract each question was actually put to", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc));
    const anchorRead = r.reads.find((x) => x.method === "rootIssuer");
    expect(anchorRead?.contract).toBe(DEPLOYED_ADDRESSES.DogTagIssuerFactory);
    expect(anchorRead?.value?.toLowerCase()).toBe(CLONE.toLowerCase());
    // The verdict-deciding reads went to the FACTORY-RESOLVED clone.
    expect(r.reads.find((x) => x.method === "isValid")?.contract.toLowerCase()).toBe(CLONE.toLowerCase());
  });

  it("re-throws the original failure rather than softening it into a value", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), failing: new Set(["isWhitelistedFor"] as const) });
    const failed = r.reads.find((x) => x.outcome === "failed");
    expect(failed?.method).toBe("isWhitelistedFor");
    expect(failed?.error).toContain("isWhitelistedFor read failed");
    expect(r.verdict).toBeNull();
  });
});

// ── the checks, one broken term at a time ───────────────────────────────────────────────────────

describe("the factory anchor", () => {
  it("FAILS when the factory has no record of the root - we asked, and that is evidence", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), rootIssuers: {} });
    const anchor = check(r, "issuer-descends-from-factory");
    expect(anchor.outcome).toBe("fail");
    // Not evidence about the credential vs could-not-ask: this one IS evidence, so it must not soften.
    expect(anchor.outcome).not.toBe("could-not-run");
  });

  it("COULD-NOT-RUN when the factory read itself failed - we could not ask", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), failing: new Set(["rootIssuer"] as const) });
    const anchor = check(r, "issuer-descends-from-factory");
    expect(anchor.outcome).toBe("could-not-run");
    expect(anchor.outcome).not.toBe("fail");
    expect(anchor.couldNotRunReason).toContain("rootIssuer read failed");
  });
});

describe("the document-names-issuing-contract check", () => {
  it("catches a document repointed at a hostile contract that vouches for it", async () => {
    const doc = validDoc();
    const forged = repointDocumentStore.apply(asRecord(doc));
    expect(forged).not.toBeNull();
    const cfg = withHostile(genuineChain(doc), doc);

    // The fake must be able to model a lying contract, or this test proves nothing.
    expect(await fakeReader(cfg).isValid(HOSTILE, doc.signature.merkleRoot)).toBe(true);

    const r = await bench(forged!.doc, cfg);
    expect(check(r, "document-names-issuing-contract").outcome).toBe("fail");
    // And the reads still went to the clone the FACTORY named, never to the attacker's address.
    expect(r.reads.find((x) => x.method === "isValid")?.contract.toLowerCase()).toBe(CLONE.toLowerCase());
    expect(r.verdict).toBe(false);
  });

  it("cannot run when no contract was resolved to disagree with", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), rootIssuers: {} });
    expect(check(r, "document-names-issuing-contract").outcome).toBe("could-not-run");
  });
});

describe("the issuer-whitelist check", () => {
  it("fails for a signer that is not authorised for the record type", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), whitelist: new Set<string>() });
    expect(check(r, "issuer-whitelisted").outcome).toBe("fail");
    expect(r.verdict).toBe(false);
  });

  it("could-not-run - never pass - when the clone never issued the root", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), issuedBy: {} });
    const wl = check(r, "issuer-whitelisted");
    expect(wl.outcome).toBe("could-not-run");
    expect(wl.outcome).not.toBe("pass");
    expect(wl.couldNotRunReason).toContain("never issued this root");
  });
});

// ── THE INVARIANT: no row asserts from a value the recorder never observed ──────────────────────

/** Every contract a check CITES as having answered. Only `readEvidence` can produce this shape. */
function citedContracts(r: BenchReport): string[] {
  const out: string[] = [];
  for (const c of r.checks) {
    for (const e of c.evidence) {
      const m = / on (0x[0-9a-fA-F]{40})\b/.exec(e.source);
      if (m?.[1]) out.push(m[1].toLowerCase());
    }
  }
  return out;
}

describe("no row asserts from a value the recorder never observed", () => {
  it("never cites a contract that is absent from the recorded reads", async () => {
    const doc = validDoc();
    const forged = repointDocumentStore.apply(asRecord(doc))!.doc;
    // The unresolved-clone worlds are the ones where the class manifests: the verifier fills
    // [issuedAt, onchainValid, revoked] with [0n, false, false] having made NO read, and reports the
    // DOCUMENT'S OWN address as `issuerAddr`. A sweep of genuine chains would pass before the fix.
    const worlds = [
      await bench(doc, genuineChain(doc), {
        domainRegistryAddr: DOMAIN_REGISTRY,
        domainClaimReader: domainReader(ISSUER.domain),
      }),
      await bench(doc, { ...genuineChain(doc), rootIssuers: {} }),
      await bench(forged, { ...withHostile(genuineChain(doc), doc), rootIssuers: {} }),
      await bench(doc, { ...genuineChain(doc), failing: new Set(["issuedAt"] as const) }),
      await bench(doc, { ...genuineChain(doc), issuedBy: {} }),
      await bench(asRecord({ nonsense: true }), genuineChain(doc)),
    ];
    let citations = 0;
    for (const r of worlds) {
      const observed = new Set(r.reads.map((x) => x.contract.toLowerCase()));
      for (const addr of citedContracts(r)) {
        citations++;
        expect(
          observed.has(addr),
          `an evidence line cites ${addr} as having answered, but no read was recorded against it`,
        ).toBe(true);
      }
    }
    // ...and citations really ARE made, or this asserts nothing.
    expect(citations).toBeGreaterThan(0);
  });

  it("reports not-revoked as could-not-run - NEVER pass - when no clone was resolved to ask", async () => {
    const doc = validDoc();
    const forged = repointDocumentStore.apply(asRecord(doc))!.doc;
    const r = await bench(forged, { ...withHostile(genuineChain(doc), doc), rootIssuers: {} });

    const revoked = check(r, "not-revoked");
    expect(revoked.outcome).toBe("could-not-run");
    expect(revoked.outcome).not.toBe("pass");
    expect(revoked.couldNotRunReason).toContain("isRevoked");
    const anchored = check(r, "anchored-on-chain");
    expect(anchored.outcome).toBe("could-not-run");
    expect(anchored.outcome).not.toBe("pass");

    // No row anywhere may name the attacker's address as a contract that answered - it was never asked.
    expect(citedContracts(r)).not.toContain(HOSTILE.toLowerCase());
    expect(r.reads.some((x) => x.contract.toLowerCase() === HOSTILE.toLowerCase())).toBe(false);

    // Nothing is lost: the anchor row already reports the definite failure, which is its honest home.
    expect(check(r, "issuer-descends-from-factory").outcome).toBe("fail");
  });

  it("does not claim the registry was asked when no isWhitelistedFor read was made", async () => {
    const doc = validDoc();
    const r = await bench(doc, { ...genuineChain(doc), rootIssuers: {} });
    const wl = check(r, "issuer-whitelisted");
    expect(wl.outcome).toBe("could-not-run");
    expect(r.reads.some((x) => x.method === "isWhitelistedFor")).toBe(false);
    const registryLine = wl.evidence.find((e) => e.label.startsWith("Registry"));
    expect(registryLine?.value).toContain("not consulted");
    expect(registryLine?.source).not.toContain(" on 0x");
  });
});

describe("the revocation and anchoring checks", () => {
  it("separates a revoked credential from one that was never anchored", async () => {
    const doc = validDoc();
    const root = doc.signature.merkleRoot;

    const revoked = await bench(doc, {
      ...genuineChain(doc),
      isRevoked: { [k(CLONE, root)]: true },
      isValid: { [k(CLONE, root)]: false },
    });
    expect(check(revoked, "not-revoked").outcome).toBe("fail");
    expect(check(revoked, "anchored-on-chain").outcome).toBe("pass");

    const neverIssued = await bench(doc, { ...genuineChain(doc), issuedAt: {}, isValid: {} });
    expect(check(neverIssued, "anchored-on-chain").outcome).toBe("fail");
    expect(check(neverIssued, "not-revoked").outcome).toBe("pass");
  });
});

describe("the expiry check", () => {
  it("fails a lapsed window and passes a live one, comparing ISO dates as plain strings", async () => {
    const lapsed = validDoc("2020-05-01");
    const live = validDoc("2027-01-11");
    const lapsedReport = await bench(lapsed, genuineChain(lapsed));
    expect(check(lapsedReport, "not-expired").outcome).toBe("fail");
    expect(check(await bench(live, genuineChain(live)), "not-expired").outcome).toBe("pass");
    // The CLEAN expired case: genuinely lapsed and untampered. Without this, nothing in the suite
    // separates "expired" from "someone edited the window", and the two have different remedies.
    expect(check(lapsedReport, "integrity").outcome).toBe("pass");
    expect(check(lapsedReport, "anchored-on-chain").outcome).toBe("pass");
    // And it is on-chain VALID while being expired - the whole reason expiry needs its own row.
    expect(lapsedReport.verdict).toBe(true);
  });

  it("could-not-run - never pass - for a document carrying no validity window", async () => {
    const doc = validDoc();
    const stripped = stripValidUntil(doc);
    const c = check(await bench(stripped, genuineChain(doc)), "not-expired");
    expect(c.outcome).toBe("could-not-run");
    expect(c.outcome).not.toBe("pass");
    expect(c.couldNotRunReason).toContain("no expiry claim to test");
  });

  it("reads validUntil out of the Merkle-covered data, unpacking the salted leaf", () => {
    expect(validUntilOf(asRecord(validDoc("2031-02-02")))).toEqual({
      keyPath: "credentialSubject.validity.validUntil",
      value: "2031-02-02",
      usable: true,
    });
  });

  it("could-not-run - never a red expired row - for a BLANK validity leaf", () => {
    // `today > ""` is true for every date, so the blank leaf used to render as "the validity window
    // lapsed on ." - an accusation against a credential whose window was simply left empty at issuance.
    const blank = setValidUntilValue(validDoc(), "credentialSubject.validity.validUntil", "");
    const window = validUntilOf(blank);
    expect(window?.usable).toBe(false);
    return bench(blank, genuineChain(validDoc())).then((r) => {
      const c = check(r, "not-expired");
      expect(c.outcome).toBe("could-not-run");
      expect(c.outcome).not.toBe("fail");
      expect(c.couldNotRunReason).toContain("no value at all");
    });
  });

  it("could-not-run for a validity leaf that is present but not an ISO date", async () => {
    const junk = setValidUntilValue(validDoc(), "credentialSubject.validity.validUntil", "next spring");
    const c = check(await bench(junk, genuineChain(validDoc())), "not-expired");
    expect(c.outcome).toBe("could-not-run");
    expect(c.couldNotRunReason).toContain("not an ISO");
  });
});

// ── the canonical validUntil chain: one list, three issuers ─────────────────────────────────────

describe("the canonical validUntil chain", () => {
  it("is the three tiers the other surfaces read, and never credentialSubject.validUntil", () => {
    // `credentialSubject.validUntil` is emitted by NO issuer. Substituting it for tier 2 made every EU
    // health certificate report "no validity window" on the one surface built to expose expiry.
    expect([...VALID_UNTIL_KEYPATHS]).toEqual([
      "credentialSubject.validity.validUntil",
      "credentialSubject.rabiesValidUntil",
      "validUntil",
    ]);
  });

  it("finds and EVALUATES EU_HEALTH_CERT's flat rabiesValidUntil leaf", async () => {
    const live = euHealthCertDoc("2029-01-14");
    expect(validUntilOf(asRecord(live))).toEqual({
      keyPath: "credentialSubject.rabiesValidUntil",
      value: "2029-01-14",
      usable: true,
    });
    const liveRow = check(await bench(live, genuineChain(live)), "not-expired");
    expect(liveRow.outcome).toBe("pass");
    expect(liveRow.outcome).not.toBe("could-not-run");

    const lapsed = euHealthCertDoc("2020-05-01");
    expect(check(await bench(lapsed, genuineChain(lapsed)), "not-expired").outcome).toBe("fail");
  });

  it("finds VACCINATION's top-level validUntil, a SIBLING of credentialSubject", async () => {
    const live = vaccinationDoc("2027-06-30");
    expect(validUntilOf(asRecord(live))).toEqual({
      keyPath: "validUntil",
      value: "2027-06-30",
      usable: true,
    });
    expect(check(await bench(live, genuineChain(live)), "not-expired").outcome).toBe("pass");
    const lapsed = vaccinationDoc("2019-03-03");
    expect(check(await bench(lapsed, genuineChain(lapsed)), "not-expired").outcome).toBe("fail");
  });

  it("lets both validity mutations apply to every issuer's shape, not just tier one", () => {
    // The mutations resolve the window through the SAME chain the check reads, so a record type the
    // check can evaluate is always one the mutations can edit. Divergence here would render as "Not
    // applicable to this record" - a claim about the record rather than about the bench.
    for (const doc of [validDoc(), euHealthCertDoc(), vaccinationDoc()]) {
      expect(expireValidityWindow.apply(asRecord(doc))).not.toBeNull();
      expect(extendValidityWindow.apply(asRecord(doc))).not.toBeNull();
    }
  });
});

describe("the issuer-domain checks", () => {
  it("passes when the document's domain matches the issuer's on-chain claim", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader(ISSUER.domain),
    });
    expect(check(r, "issuer-domain-claim").outcome).toBe("pass");
  });

  it("fails when the document claims a domain the issuer never published", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader("ministry-of-health.example"),
    });
    expect(check(r, "issuer-domain-claim").outcome).toBe("fail");
  });

  it("could-not-run for an unconfigured registry, an unreadable one, and an unpublished claim - each with its own reason", async () => {
    const doc = validDoc();
    const unconfigured = await bench(doc, genuineChain(doc));
    const unreadable = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: failingDomainReader,
    });
    const noClaim = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader(null),
    });
    for (const r of [unconfigured, unreadable, noClaim]) {
      expect(check(r, "issuer-domain-claim").outcome).toBe("could-not-run");
    }
    // Three DIFFERENT reasons: collapsing them is how "we never asked" comes to read as "nothing there".
    const reasons = [unconfigured, unreadable, noClaim].map(
      (r) => check(r, "issuer-domain-claim").couldNotRunReason,
    );
    expect(new Set(reasons).size).toBe(3);
    expect(reasons[0]).toContain("No IssuerDomainRegistry address is configured");
    expect(reasons[1]).toContain("registry unreachable");
    expect(reasons[2]).toContain("holds no binding");
  });

  it("never lets the on-chain claim imply that DNS agreed", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader(ISSUER.domain),
    });
    expect(check(r, "issuer-domain-claim").outcome).toBe("pass");
    // The DNS half is a SEPARATE row and is never satisfied by the registry read.
    const dns = check(r, "issuer-domain-dns");
    expect(dns.outcome).toBe("could-not-run");
    expect(dns.couldNotRunReason).toContain("server-side");
  });
});

// ── the adversarial half ────────────────────────────────────────────────────────────────────────

describe("the mutations trip the specific check that catches them", () => {
  it("a tampered covered field trips integrity ALONE - the chain still says the root is fine", async () => {
    const doc = validDoc();
    const m = tamperCoveredField.apply(asRecord(doc));
    expect(m).not.toBeNull();
    const r = await bench(m!.doc, genuineChain(doc));
    expect(check(r, "integrity").outcome).toBe("fail");
    // The claimed root is untouched, so every on-chain read still passes. A surface reporting only the
    // chain's answer would call this valid - which is why integrity is its own row.
    expect(check(r, "anchored-on-chain").outcome).toBe("pass");
    expect(check(r, "not-revoked").outcome).toBe("pass");
    expect(check(r, "issuer-whitelisted").outcome).toBe("pass");
    expect(r.verdict).toBe(false);
  });

  it("relabelling the issuer name trips NOTHING here, and the mutation says so honestly", async () => {
    const doc = validDoc();
    const m = relabelIssuerName.apply(asRecord(doc));
    expect(m).not.toBeNull();
    const before = await bench(doc, genuineChain(doc));
    const after = await bench(m!.doc, genuineChain(doc));
    // Every check lands exactly where it did before the lie was told.
    expect(outcomes(after)).toEqual(outcomes(before));
    expect(after.verdict).toBe(true);
    // The declaration must match that reality rather than flatter the bench.
    expect(relabelIssuerName.caughtBy).toEqual([]);
    expect(relabelIssuerName.caughtElsewhere).toContain("name()");
  });

  it("a forged validity window satisfies the expiry row and is caught by integrity instead", async () => {
    const lapsed = validDoc("2020-05-01");
    const m = extendValidityWindow.apply(asRecord(lapsed));
    expect(m).not.toBeNull();
    const r = await bench(m!.doc, genuineChain(lapsed));
    // The expiry row reads the date the DOCUMENT states, so the forgery satisfies it...
    expect(check(r, "not-expired").outcome).toBe("pass");
    // ...and integrity is what actually protects the window.
    expect(check(r, "integrity").outcome).toBe("fail");
  });

  it("an expired record is refused by the expiry row while the chain still calls it anchored", async () => {
    const doc = validDoc();
    const m = expireValidityWindow.apply(asRecord(doc));
    expect(m).not.toBeNull();
    const r = await bench(m!.doc, genuineChain(doc));
    expect(check(r, "not-expired").outcome).toBe("fail");
    expect(check(r, "anchored-on-chain").outcome).toBe("pass");
    expect(check(r, "not-revoked").outcome).toBe("pass");
  });

  it("refuses to apply rather than inventing a field the record does not have", () => {
    const noWindow = stripValidUntil(validDoc());
    expect(expireValidityWindow.apply(noWindow)).toBeNull();
    expect(extendValidityWindow.apply(noWindow)).toBeNull();
  });

  it("declares only real check ids, and never claims to catch and miss the same check", async () => {
    const doc = validDoc();
    const ids = new Set((await bench(doc, genuineChain(doc))).checks.map((c) => c.id));
    for (const m of BENCH_MUTATIONS) {
      for (const id of m.caughtBy) expect(ids, `${m.id} caughtBy ${id}`).toContain(id);
      for (const u of m.notCaughtBy) {
        expect(ids, `${m.id} notCaughtBy ${u.id}`).toContain(u.id);
        expect(m.caughtBy, `${m.id} cannot both catch and miss ${u.id}`).not.toContain(u.id);
        expect(u.why.length).toBeGreaterThan(20);
      }
    }
  });
});

// ── the verdict's scope, stated rather than left to be inferred ─────────────────────────────────

describe("which rows the verifier's verdict folds in", () => {
  it("marks expiry and the issuer-domain rows as reported BESIDE the verdict, not inside it", async () => {
    const doc = validDoc();
    const r = await bench(doc, genuineChain(doc), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader(ISSUER.domain),
    });
    const gates = Object.fromEntries(r.checks.map((c) => [c.id, c.gatesVerdict]));
    expect(gates).toEqual({
      integrity: true,
      "issuer-descends-from-factory": true,
      "document-names-issuing-contract": true,
      "issuer-whitelisted": true,
      "anchored-on-chain": true,
      "not-revoked": true,
      "not-expired": false,
      "issuer-domain-claim": false,
      "issuer-domain-dns": false,
      // Observations about the verdict's own inputs. Both must stay `false`: the verifier's formula
      // is `integrity && onchain && issuerWhitelisted === true` and this module copies it rather than
      // competing with it, so a row promoted to gating here would be the bench inventing a verdict.
      "registry-governs-issuer": false,
      "whitelisted-at-issuance": false,
    });
  });

  it("a valid verdict over a red row happens ONLY where the row is marked as not gating", async () => {
    // This is the property that stops the page reading as self-contradictory: every red row sitting
    // under a `true` verdict must be one the verifier genuinely does not consider.
    const lapsed = validDoc("2020-05-01");
    const r = await bench(lapsed, genuineChain(lapsed), {
      domainRegistryAddr: DOMAIN_REGISTRY,
      domainClaimReader: domainReader("somewhere-else.example"),
    });
    expect(r.verdict).toBe(true);
    const redAndGating = r.checks.filter((c) => c.outcome === "fail" && c.gatesVerdict);
    expect(redAndGating.map((c) => c.id)).toEqual([]);
    // ...and there really ARE red rows, or this asserts nothing.
    expect(r.checks.filter((c) => c.outcome === "fail").map((c) => c.id).sort()).toEqual([
      "issuer-domain-claim",
      "not-expired",
    ]);
  });

  it("a refused verdict is always explained by at least one row marked as gating", async () => {
    // The other direction, and the dangerous one. The test above only constrains `verdict: true`, so a
    // GATING row mislabelled as "not in the verdict" would slip past it - and that mislabelling is what
    // would let a red integrity row be excused on the page as merely informational.
    const doc = validDoc();
    const cases: BenchReport[] = [
      // integrity red
      await bench(tamperCoveredField.apply(asRecord(doc))!.doc, genuineChain(doc)),
      // whitelist red
      await bench(doc, { ...genuineChain(doc), whitelist: new Set<string>() }),
      // anchor red
      await bench(doc, { ...genuineChain(doc), rootIssuers: {} }),
      // store-agreement red
      await bench(repointDocumentStore.apply(asRecord(doc))!.doc, withHostile(genuineChain(doc), doc)),
    ];
    for (const r of cases) {
      expect(r.verdict).toBe(false);
      const blocking = r.checks.filter((c) => c.gatesVerdict && c.outcome !== "pass");
      expect(
        blocking.map((c) => c.id),
        "a false verdict with no gating row to explain it means the page cannot say why",
      ).not.toEqual([]);
    }
  });
});

describe("docFromShareResponse", () => {
  it("unwraps both envelope spellings and passes a bare document through", () => {
    const doc = { data: {}, signature: {} };
    expect(docFromShareResponse({ wrappedDoc: doc })).toBe(doc);
    expect(docFromShareResponse({ wrapped_doc: doc })).toBe(doc);
    expect(docFromShareResponse(doc)).toBe(doc);
  });

  it("does not mistake a non-object envelope value for the document", () => {
    // `{wrappedDoc: "..."}` must fall through to the body rather than returning a string as the doc.
    const body = { wrappedDoc: "not-an-object" };
    expect(docFromShareResponse(body)).toBe(body);
    expect(docFromShareResponse(null)).toEqual({});
    expect(docFromShareResponse("nope")).toEqual({});
  });
});
