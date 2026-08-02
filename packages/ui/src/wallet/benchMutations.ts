import { flattenData, parsePacked } from "@dogtag/standard/wrap";
import { validUntilOf, type BenchCheckId } from "./verificationBench";

/**
 * The adversarial half of the bench: take a record and tell a specific lie with it, then watch which
 * check objects.
 *
 * # Why each mutation declares what will NOT catch it
 *
 * A mutation that trips a red row teaches an operator one thing. A mutation paired with an honest list
 * of the checks it walks straight past teaches them the shape of the guarantee - which is the thing
 * they actually need in order to judge a claim themselves. {@link BenchMutation.notCaughtBy} is
 * therefore not a caveat bolted on afterwards; it is half the content, and for
 * {@link relabelIssuerName} it is ALL of it.
 *
 * # Why `caughtBy` is a claim about DESIGN, not a prediction of the run
 *
 * These lists say which checks are DESIGNED to catch a lie. They are not a forecast of what a
 * particular run will show: a record that was already revoked stays revoked under every mutation, and a
 * chain that cannot be reached leaves rows unable to run regardless. The bench always renders the
 * checks' ACTUAL outcomes; these lists sit beside them as the stated expectation, and a divergence
 * between the two is exactly the thing worth looking at rather than something to paper over.
 */

/** A check this mutation is expected to slip past, and why it does. */
export interface UncaughtBy {
  id: BenchCheckId;
  why: string;
}

export interface BenchMutation {
  id: string;
  /** Short imperative label for the button. */
  label: string;
  /** The lie this tells, in plain language. */
  tells: string;
  /** Checks DESIGNED to catch this lie. Empty is a legitimate and important answer. */
  caughtBy: BenchCheckId[];
  /** Checks this lie is expected to walk straight past, each with the reason it does. */
  notCaughtBy: UncaughtBy[];
  /**
   * Checks OUTSIDE this bench's code path that do catch it. Named so an empty `caughtBy` is never
   * mistaken for "nothing anywhere catches this" - a different and much stronger claim.
   */
  caughtElsewhere?: string;
  /**
   * Apply the mutation. Returns `null` when this record cannot carry the lie (for example a document
   * with no validity window cannot have one forged by editing). Refusing is honest; inventing a field
   * so the button always works would make the demonstration a fiction.
   */
  apply(doc: Record<string, unknown>): { doc: Record<string, unknown>; note: string } | null;
}

function clone(doc: Record<string, unknown>): Record<string, unknown> {
  return JSON.parse(JSON.stringify(doc));
}

function issuerBlock(doc: Record<string, unknown>): Record<string, unknown> | null {
  const i = doc.issuer;
  return typeof i === "object" && i !== null && !Array.isArray(i)
    ? (i as Record<string, unknown>)
    : null;
}

/** Set a nested `data` keyPath's packed value, PRESERVING its salt and type tag. */
function setPackedValue(
  doc: Record<string, unknown>,
  keyPath: string,
  newValue: string,
): Record<string, unknown> | null {
  const parts = keyPath.split(".");
  let node = doc.data;
  for (const p of parts.slice(0, -1)) {
    if (typeof node !== "object" || node === null) return null;
    node = (node as Record<string, unknown>)[p];
  }
  if (typeof node !== "object" || node === null) return null;
  const leafKey = parts[parts.length - 1];
  if (leafKey === undefined) return null;
  const packed = (node as Record<string, unknown>)[leafKey];
  if (typeof packed !== "string") return null;
  let saltHex: string;
  let tag: number;
  try {
    ({ saltHex, tag } = parsePacked(packed));
  } catch {
    return null;
  }
  // Keep salt + tag so ONLY the value changes: that is what makes the resulting integrity failure
  // attributable to the value rather than to a malformed leaf.
  (node as Record<string, unknown>)[leafKey] = `${saltHex}:${tag}:${newValue}`;
  return doc;
}

/**
 * The keyPath of this document's validity window, if it carries one.
 *
 * Delegates to the expiry check's OWN resolver rather than re-walking the canonical keyPath chain with
 * a second selection rule. Two rules over one shared list can still diverge - a present-but-blank tier
 * is one the check falls through and a presence-only scan would stop at - and a mutation that edited a
 * different leaf than the row reads would demonstrate nothing.
 */
