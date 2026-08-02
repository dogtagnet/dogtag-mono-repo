// Submodule imports rather than the `@dogtag/standard` barrel - see the note at the top of
// `./verifyCredential.ts`. The barrel drags in `consent.ts` -> `circomlibjs`, which touches the Node
// `Buffer` global at module init and throws in a browser.
import { wrapDocument } from "@dogtag/standard/wrap";
// `types` is a leaf module of plain declarations plus the `TypeTag` enum - no `circomlibjs`, so it is
// browser-safe on the same terms as `wrap`.
import { TypeTag, type IssuerMeta, type WrappedDoc } from "@dogtag/standard/types";
import { DEPLOYED_ADDRESSES, recordTypeKey, sortLogPoints, ZERO_ADDRESS } from "./contracts";
import type { AuthorityGeneration, LogPoint, WhitelistGrantEvent } from "./contracts";
import type { IssuerChainReader } from "./verifyCredential";
import {
  runVerificationBench,
  type BenchCheckId,
  type BenchReport,
  type CheckOutcome,
  type GrantHistoryReader,
  type IssuerAuthorityReader,
} from "./verificationBench";

/**
 * The attack catalogue: deliberately fraudulent records, each run through the REAL verification path,
 * each declaring WHICH check must refuse it.
 *
 * # Why "which check", and not "was it refused"
 *
 * A record refused for the wrong reason is a latent bug wearing a green tick. If a forged issuer is
 * caught by the expiry row because the fixture happened to be stale, the suite is green and the issuer
 * pillar is dead. So every scenario declares an EXPECTED OUTCOME FOR EVERY CHECK - the whole vector,
 * not one row - and the tests assert the vector. A scenario that starts passing for a different reason
 * fails.
 *
 * # Why the chain is scripted rather than live
 *
 * These scenarios need chain states that a live chain cannot be asked to produce on demand - a signer
 * delisted after it issued, a contract the factory never deployed vouching for a root, a registry that
 * does not govern the clone. Each carries its own {@link ScenarioWorld}: a document plus a complete set
 * of readers. Nothing here touches the network, so the catalogue runs identically in a unit test and in
 * the browser, and the page can render live outcomes beside the declared expectations.
 *
 * EVERY read is keyed on the contract it is put to - the six `DogTagIssuer`/factory getters and the
 * `RootIssued` log on the contract address, `DogTagIssuer.registry()` on the clone whose authority is
 * being asked for, and the `Whitelisted`/`Delisted` grant log on the REGISTRY address it is read from.
 * A fake that ignored which contract it was asked about could not represent "the hostile contract
 * answers `true` while the clone the factory named answers `false`", nor "the grant is in the registry
 * the clone names and only there", and every forged-issuer scenario written against one would pass for
 * the wrong reason - the trap recorded against `MockChain` in
 * `crates/dogtag-standard-rs/src/verify.rs`.
 *
 * That claim was once written as a blanket "ADDRESS-KEYED throughout" while `grants` in fact discarded
 * its registry argument, and the blanket wording is why the gap survived review: it read as a property
 * of the module rather than of each reader. Stated per-reader it is checkable, and
 * `benchScenarios.test.ts` asserts the registry keying directly so a reader that stopped honouring its
 * address goes red rather than quietly agreeing with itself. A claim broader than the code is its own
 * defect - if a future reader cannot be keyed, narrow this sentence rather than widening it.
 *
 * # What a scenario may NOT do
 *
 * IT MAY NOT ADJUST ITS EXPECTATION TO WHATEVER THE CODE CURRENTLY PRINTS. An expectation edited down
 * to match the implementation is a test that certifies the bug - the single failure mode that would
 * make this whole catalogue worthless, because it converts every future regression into a green tick.
 *
 * So where the implementation is wrong, {@link BenchScenario.expected} goes on stating the CORRECT
 * answer and the gap is recorded in {@link BenchScenario.knownDefect}, which pins today's behaviour
 * separately. The suite asserts BOTH, plus that the two still differ - so the day the defect is fixed
 * the scenario goes red and whoever fixed it must delete the field, rather than the finding quietly
 * evaporating.
 *
 * NO SCENARIO CARRIES A PIN TODAY, and `benchScenarios.test.ts` asserts that set is empty.
 * {@link signerDelistedAfterIssuance} is the worked example of the whole mechanism: it pinned the
 * forward-only delisting defect until the verdict formula moved, at which point the scenario went red
 * and the pin had to be deleted rather than the finding evaporating. The machinery is kept for the
 * next finding, not because one is outstanding.
 */

// ── the scripted world ──────────────────────────────────────────────────────────────────────────

/** The registry and factory this client is configured with. Never sourced from a document. */
export const SCENARIO_REGISTRY = DEPLOYED_ADDRESSES.IssuerRegistry;
export const SCENARIO_FACTORY = DEPLOYED_ADDRESSES.DogTagIssuerFactory;

/** The clone the factory names for an honest root. */
const CLONE = "0x00000000000000000000000000000000c10e0001";
/** A contract the factory never deployed, run by whoever forged the document. */
const HOSTILE = "0x000000000000000000000000000000000000ba0b";
/** A second, unrelated `IssuerRegistry` instance - the misconfiguration case. */
export const FOREIGN_REGISTRY = "0x00000000000000000000000000000000f06e1600";
/** The signer that anchored the root on chain (`clone.issuedBy[R]`). */
export const SIGNER = "0xabc0000000000000000000000000000000000abc";

