// Reading a held credential — decode the salted, type-tagged leaves of a WrappedDoc back into
// human-readable fields, and derive the summary the wallet UI shows (pet, type, issuer, validity).
//
// A WrappedDoc's `data` is a nested map of `keyPath -> "saltHex:tag:value"` packed leaves. We use the
// SAME parsing the SDK's verify/obfuscate use (`flattenData` + `parsePacked` + `scalarFromPacked`) so
// what the owner sees is exactly what was hashed into the on-chain Merkle root.
import {
  checkIntegrity,
  flattenData,
  parsePacked,
  scalarFromPacked,
  TypeTag,
  type FragmentState,
  type WrappedDoc,
} from "@dogtag/standard";

export type Category = "Health" | "Identity" | "Travel" | "Service" | "Other";

export interface DecodedField {
  keyPath: string;
  label: string;
  value: string;
}

export interface CredentialSummary {
  dogTagId: string | null;
  petName: string | null;
  recordType: string;
  issuerName: string;
  issuerDomain: string;
  documentStore: string;
  category: Category;
  validUntil: string | null;
  validFrom: string | null;
  credentialRoot: string;
  integrity: FragmentState;
}

/** Decode one packed leaf to a display string; bytes are shown as their 0x hex. */
function decodePacked(packed: string): string {
  const { tag, valueRest } = parsePacked(packed);
  const scalar = scalarFromPacked(tag, valueRest);
  if (scalar.tag === TypeTag.Null) return "—";
  if (scalar.tag === TypeTag.Bytes) {
    let h = "0x";
    for (const b of scalar.value) h += b.toString(16).padStart(2, "0");
    return h;
  }
  return String(scalar.value);
}

/** camelCase / dotted keyPath leaf -> "Title Case" label (credentialSubject.lotNumber -> "Lot number"). */
export function labelFor(keyPath: string): string {
  const leaf = keyPath.split(".").pop() ?? keyPath;
  const spaced = leaf
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_]+/g, " ")
    .trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** Every disclosed field of the credential, in document order, decoded for display. */
export function decodeFields(doc: WrappedDoc): DecodedField[] {
  return flattenData(doc.data).map(([keyPath, packed]) => ({
    keyPath,
    label: labelFor(keyPath),
    value: decodePacked(packed),
  }));
}

function fieldValue(doc: WrappedDoc, ...keyPaths: string[]): string | null {
  const flat = new Map(flattenData(doc.data));
  for (const kp of keyPaths) {
    const packed = flat.get(kp);
    if (packed !== undefined) return decodePacked(packed);
  }
  return null;
}

function categoryOf(recordType: string): Category {
  const t = recordType.toUpperCase();
  if (t.includes("VACCIN") || t.includes("HEALTH") || t.includes("RABIES")) return "Health";
  if (t.includes("PROFILE") || t.includes("IDENTITY") || t.includes("PASSPORT")) return "Identity";
  if (t.includes("TRAVEL") || t.includes("CLEARANCE") || t.includes("MOVEMENT")) return "Travel";
  if (t.includes("GROOM") || t.includes("SERVICE") || t.includes("ATTEST")) return "Service";
  return "Other";
}

/** The at-a-glance summary the wallet card + detail header render. */
export function summarize(doc: WrappedDoc): CredentialSummary {
  const recordType = doc.issuer.recordType || fieldValue(doc, "recordType") || "CREDENTIAL";
  return {
    dogTagId: fieldValue(doc, "credentialSubject.dogTagId", "dogTagId"),
    petName: fieldValue(doc, "credentialSubject.name", "credentialSubject.petName", "pet.name"),
    recordType,
    issuerName: doc.issuer.name || "Unknown issuer",
    issuerDomain: doc.issuer.domain || "",
    documentStore: doc.issuer.documentStore || "",
    category: categoryOf(recordType),
    validUntil: fieldValue(
      doc,
      "credentialSubject.validUntil",
      "credentialSubject.rabiesValidUntil",
      "credentialSubject.expiresOn",
      "validUntil",
    ),
    validFrom: fieldValue(
      doc,
      "credentialSubject.validFrom",
      "credentialSubject.administeredOn",
      "credentialSubject.examinationDate",
      "validFrom",
    ),
    credentialRoot: doc.signature.merkleRoot,
    integrity: checkIntegrity(doc).state,
  };
}

/** Best-effort parse of pasted text into a WrappedDoc, with a helpful error on malformed input. */
export function parseWrappedDoc(text: string): WrappedDoc {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("Not valid JSON. Paste the wrapped credential document an issuer gave you.");
  }
  const doc = parsed as Partial<WrappedDoc>;
  if (!doc || typeof doc !== "object" || !doc.signature || !doc.issuer || doc.data === undefined) {
    throw new Error("This does not look like a DogTag credential (missing data/signature/issuer).");
  }
  if (doc.signature.type !== "DogTagMerkleProof" || !doc.signature.merkleRoot) {
    throw new Error("Missing a DogTagMerkleProof signature with a merkleRoot.");
  }
  return doc as WrappedDoc;
}