function validUntilKeyPath(doc: Record<string, unknown>): string | null {
  return validUntilOf(doc)?.keyPath ?? null;
}

/** The first covered leaf that is safe to tamper with - never the non-obfuscatable `dogTagId`. */
function firstTamperableLeaf(doc: Record<string, unknown>): string | null {
  const flat = flattenData(doc.data);
  const entry = flat.find(([kp]) => kp !== "credentialSubject.dogTagId");
  return entry ? entry[0] : null;
}

/**
 * Rewrite the issuer's display name. Nothing in this bench's path objects.
 *
 * `issuer.name` sits OUTSIDE the Merkle root, so integrity still reconciles; the on-chain reads are all
 * keyed on the ROOT and the resolved contract, and none of them ever looks at a display string. This is
 * the mutation whose honest answer is "no row here goes red", and reporting that plainly is worth more
 * than any green tick on this page.
 */
export const relabelIssuerName: BenchMutation = {
  id: "relabel-issuer-name",
  label: "Relabel the issuer's name",
  tells: "The credential was issued by an authority with a different, more impressive name.",
  caughtBy: [],
  notCaughtBy: [
    {
      id: "integrity",
      why: "The whole `issuer` block sits outside the Merkle root, so rewriting the name does not disturb it.",
    },
    {
      id: "issuer-descends-from-factory",
      why: "The factory is asked about the ROOT, not about any name the document prints.",
    },
    {
      id: "document-names-issuing-contract",
      why: "That check compares contract ADDRESSES; the display name is not one.",
    },
    {
      id: "issuer-whitelisted",
      why: "The whitelist is keyed on the signer and the record type the contract itself declares - never on a name.",
    },
  ],
  caughtElsewhere:
    "The government verify route reads the clone's own `name()` - written by the factory's KYC-gated `createIssuer` - and reports a `nameConflict` against the document's claim. That check is server-side and is not part of this browser path.",
  apply(doc) {
    const d = clone(doc);
    const issuer = issuerBlock(d);
    if (!issuer) return null;
    const was = typeof issuer.name === "string" ? issuer.name : "(absent)";
    issuer.name = "National Veterinary Authority";
    return { doc: d, note: `issuer.name: "${was}" -> "National Veterinary Authority"` };
  },
};

/**
 * Point the document at a contract of the attacker's choosing.
 *
 * The sharp version of the relabelling attack: deploy a contract that answers `isValid = true` for any
 * root and name it in `issuer.documentStore`. It fails because every verdict-deciding read is made
 * against the clone the FACTORY named for this root, not against the address the document supplies.
 */
export const repointDocumentStore: BenchMutation = {
  id: "repoint-document-store",
  label: "Point it at a different issuer contract",
  tells: "This credential was issued by a contract the attacker controls, which vouches for it.",
  caughtBy: ["document-names-issuing-contract"],
  notCaughtBy: [
    {
      id: "integrity",
      why: "`issuer.documentStore` is outside the Merkle root, so the document still hashes to the root it claims.",
    },
    {
      id: "issuer-descends-from-factory",
      why: "That check asks the factory about the ROOT and is unaffected by which contract the document names - which is exactly why the attack fails: the reads never go to the attacker's address.",
    },
    {
      id: "anchored-on-chain",
      why: "The anchoring read is made against the factory-resolved contract, so the substituted address never gets to answer.",
    },
  ],
  apply(doc) {
    const d = clone(doc);
    const issuer = issuerBlock(d);
    if (!issuer) return null;
    const was = typeof issuer.documentStore === "string" ? issuer.documentStore : "(absent)";
    const hostile = "0x000000000000000000000000000000000000ba0b";
    issuer.documentStore = hostile;
    return { doc: d, note: `issuer.documentStore: ${was} -> ${hostile}` };
  },
};

/**
 * Change a field the Merkle root covers. The integrity recompute is what refuses it.
 *
 * Note what does NOT change: `signature.merkleRoot` still names the genuinely anchored root, so every
 * on-chain read still passes. A surface that reported only the chain's answer would call this valid.
 */