const RECORD_TYPE = "VACCINATION";
export const RECORD_TYPE_KEY = recordTypeKey(RECORD_TYPE);
/** A record type the same signer holds no grant for - the relabelling target. */
const OTHER_RECORD_TYPE = "TRAVEL_CLEARANCE";

/** The report's block, and the log heights the scenarios sequence themselves against. */
const AT_BLOCK = 1_000_000n;
const GRANTED = { blockNumber: 700_000n, logIndex: 2 };
const ANCHORED = { blockNumber: 800_000n, logIndex: 5 };
const DELISTED_BEFORE = { blockNumber: 750_000n, logIndex: 1 };
const DELISTED_AFTER = { blockNumber: 900_000n, logIndex: 3 };

/** Frozen so a scenario's root - and therefore every assertion about it - is stable across runs. */
const NOW = 1_700_000_000;
const TODAY = "2023-11-14";
/** Comfortably in the future at {@link TODAY}, so no scenario is caught by expiry by accident. */
const VALID_UNTIL = "2027-01-11";

const ISSUER: IssuerMeta = {
  name: "Seaport Animal Hospital",
  domain: "vet.seaport.example",
  documentStore: CLONE,
  recordType: RECORD_TYPE,
};

/** Deterministic salts: a fixture whose root moved between runs could not be asserted about. */
function fixedSalts() {
  let seq = 0;
  return () => new Uint8Array(16).fill(++seq);
}

function genuineDoc(issuer: IssuerMeta = ISSUER): WrappedDoc {
  return wrapDocument(
    {
      credentialSubject: {
        dogTagId: { tag: TypeTag.Integer, value: "42" },
        name: { tag: TypeTag.String, value: "Rex" },
        validity: { validUntil: { tag: TypeTag.String, value: VALID_UNTIL } },
      },
    },
    issuer,
    fixedSalts(),
  );
}

const asRecord = (d: WrappedDoc) => JSON.parse(JSON.stringify(d)) as Record<string, unknown>;
const k = (a: string, b: string) => `${a.toLowerCase()}|${b.toLowerCase()}`;
/**
 * The registry-scoped key: `IssuerRegistry._wl` and its `Whitelisted`/`Delisted` events are PER
 * CONTRACT, so both the current-state set and the grant log are properties of one registry instance.
 * Keying them on the pair alone made a foreign registry answer with the configured one's grants, which
 * is precisely the state `foreignRegistry` exists to model.
 */
const k3 = (registry: string, a: string, b: string) => `${registry.toLowerCase()}|${k(a, b)}`;

/** The scripted chain a scenario runs against. Absent entries are the chain's own "no" answers. */
interface ChainScript {
  /** factory `rootIssuer`: root -> clone. Absent == the factory has NO record of that root. */
  rootIssuers?: Record<string, string>;
  isValid?: Record<string, boolean>;
  issuedAt?: Record<string, bigint>;
  isRevoked?: Record<string, boolean>;
  issuedBy?: Record<string, string>;
  recordTypes?: Record<string, string>;
  /** contract -> the registry its own `onlyWhitelisted` consults. */
  issuerRegistries?: Record<string, string>;
  rootIssuedLogs?: Record<string, LogPoint>;
  /** `(registry, recordTypeKey, signer)` -> that REGISTRY's own grant log, oldest first. */
  grants?: Record<string, WhitelistGrantEvent[]>;
  /**
   * registry -> which generation's vocabulary that authority speaks, as the successor probe would
   * establish it.
   *
   * ABSENT MEANS `"legacy"`, and that default is stated here rather than left to fall through: every
   * scenario in this catalogue models a generation-1 chain, so an authority whose grant log is empty
   * really has answered, and its emptiness really is evidence about the credential. Making the
   * default "undetermined" instead would silently convert every scripted refusal into an unresolved
   * pillar - the catalogue would still be green while asserting nothing.
   */
  authorityGenerations?: Record<string, AuthorityGeneration>;
  /** Every read throws with this message - the wrong-chain / unreachable-endpoint case. */
  allReadsFail?: string;
}

/**
 * A chain on which the document is exactly what it claims: the factory names CLONE, CLONE anchored the
 * root from SIGNER at {@link ANCHORED}, declares record type VACCINATION, is governed by the registry
 * this client is configured with, and SIGNER was granted that record type at {@link GRANTED} and still
 * holds it. Every scenario starts here and breaks ONE thing.
 */
function honestChain(root: string): ChainScript {
  return {
    rootIssuers: { [root.toLowerCase()]: CLONE },
    isValid: { [k(CLONE, root)]: true },
    issuedAt: { [k(CLONE, root)]: 1_699_000_000n },
    isRevoked: { [k(CLONE, root)]: false },
    issuedBy: { [k(CLONE, root)]: SIGNER },
    recordTypes: { [CLONE.toLowerCase()]: RECORD_TYPE_KEY },
    issuerRegistries: { [CLONE.toLowerCase()]: SCENARIO_REGISTRY },
    rootIssuedLogs: { [k(CLONE, root)]: ANCHORED },
    // Recorded in the registry that GOVERNS the clone - which here is also the one this client is
    // configured with. `issue()` is `onlyWhitelisted` against the clone's own `registry` slot, so an
    // honest anchoring implies the grant is in THAT registry's log and nowhere else.
    grants: {
      [k3(SCENARIO_REGISTRY, RECORD_TYPE_KEY, SIGNER)]: [{ kind: "whitelisted", ...GRANTED }],
    },
  };
}

