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

export interface ShareField extends DecodedField {
  /** true when this leaf may not be withheld (it must remain present for the credential to verify). */
  locked: boolean;
}

/** Every field of the credential the holder may reveal or withhold, with dogTagId locked-on. */
export function shareableFields(doc: WrappedDoc): ShareField[] {
  return decodeFields(doc).map((f) => ({ ...f, locked: NON_OBFUSCATABLE_PATHS.includes(f.keyPath) }));
}

/**
 * Produce a redacted copy of the credential with the named leaves cryptographically withheld. The
 * Merkle root is unchanged, so the copy still verifies as the same authentic credential — the recipient
 * simply cannot read the withheld values. Non-obfuscatable leaves (dogTagId) are never withheld, even
 * if passed. Withholding nothing returns the original document untouched.
 */
export function redact(doc: WrappedDoc, withheld: Iterable<string>): WrappedDoc {
  const paths = [...withheld].filter((kp) => !NON_OBFUSCATABLE_PATHS.includes(kp));
  return paths.length === 0 ? doc : obfuscate(doc, paths);
}

/** Recompute the integrity verdict of a (possibly redacted) doc — it stays VALID after redaction. */
export function integrityOf(doc: WrappedDoc): FragmentState {
  return checkIntegrity(doc).state;
}