export const tamperCoveredField: BenchMutation = {
  id: "tamper-covered-field",
  label: "Tamper with a covered field",
  tells: "One of the credential's actual claims says something different than when it was issued.",
  caughtBy: ["integrity"],
  notCaughtBy: [
    {
      id: "anchored-on-chain",
      why: "`signature.merkleRoot` is untouched, so the chain is asked about the genuine root and still reports it anchored.",
    },
    {
      id: "not-revoked",
      why: "Same reason: the revocation read is keyed on the unchanged claimed root.",
    },
    {
      id: "issuer-whitelisted",
      why: "The issuing signer and record type are unchanged; only the document's contents moved.",
    },
  ],
  apply(doc) {
    const d = clone(doc);
    const kp = firstTamperableLeaf(d);
    if (!kp) return null;
    const before = flattenData(d.data).find(([k]) => k === kp)?.[1] ?? "";
    let was = "";
    try {
      was = parsePacked(before).valueRest;
    } catch {
      return null;
    }
    const after = `${was}-TAMPERED`;
    return setPackedValue(d, kp, after) ? { doc: d, note: `data.${kp}: "${was}" -> "${after}"` } : null;
  },
};

/**
 * Push the validity window into the past, so the record reads as lapsed.
 *
 * This is the "present an expired record" case. It is worth running for what the OTHER rows do: nothing
 * on-chain moves, because the chain records anchoring and revocation but has no concept of a validity
 * window. An expired credential is on-chain-valid, and only the offline expiry row says otherwise.
 *
 * Integrity also goes red, and that is not a flaw in the demonstration - it is the point. `validUntil`
 * is covered by the Merkle root, so a window cannot be moved in either direction without breaking the
 * anchor. Both rows firing is the honest picture.
 */
export const expireValidityWindow: BenchMutation = {
  id: "expire-validity-window",
  label: "Present it as an expired record",
  tells: "The credential's validity window has already lapsed.",
  caughtBy: ["not-expired", "integrity"],
  notCaughtBy: [
    {
      id: "anchored-on-chain",
      why: "The chain records anchoring, not validity windows - an expired credential is still anchored.",
    },
    {
      id: "not-revoked",
      why: "Expiry is not revocation: the issuer never revoked this root, so the chain rightly says so.",
    },
    {
      id: "issuer-whitelisted",
      why: "The issuer's authority to have issued it is unaffected by the window having since lapsed.",
    },
  ],
  apply(doc) {
    const d = clone(doc);
    const kp = validUntilKeyPath(d);
    if (!kp) return null;
    const past = "2000-01-01";
    return setPackedValue(d, kp, past) ? { doc: d, note: `data.${kp} -> ${past} (already lapsed)` } : null;
  },
};

/**
 * Push the validity window into the future, so a lapsed record reads as current.
 *
 * The sharper half of the pair above, and the one that shows why the expiry row cannot stand alone: the
 * expiry check reads the date the document gives it and would report a forged 2099 window as perfectly
 * valid. Integrity is the check that actually protects the window, because the field is inside the root.
 */
export const extendValidityWindow: BenchMutation = {
  id: "extend-validity-window",
  label: "Forge a longer validity window",
  tells: "A lapsed credential is still in date.",
  caughtBy: ["integrity"],
  notCaughtBy: [
    {
      id: "not-expired",
      why: "The expiry check compares today against the date the DOCUMENT states, so a forged future date satisfies it. It is integrity, not this row, that protects the window.",
    },
    {
      id: "anchored-on-chain",
      why: "The claimed root is unchanged, so the chain still reports the genuine root anchored.",
    },
    {
      id: "issuer-whitelisted",
      why: "The issuing signer and record type are untouched.",
    },
  ],
  apply(doc) {
    const d = clone(doc);
    const kp = validUntilKeyPath(d);
    if (!kp) return null;
    const future = "2099-12-31";
    return setPackedValue(d, kp, future) ? { doc: d, note: `data.${kp} -> ${future} (forged)` } : null;
  },
};

/** Every mutation the bench offers, in the order the panel renders them. */
export const BENCH_MUTATIONS: BenchMutation[] = [
  relabelIssuerName,
  repointDocumentStore,
  tamperCoveredField,
  expireValidityWindow,
  extendValidityWindow,
];