/** Install a contract the factory never deployed, answering with attacker-chosen values. */
function withHostileContract(script: ChainScript, root: string): ChainScript {
  return {
    ...script,
    isValid: { ...script.isValid, [k(HOSTILE, root)]: true },
    issuedAt: { ...script.issuedAt, [k(HOSTILE, root)]: 1_699_000_000n },
    isRevoked: { ...script.isRevoked, [k(HOSTILE, root)]: false },
    issuedBy: { ...script.issuedBy, [k(HOSTILE, root)]: SIGNER },
    recordTypes: { ...script.recordTypes, [HOSTILE.toLowerCase()]: RECORD_TYPE_KEY },
    issuerRegistries: { ...script.issuerRegistries, [HOSTILE.toLowerCase()]: SCENARIO_REGISTRY },
  };
}

export interface ScenarioReaders {
  reader: IssuerChainReader;
  authorityReader: IssuerAuthorityReader;
  grantHistoryReader: GrantHistoryReader;
}

function readersFor(script: ChainScript): ScenarioReaders {
  const guard = () => {
    if (script.allReadsFail) throw new Error(script.allReadsFail);
  };
  return {
    reader: {
      async rootIssuer(_factory, root) {
        guard();
        return script.rootIssuers?.[root.toLowerCase()] ?? ZERO_ADDRESS;
      },
      async recordType(addr) {
        guard();
        return script.recordTypes?.[addr.toLowerCase()] ?? `0x${"0".repeat(64)}`;
      },
      async issuedAt(addr, root) {
        guard();
        return script.issuedAt?.[k(addr, root)] ?? 0n;
      },
      async isValid(addr, root) {
        guard();
        return script.isValid?.[k(addr, root)] ?? false;
      },
      async isRevoked(addr, root) {
        guard();
        return script.isRevoked?.[k(addr, root)] ?? false;
      },
      async issuedBy(addr, root) {
        guard();
        return script.issuedBy?.[k(addr, root)] ?? ZERO_ADDRESS;
      },
      // The verifier's own pillar reads the SAME three script maps the bench's independent
      // `grantHistoryReader` below does. One fake feeds both, so the two rows cannot silently be
      // shown different chains - and a scenario states its chain once rather than twice.
      async issuerRegistry(cloneAddr) {
        guard();
        return script.issuerRegistries?.[cloneAddr.toLowerCase()] ?? ZERO_ADDRESS;
      },
      async rootIssuedAt(cloneAddr, root) {
        guard();
        return script.rootIssuedLogs?.[k(cloneAddr, root)] ?? null;
      },
      async grantHistory(registryAddr, key, signer) {
        guard();
        return sortLogPoints(script.grants?.[k3(registryAddr, key, signer)] ?? []);
      },
      // The live probe never throws - a read it could not put is `"undetermined"` - so neither does
      // this. `allReadsFail` is still honoured, because the wrong-chain scenario's whole point is
      // that NO address-bound read reaches a peer reporting the wrong chain: that must show up as an
      // unestablished generation, not as a scripted `"legacy"` the fake invented.
      async authorityGeneration(registryAddr) {
        if (script.allReadsFail) return "undetermined";
        return script.authorityGenerations?.[registryAddr.toLowerCase()] ?? "legacy";
      },
    },
    authorityReader: {
      async registryOf(cloneAddr) {
        guard();
        return script.issuerRegistries?.[cloneAddr.toLowerCase()] ?? ZERO_ADDRESS;
      },
    },
    grantHistoryReader: {
      async rootIssuedAt(cloneAddr, root) {
        guard();
        return script.rootIssuedLogs?.[k(cloneAddr, root)] ?? null;
      },
      // Keyed on the REGISTRY as well as the pair. A fake that discarded this argument would return
      // the same history whichever authority was asked, so `foreignRegistry` would look correct while
      // the production code read the wrong contract - which is exactly what happened.
      async grants(registryAddr, key, signer) {
        guard();
        // Sorted here rather than trusted from the fixture, exactly as the live reader sorts what
        // `eth_getLogs` returns - so a scenario cannot accidentally rely on declaration order.
        return sortLogPoints(script.grants?.[k3(registryAddr, key, signer)] ?? []);
      },
      // Reads the SAME script map the verifier's reader above does, so the gating row and this
      // advisory one cannot be shown different chains. Keyed on the registry for the same reason
      // `grants` is: this is a property of one authority instance.
      async authorityGeneration(registryAddr) {
        if (script.allReadsFail) return "undetermined";
        return script.authorityGenerations?.[registryAddr.toLowerCase()] ?? "legacy";
      },
    },
  };
}

/** Everything one scenario needs to run - a document plus a complete, network-free chain. */
export interface ScenarioWorld extends ScenarioReaders {
  wrappedDoc: Record<string, unknown>;
  registryAddr: string;
  factoryAddr: string;
  blockNumber: bigint;
  today: string;
  now: number;
}

