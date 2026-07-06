// REDACT — holder-controlled selective disclosure, the Merkle counterpart of the ZK present flow.
//
// A held credential is a salted Merkle tree of typed leaves. `@dogtag/standard`'s `obfuscate` moves a
// chosen leaf's HASH into `privacy.obfuscated[]` and drops its cleartext, leaving the on-chain Merkle
// root R UNCHANGED. So the holder can hand a recipient a copy that reveals only the fields they choose:
// the recipient still recomputes the same root and confirms this is the same, authentic, unmodified
// credential (`checkIntegrity`) and that it is anchored/valid on-chain (`DogTagIssuer.isValid`) — they
// just cannot read the withheld values. This is the SAME primitive the native holder apps use for
// "Share redacted" (the mobile FFI `obfuscateDocumentJson`); the web wallet now offers it too. NO ZK on
// this path — it is the selective-disclosure Merkle proof, distinct from the anonymous ZK presentation.
import { checkIntegrity, obfuscate, type FragmentState, type WrappedDoc } from "@dogtag/standard";
import { decodeFields, type DecodedField } from "./credential";

// Leaves that must stay revealed for the credential to verify at all — mirrors `NON_OBFUSCATABLE` in
// `@dogtag/standard`'s verify (audit-05 V3/V6): dogTagId is required-present, so withholding it would
// make the redacted copy fail its integrity check. The UI locks these on.
export const NON_OBFUSCATABLE_PATHS = ["credentialSubject.dogTagId"];

// Leaves whose value is ALSO carried in cleartext in the always-revealed `issuer` block, so withholding
// them from the Merkle data would be an illusion - the recipient still reads the value from `doc.issuer`.
// `recordType` mirrors `doc.issuer.recordType`, which the recipient needs for the identity pillar and
// which `obfuscate` never touches. The UI marks these public rather than offering a toggle that lies.
export const PUBLIC_PATHS = ["recordType"];

// Every leaf the holder cannot meaningfully withhold - either required-present or public in the issuer.
const LOCKED_PATHS = new Set([...NON_OBFUSCATABLE_PATHS, ...PUBLIC_PATHS]);

/** Why a leaf stays revealed: it must remain present to verify, or it is public in the issuer block. */
export type LockReason = "required" | "public";

export interface ShareField extends DecodedField {
  /** null when the holder may reveal or withhold this leaf; otherwise why it is locked-on. */
  lockReason: LockReason | null;
  /** true when this leaf may not be withheld (it must remain present, or it is inherently public). */
  locked: boolean;
}

/** Every field of the credential the holder may reveal or withhold, with locked leaves flagged. */
export function shareableFields(doc: WrappedDoc): ShareField[] {
  return decodeFields(doc).map((f) => {
    const lockReason: LockReason | null = NON_OBFUSCATABLE_PATHS.includes(f.keyPath)
      ? "required"
      : PUBLIC_PATHS.includes(f.keyPath)
        ? "public"
        : null;
    return { ...f, lockReason, locked: lockReason !== null };
  });
}

/**
 * Produce a redacted copy of the credential with the named leaves cryptographically withheld. The
 * Merkle root is unchanged, so the copy still verifies as the same authentic credential — the recipient
 * simply cannot read the withheld values. Locked leaves (required or public) are never withheld, and any
 * path not actually present in the document is ignored, so `obfuscate` can never fault on a stale path.
 * Withholding nothing returns the original document untouched.
 */
export function redact(doc: WrappedDoc, withheld: Iterable<string>): WrappedDoc {
  const present = new Set(decodeFields(doc).map((f) => f.keyPath));
  const paths = [...withheld].filter((kp) => present.has(kp) && !LOCKED_PATHS.has(kp));
  return paths.length === 0 ? doc : obfuscate(doc, paths);
}

/** Recompute the integrity verdict of a (possibly redacted) doc — it stays VALID after redaction. */
export function integrityOf(doc: WrappedDoc): FragmentState {
  return checkIntegrity(doc).state;
}