// ── the scenarios ───────────────────────────────────────────────────────────────────────────────

/** A check this scenario is expected to walk straight past, and why it does. */
export interface ScenarioBlindSpot {
  id: BenchCheckId;
  why: string;
}

export interface BenchScenario {
  id: string;
  title: string;
  /** The lie, or the situation, in plain language. */
  tells: string;
  /**
   * Whether the protocol says this record is GENUINE and must survive verification.
   *
   * `true` here is the sharp end of the catalogue: a fraud suite that only ever asserts refusals
   * cannot notice a verifier that refuses everything, and over-refusal is a real failure mode - it
   * renders honest credentials as forgeries.
   */
  mustVerify: boolean;
  /**
   * The checks whose refusal is the POINT of this scenario. Asserted individually, on top of the full
   * expected vector, so a rename or a reshuffle cannot quietly leave the named check unexercised.
   */
  refusedBy: BenchCheckId[];
  /** Checks that legitimately do not object, each with the reason. Half the content. */
  blindSpots: ScenarioBlindSpot[];
  /** Anything a reader needs in order not to over-read the result. */
  notes?: string;
  /**
   * What SHOULD happen, per the protocol - for every check, never a partial vector.
   *
   * This is the CORRECT answer and stays correct even where the code disagrees. An expectation edited
   * down to whatever the implementation currently prints is a test that certifies the bug, so where
   * the two differ the gap is recorded in {@link knownDefect} and this field is left alone.
   */
  expected: Record<BenchCheckId, CheckOutcome>;
  /** The verdict the protocol requires. `null` means the verifier should produce none at all. */
  expectedVerdict: boolean | null;
  /**
   * A DOCUMENTED DEFECT: the code does not do what {@link expected} says it should.
   *
   * Present only where the implementation is wrong. It pins the current behaviour so a change is
   * noticed, WITHOUT moving the expectation - the suite asserts both, and additionally asserts that
   * the two still differ, so the day the defect is fixed this scenario goes red and whoever fixed it
   * must delete the field rather than the finding quietly evaporating.
   */
  knownDefect?: {
    /** What the bench actually produces today, for every check. */
    observed: Record<BenchCheckId, CheckOutcome>;
    /** The verdict it actually produces today. */
    observedVerdict: boolean | null;
    /** Why the correct answer is the correct one, with its source. Rendered in the UI in full. */
    statement: string;
  };
  build(): ScenarioWorld;
}

/** Build a world from a document and a chain script. */
function world(doc: Record<string, unknown>, script: ChainScript): ScenarioWorld {
  return {
    wrappedDoc: doc,
    ...readersFor(script),
    registryAddr: SCENARIO_REGISTRY,
    factoryAddr: SCENARIO_FACTORY,
    blockNumber: AT_BLOCK,
    today: TODAY,
    now: NOW,
  };
}

/**
 * The outcome vector of an honest record, as the shared baseline every scenario deviates from.
 */
const HONEST: Record<BenchCheckId, CheckOutcome> = {
  integrity: "pass",
  "issuer-descends-from-factory": "pass",
  "document-names-issuing-contract": "pass",
  "registry-governs-issuer": "pass",
  "issuer-whitelisted": "pass",
  "whitelisted-at-issuance": "pass",
  "anchored-on-chain": "pass",
  "not-revoked": "pass",
  "not-expired": "pass",
};

/** The vector every on-chain row collapses to when no clone resolves - nothing was asked of anything. */
const NO_CLONE: Partial<Record<BenchCheckId, CheckOutcome>> = {
  "issuer-descends-from-factory": "fail",
  "document-names-issuing-contract": "could-not-run",
  "registry-governs-issuer": "could-not-run",
  "issuer-whitelisted": "could-not-run",
  "whitelisted-at-issuance": "could-not-run",
  "anchored-on-chain": "could-not-run",
  "not-revoked": "could-not-run",
};

const expect = (
  overrides: Partial<Record<BenchCheckId, CheckOutcome>>,
): Record<BenchCheckId, CheckOutcome> => ({ ...HONEST, ...overrides });

/**
 * A covered leaf says something other than what was issued. The Merkle recompute refuses it.
 *
 * What does NOT change is the point: `signature.merkleRoot` still names the genuinely anchored root,
 * so every on-chain row stays green. A surface that reported only the chain's answer would call this
 * credential valid.
 */
export const tamperedCoveredField: BenchScenario = {
  id: "tampered-covered-field",
  title: "A covered field was edited after issuance",
  tells: "One of the credential's actual claims differs from what the issuer signed.",
  mustVerify: false,
  refusedBy: ["integrity"],
  blindSpots: [
    {
      id: "anchored-on-chain",
      why: "`signature.merkleRoot` is untouched, so the chain is asked about the genuine root and still reports it anchored.",
    },
    {
      id: "not-revoked",
      why: "Same reason - the revocation read is keyed on the unchanged claimed root.",
    },
    {
      id: "issuer-whitelisted",
      why: "The issuing signer and record type are unchanged; only the document's contents moved.",
    },
  ],
  expected: expect({ integrity: "fail" }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc();
    const script = honestChain(doc.signature.merkleRoot);
    const d = asRecord(doc);
    const data = d.data as Record<string, Record<string, string>>;
    const subject = data.credentialSubject;
    if (!subject || typeof subject.name !== "string") {
      throw new Error("scenario fixture lost its tamperable leaf");
    }
    // Keep the salt and type tag, change ONLY the value - so the resulting integrity failure is
    // attributable to the claim rather than to a leaf that no longer parses.
    const [saltHex, tag] = subject.name.split(":");
    subject.name = `${saltHex}:${tag}:Fido`;
    return world(d, script);
  },
};

/**
 * The document nominates a contract of the attacker's choosing, which vouches for the root.
 *
 * This is the sharp version of every relabelling attack, and the reason the factory anchor exists: the
 * hostile contract answers `isValid = true` and names a genuinely whitelisted signer, so ASKING IT
 * would pass. It is never asked. `rootIssuer` is written only from inside a clone's `issue()` under
 * `require(isClone[msg.sender])` and is strictly write-once, so a contract the factory never deployed
 * can never appear there.
 */
export const hostileIssuerContract: BenchScenario = {
  id: "hostile-issuer-contract",
  title: "A contract the attacker controls vouches for the record",
  tells:
    "This credential was issued by a contract the attacker deployed, which answers `isValid = true` for it and names a whitelisted signer.",
  mustVerify: false,
  refusedBy: ["issuer-descends-from-factory"],
  blindSpots: [
    {
      id: "integrity",
      why: "The whole `issuer` block sits outside the Merkle root, so naming a different contract does not disturb the recompute.",
    },
  ],
  notes:
    "No read is ever made against the attacker's address, so no evidence line may cite it - the report's `reads` log is the proof of that.",
  expected: expect({ ...NO_CLONE }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc({ ...ISSUER, documentStore: HOSTILE });
    const root = doc.signature.merkleRoot;
    // The attacker's contract is fully populated and fully persuasive; the factory simply has no
    // record of the root, which is the only thing that matters.
    const script = withHostileContract(honestChain(root), root);
    return world(asRecord(doc), { ...script, rootIssuers: {} });
  },
};

/**
 * A genuine credential relabelled as a record type its issuer holds no grant for.
 *
 * `issuer.recordType` sits outside the Merkle root, so this costs the forger nothing. The pillar
 * refuses it because the whitelist question is keyed on the record type THE CLONE declares, never on
 * the document's claim - a signer authorised for one type must not be able to carry a credential
 * relabelled as another.
 */
export const relabelledRecordType: BenchScenario = {
  id: "relabelled-record-type",
  title: "A real credential relabelled as a different record type",
  tells:
    "A vaccination record is presented as a government travel clearance, whose issuer this signer was never whitelisted for.",
  mustVerify: false,
  refusedBy: ["issuer-whitelisted"],
  blindSpots: [
    {
      id: "integrity",
      why: "`issuer.recordType` is outside the Merkle root, so the relabel does not disturb the recompute.",
    },
    {
      id: "issuer-descends-from-factory",
      why: "The root really was anchored by a factory clone; the lie is about what KIND of credential it is.",
    },
    {
      id: "anchored-on-chain",
      why: "The anchoring read is keyed on the root, which is genuine and unchanged.",
    },
    {
      id: "whitelisted-at-issuance",
      why: "It cannot run, and that is correct: the verifier refuses a relabel BEFORE resolving a signer, so there is no signer whose grant history could be read. Reporting the clone's own signer as 'authorised' here would read as excusing the relabel.",
    },
  ],
  notes:
    "The whitelist row's refusal here is not 'this signer is unauthorised' but 'the envelope misrepresents the record type'. Both land on the same row, which is why the row's finding names both possibilities rather than asserting one. Read the scenario precisely: it is 'not whitelisted FOR THE TYPE CLAIMED', not the flat 'never whitelisted'. The flat case is UNREACHABLE for a factory-resolved root - `issue()` is `onlyWhitelisted` and writes `issuedBy[r]` in the same call, and `rootIssuer[r]` is write-once and writable only from inside a clone's `issue()` - so a root the factory names was necessarily anchored by a then-authorised signer. Presenting a scripted chain in which it is not would be testing a state the protocol cannot produce.",
  expected: expect({ "issuer-whitelisted": "fail", "whitelisted-at-issuance": "could-not-run" }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc({ ...ISSUER, recordType: OTHER_RECORD_TYPE });
    return world(asRecord(doc), honestChain(doc.signature.merkleRoot));
  },
};

/**
 * The signer had already been delisted when this root was anchored.
 *
 * On an honest chain this state is UNREACHABLE - `issue()` is `onlyWhitelisted`, so the anchoring
 * could not have happened - which is exactly why a record presenting it is fraudulent. Both whitelist
 * rows refuse it, and they refuse it for different reasons worth keeping apart: the gating row because
 * the signer is not authorised NOW, the historical row because it was not authorised THEN.
 */
export const signerDelistedBeforeIssuance: BenchScenario = {
  id: "signer-delisted-before-issuance",
  title: "The signer was delisted BEFORE the root was anchored",
  tells: "A credential claims an issuance that happened while its signer held no authority.",
  mustVerify: false,
  refusedBy: ["whitelisted-at-issuance", "issuer-whitelisted"],
  blindSpots: [
    {
      id: "integrity",
      why: "The document is internally consistent; the lie is about the authority behind it.",
    },
    {
      id: "anchored-on-chain",
      why: "The chain records that the root was anchored - it does not re-check, at read time, who was authorised to anchor it.",
    },
  ],
  expected: expect({ "issuer-whitelisted": "fail", "whitelisted-at-issuance": "fail" }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc();
    const root = doc.signature.merkleRoot;
    const base = honestChain(root);
    return world(asRecord(doc), {
      ...base,
      grants: {
        [k3(SCENARIO_REGISTRY, RECORD_TYPE_KEY, SIGNER)]: [
          { kind: "whitelisted", ...GRANTED },
          { kind: "delisted", ...DELISTED_BEFORE },
        ],
      },
    });
  },
};

/**
 * The signer was authorised when it anchored this root, and was delisted afterwards. It MUST verify.
 *
 * `DogTagIssuer.sol:82` states the rule in the contract's own words - "delisting is forward-only" -
 * and `adminRevoke` exists precisely because a delist does NOT retroactively invalidate what a signer
 * already anchored. The write path guarantees the historical fact independently: `issue()` is
 * `onlyWhitelisted`, sets `issuedBy[r] = msg.sender` in the same call, and `rootIssuer[r]` is
 * write-once and writable only from inside a clone's `issue()`.
 *
 * THIS SCENARIO WAS THE DEFECT, WRITTEN DOWN. Until the verdict formula moved, the verifier refused
 * this credential: its mandatory pillar read `isWhitelistedFor`, a CURRENT-state getter, so an
 * ordinary key rotation, a retiring vet or a lapsed practice licence rendered every credential that
 * signer ever issued as a forgery, fleet-wide. `expected` was left stating the CORRECT answer and a
 * separate `knownDefect` pinned what the code really did, with a test asserting the two still
 * differed - so the day the defect was fixed this scenario went red and whoever fixed it had to
 * delete the pin rather than let the finding quietly evaporate. That is what happened here.
 *
 * The blast radius was always bounded in the safe direction, which is why nothing had to be revoked:
 * by the write-path argument above, a current-state read on a factory-resolved root could only ever
 * produce false NEGATIVES, never a false pass.
 */
export const signerDelistedAfterIssuance: BenchScenario = {
  id: "signer-delisted-after-issuance",
  title: "The signer was delisted AFTER the root was anchored",
  tells:
    "A genuine credential whose issuing signer has since been delisted - a key rotation, a retirement, a lapsed licence.",
  mustVerify: true,
  refusedBy: [],
  blindSpots: [],
  notes:
    "Both whitelist rows pass, and they are still separate on purpose: the gating one is the VERIFIER's own answer, the advisory one is the bench's INDEPENDENT reconstruction from the same log. Their agreeing here is corroboration, and it is what makes this scenario a regression detector - revert the verifier to a current-state read and the gating row alone turns red beside a green historical one.",
  // Nothing is wrong with this credential, so every check that can run passes and the verdict is
  // `true`. This vector was never edited to match the code; the code moved to match it.
  expected: expect({}),
  expectedVerdict: true,
  build() {
    const doc = genuineDoc();
    const root = doc.signature.merkleRoot;
    const base = honestChain(root);
    return world(asRecord(doc), {
      ...base,
      // Granted long before the anchoring, delisted long after it - in the registry that governs the
      // clone, which is the only log an honest `onlyWhitelisted` issuance could rest on.
      grants: {
        [k3(SCENARIO_REGISTRY, RECORD_TYPE_KEY, SIGNER)]: [
          { kind: "whitelisted", ...GRANTED },
          { kind: "delisted", ...DELISTED_AFTER },
        ],
      },
    });
  },
};

/** A revoked credential presented as live. The chain is the source of truth and says otherwise. */
export const revokedPresentedAsLive: BenchScenario = {
  id: "revoked-presented-as-live",
  title: "A revoked credential presented as live",
  tells: "The issuer revoked this credential on-chain; the holder presents it as current.",
  mustVerify: false,
  refusedBy: ["not-revoked"],
  blindSpots: [
    {
      id: "integrity",
      why: "Revocation happens on-chain and leaves the document untouched, so the recompute still reconciles.",
    },
    {
      id: "issuer-descends-from-factory",
      why: "The credential was genuinely issued - `rootIssuer` still names the clone that anchored it.",
    },
    {
      id: "anchored-on-chain",
      why: "It WAS anchored: `issuedAt` is non-zero. Anchoring and revocation are separate facts, which is why they are separate rows.",
    },
    {
      id: "not-expired",
      why: "The chain has no concept of a validity window, and this document's own window has not lapsed.",
    },
  ],
  expected: expect({ "not-revoked": "fail" }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc();
    const root = doc.signature.merkleRoot;
    const base = honestChain(root);
    return world(asRecord(doc), {
      ...base,
      isValid: { [k(CLONE, root)]: false },
      isRevoked: { [k(CLONE, root)]: true },
    });
  },
};

/**
 * This client is configured with a registry that does not gate the issuing contract.
 *
 * `IssuerRegistry._wl` is a per-contract mapping and `DogTagIssuer.onlyWhitelisted` consults the ONE
 * registry in the clone's own `registry` slot. So the only authority that can answer for this
 * contract's issuances is that one, and both whitelist rows go there - off the clone, never off this
 * client's configuration. Both therefore PASS here, correctly.
 *
 * That is precisely what the scenario is for. It is the live catcher for the sourcing rule: point
 * either row back at `registryAddr` and it goes red, because the configured registry's log is empty
 * and the row would print a definite refusal of a genuine credential - our own misconfiguration
 * rendered as an accusation, the fail-closed mirror of the fail-open bug this surface exists to
 * prevent. `registry-governs-issuer` still reports the mis-pairing, because it IS worth telling an
 * operator about; it just no longer voids anything.
 */
export const foreignRegistry: BenchScenario = {
  id: "foreign-registry",
  title: "The authority this client is configured with does not govern this issuer",
  tells:
    "The client's configured registry is not the one gating the issuing contract - a mis-paired factory/registry deployment.",
  mustVerify: false,
  refusedBy: ["registry-governs-issuer"],
  blindSpots: [
    {
      id: "issuer-whitelisted",
      why: "It PASSES, and correctly: it asks the registry the CLONE names rather than the one this client is configured with, and that registry's log holds the grant. Asking the configured one would find nothing and refuse a genuine credential over our own misconfiguration.",
    },
    {
      id: "integrity",
      why: "Nothing about the document changed; the fault is in which contract this client is configured with.",
    },
    {
      id: "issuer-descends-from-factory",
      why: "The clone really was deployed by the configured factory - it is the REGISTRY axis of the pair that is wrong.",
    },
    {
      id: "whitelisted-at-issuance",
      why: "It PASSES for the same reason and from the same log as the gating row - the bench reads it independently, so the two agreeing here is a corroboration rather than a restatement.",
    },
  ],
  notes:
    "The verdict stays `true`, and that is the honest outcome: the credential is genuine, and this client's own mis-pairing is not evidence about it. The row that objects is ADVISORY by design and says which axis of the pair is wrong. Both whitelist rows source their authority from the clone, so both are unaffected - which is the property this scenario exists to hold in place: point either at `registryAddr` and it goes red here.",
  expected: expect({ "registry-governs-issuer": "fail" }),
  // The credential itself is fine and the row that objects is ADVISORY, so the verifier's verdict
  // correctly stays `true`. Refusing it would convert this client's own misconfiguration into an
  // accusation about a credential.
  expectedVerdict: true,
  build() {
    const doc = genuineDoc();
    const root = doc.signature.merkleRoot;
    const base = honestChain(root);
    return world(asRecord(doc), {
      ...base,
      // The clone is gated by a registry this client is not configured with.
      issuerRegistries: { [CLONE.toLowerCase()]: FOREIGN_REGISTRY },
      // The grant sits in the GOVERNING registry, and it has to: `issue()` is `onlyWhitelisted`
      // against the clone's own `registry` slot, so the anchoring this document reports could not have
      // happened unless FOREIGN_REGISTRY - not the configured one - recorded the grant. A fixture with
      // the grant only in SCENARIO_REGISTRY would be a chain state the protocol cannot produce, and it
      // is the emptiness of the CONFIGURED registry's log that makes this scenario the catcher.
      grants: {
        [k3(FOREIGN_REGISTRY, RECORD_TYPE_KEY, SIGNER)]: [{ kind: "whitelisted", ...GRANTED }],
      },
    });
  },
};

/**
 * A wholly fabricated credential that is internally perfect. Integrity PASSES.
 *
 * This is the scenario that shows what the offline recompute is and is not worth: the forger built the
 * document properly, so `targetHash`, the recomputed root and `signature.merkleRoot` all agree. Nothing
 * about the document is wrong. It is refused solely because no contract the factory deployed ever
 * anchored that root - the chain, not the maths, is what makes a credential real.
 */
export const unanchoredSelfConsistentForgery: BenchScenario = {
  id: "unanchored-self-consistent-forgery",
  title: "A perfectly-formed credential that was never anchored",
  tells:
    "A credential invented from scratch. Its Merkle maths is flawless and self-consistent; it simply corresponds to nothing on chain.",
  mustVerify: false,
  refusedBy: ["issuer-descends-from-factory"],
  blindSpots: [
    {
      id: "integrity",
      why: "It PASSES. The document really does hash to the root it names - integrity proves internal consistency, never that anyone issued it.",
    },
    {
      id: "not-expired",
      why: "The forger chose a live validity window, and the expiry row reads the date the document states.",
    },
  ],
  notes:
    "This report is INDISTINGUISHABLE from what a genuine credential anchored under a superseded contract generation produces - both resolve `rootIssuer` to the zero address. The bench cannot tell them apart, and the anchor row's wording ('no contract it deployed ever issued this credential') is literally true of THIS factory in both cases. Recorded rather than papered over; the repo's own note on `rootIssuer` anchoring says retiring the previous generation is intended behaviour and must not be diagnosed as a bug.",
  expected: expect({ ...NO_CLONE }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc();
    // An honest chain for a DIFFERENT root: the factory is live and answering, it simply has no record
    // of this one. Handing it an empty index would also test an unreachable factory by accident.
    const other = wrapDocument(
      { credentialSubject: { dogTagId: { tag: TypeTag.Integer, value: "43" } } },
      ISSUER,
      fixedSalts(),
    );
    return world(asRecord(doc), honestChain(other.signature.merkleRoot));
  },
};

/**
 * A record carrying a batch-inclusion proof, claiming membership of an anchored batch root.
 *
 * `checkIntegrity` refuses ANY non-empty `signature.proof` outright rather than folding it. The fold
 * (`merkle.processProof`) is permissive - it trusts an opaque leaf hash, so an internal node folds to
 * the root just as happily as a real leaf - and the DSDP C1 invariant forbids that primitive in the
 * trust path. Doc->batch-root inclusion never shipped, so a record presenting one is claiming a
 * capability the standard does not have.
 */
export const batchInclusionProof: BenchScenario = {
  id: "batch-inclusion-proof",
  title: "A record claiming membership of an anchored batch",
  tells:
    "The document carries an inclusion proof from its own hash up to the anchored root, rather than being the anchored root itself.",
  mustVerify: false,
  refusedBy: ["integrity"],
  blindSpots: [
    {
      id: "anchored-on-chain",
      why: "The root it names IS anchored - that is what makes this shape persuasive. Only the refusal to fold an opaque proof stands between it and a pass.",
    },
    {
      id: "issuer-whitelisted",
      why: "The signer and record type are genuine; the lie is about how this document relates to the root.",
    },
  ],
  notes:
    "The permissive fold this refuses is the DSDP C1 hazard: `processProof` authenticates nothing about the leaf it starts from, so presenting an internal node as a disclosed leaf would fold to the same root.",
  expected: expect({ integrity: "fail" }),
  expectedVerdict: false,
  build() {
    const doc = genuineDoc();
    const script = honestChain(doc.signature.merkleRoot);
    const d = asRecord(doc);
    const signature = d.signature as Record<string, unknown>;
    signature.proof = [`0x${"cd".repeat(32)}`];
    return world(d, script);
  },
};

/**
 * The endpoint is not the chain these contracts live on.
 *
 * The transport guard probes `eth_chainId` before every address-bound request and refuses to send one
 * to a peer reporting the wrong chain, so no read reaches an address. The property worth asserting is
 * what the bench does with that: EVERY on-chain row must report `could-not-run` naming the chain
 * mismatch, and NOT ONE may report `fail`.
 *
 * That distinction is the whole brief. "The factory has no record of this root" is an accusation about
 * a credential; on a wrong-chain endpoint it would be an accusation nobody was in a position to make -
 * the exact collapse this surface exists to prevent, arriving from the environment rather than the
 * document.
 */
export const wrongChainEndpoint: BenchScenario = {
  id: "wrong-chain-endpoint",
  title: "The endpoint is a different chain",
  tells:
    "A genuine credential checked against an RPC endpoint that is not ROAX - the chain guard refuses to send an address-bound read to it.",
  mustVerify: false,
  refusedBy: [],
  blindSpots: [
    {
      id: "integrity",
      why: "It PASSES, correctly: the recompute is pure offline maths and needs no chain at all.",
    },
    {
      id: "not-expired",
      why: "Also offline - the validity window is a covered leaf, compared against today's date.",
    },
  ],
  notes:
    "Nothing here is a `fail`, and that IS the assertion. The verdict is `null` rather than `false`: the verifier fails closed and produces no answer, and an absent verdict must never be rendered as a refusal.",
  expected: expect({
    "issuer-descends-from-factory": "could-not-run",
    "document-names-issuing-contract": "could-not-run",
    "registry-governs-issuer": "could-not-run",
    "issuer-whitelisted": "could-not-run",
    "whitelisted-at-issuance": "could-not-run",
    "anchored-on-chain": "could-not-run",
    "not-revoked": "could-not-run",
  }),
  // NOT `false`. The verifier fails closed and produces no verdict at all; rendering an absent
  // verdict as a refusal is the collapse this whole surface exists to prevent.
  expectedVerdict: null,
  build() {
    const doc = genuineDoc();
    return world(asRecord(doc), {
      ...honestChain(doc.signature.merkleRoot),
      allReadsFail:
        "The endpoint reports chain 1; DogTag's bundled contracts are for chain 135. The bundled endpoint also could not establish ROAX chain 135; no blockchain read was sent.",
    });
  },
};

/** The honest control. Without it the catalogue cannot tell a strict verifier from a broken one. */
export const genuineCredential: BenchScenario = {
  id: "genuine-credential",
  title: "A genuine credential (control)",
  tells: "Nothing is wrong with this record. Every check that can run must pass.",
  mustVerify: true,
  refusedBy: [],
  blindSpots: [],
  notes:
    "A suite of nothing but frauds would go green against a verifier that refuses everything. This is the case that makes the other results mean something.",
  expected: expect({}),
  expectedVerdict: true,
  build() {
    const doc = genuineDoc();
    return world(asRecord(doc), honestChain(doc.signature.merkleRoot));
  },
};

/** The catalogue, in the order the page renders it. */
export const BENCH_SCENARIOS: BenchScenario[] = [
  genuineCredential,
  tamperedCoveredField,
  hostileIssuerContract,
  relabelledRecordType,
  signerDelistedBeforeIssuance,
  signerDelistedAfterIssuance,
  revokedPresentedAsLive,
  foreignRegistry,
  unanchoredSelfConsistentForgery,
  batchInclusionProof,
  wrongChainEndpoint,
];

/** Run one scenario through the REAL bench - which runs the REAL verifier. No network. */
export async function runBenchScenario(scenario: BenchScenario): Promise<BenchReport> {
  return runVerificationBench(scenario.build());
}
