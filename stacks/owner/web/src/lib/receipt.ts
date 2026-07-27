// RECEIPT — the holder's CDC-grade travel-clearance receipt, derived purely from a held WrappedDoc.
//
// This is the owner-web (holder) counterpart of the government portal's receipt page
// (`stacks/government/web/src/pages/Receipt.tsx`) and the native apps' `TravelReceiptView` — the
// "show this on your phone" surface of the CDC "Dog Import Form Receipt". It renders ENTIRELY from
// the locally-held credential (the decoded, salted Merkle leaves) plus a single live on-chain
// `DogTagIssuer.isValid(root)` read: no backend, no new plumbing (govarch-r8 §3.2). Section A/B/C,
// the validity window and the public `receiptId` are revealed cleartext leaves; a received *redacted*
// copy simply drops the withheld leaves (their hash stays in `privacy.obfuscated[]`, so the copy
// still verifies against the same on-chain root) — the receipt omits those rows and flags the count.
import type { WrappedDoc } from "@dogtag/standard";
import { decodeFields } from "./credential";

/** Record types that render as a formal receipt (vs the generic decoded-field detail view). */
const RECEIPT_TYPES = new Set(["TRAVEL_CLEARANCE", "EU_HEALTH_CERT"]);

/** True when this credential should offer the CDC-grade receipt view. */
export function isReceiptType(recordType: string | null | undefined): boolean {
  return RECEIPT_TYPES.has((recordType || "").toUpperCase());
}

/** The credential's on-chain validity as the owner wallet reads it (mirrors CredentialDetail). */
export type OnChain = "checking" | "valid" | "invalid" | "unknown";

/** The receipt verdict shown in the status banner. */
export type EffectiveStatus = "VALID" | "EXPIRED" | "REVOKED" | "UNCONFIRMED";

export interface Row {
  label: string;
  value: string;
}

export interface ReceiptSection {
  title: string;
  rows: Row[];
}

export interface ReceiptModel {
  isTravel: boolean;
  title: string;
  authorityName: string;
  receiptId: string;
  issuanceDate: string;
  validUntil: string;
  multipleEntries: boolean;
  departureBinding: string;
  dogTagId: string;
  legalBasisVersion: string;
  /** CDC Section A/B/C (travel) or the Annex-IV clinical block (health), blanks dropped. */
  sections: ReceiptSection[];
  /** How many leaves the holder withheld (obfuscated) in this copy; 0 for a full credential. */
  obfuscatedCount: number;
  root: string;
  documentStore: string;
  issuerDomain: string;
  /** Public, PII-free status URL the QR encodes (`https://<issuerDomain>/r/<receiptId>`) or "". */
  publicStatusUrl: string;
}

/** Decode every leaf into a flat `keyPath -> value` map (envelope leaves keep their full path, e.g.
 *  `legalBasisVersion`; subject leaves keep their `credentialSubject.` prefix). */
function decodeAll(doc: WrappedDoc): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of decodeFields(doc)) out[f.keyPath] = f.value;
  return out;
}

/** Humanize a snake_case enum ("service_animal" → "Service Animal"); pass booleans/plain codes through. */
export function humanize(v: string): string {
  if (v === "") return "";
  if (v === "true") return "Yes";
  if (v === "false") return "No";
  if (!v.includes("_")) return v;
  return v
    .split("_")
    .map((w) => (w ? w[0]!.toUpperCase() + w.slice(1) : w))
    .join(" ");
}

/** ISO `YYYY-MM-DD` for "today" (UTC) — used for the derived validity-window lapse check. */
function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}

/** True when the credential's `validity.validUntil` window has lapsed (lexical ISO-date compare). */
function validUntilLapsed(validUntil: string): boolean {
  if (validUntil.length < 10) return false;
  return validUntil.slice(0, 10) < todayIso();
}

/** True when an ISO validity window has already lapsed. Shared by card/detail receipt badges. */
export function isExpired(validUntil: string | null | undefined): boolean {
  return validUntilLapsed(validUntil || "");
}

/**
 * Derive the receipt verdict the way the government portal + native apps do: a live on-chain revoke
 * wins, then a lapsed validity window (EXPIRED), else VALID. When the chain read has not resolved
 * (or errored) we do NOT claim VALID — a lapsed window still shows EXPIRED, otherwise UNCONFIRMED.
 */
export function deriveStatus(onchain: OnChain, validUntil: string): EffectiveStatus {
  if (onchain === "invalid") return "REVOKED";
  if (onchain === "valid") return validUntilLapsed(validUntil) ? "EXPIRED" : "VALID";
  // checking / unknown — the on-chain truth is not yet confirmed.
  return validUntilLapsed(validUntil) ? "EXPIRED" : "UNCONFIRMED";
}

/** A print-safe fixed hex accent for a verdict (the receipt sheet uses a fixed light palette, not
 *  the wallet's theme tokens, so it renders identically on paper). Mirrors the public `/r/:id` page. */
export function statusAccent(status: EffectiveStatus): string {
  switch (status) {
    case "VALID":
      return "#16a34a";
    case "EXPIRED":
      return "#d97706";
    case "REVOKED":
      return "#dc2626";
    default:
      return "#64748b";
  }
}

/** Owner-facing status copy. `isValid(root)==false` can mean revoked or never anchored; for a held
 * credential the product-relevant warning is that it is no longer a valid on-chain receipt. */
export function statusLabel(status: EffectiveStatus): string {
  switch (status) {
    case "VALID":
      return "Valid";
    case "EXPIRED":
      return "Expired";
    case "REVOKED":
      return "Revoked / not anchored";
    default:
      return "Unconfirmed";
  }
}

export function statusBadgeClass(status: EffectiveStatus): "ok" | "warn" | "bad" {
  if (status === "VALID") return "ok";
  if (status === "EXPIRED" || status === "UNCONFIRMED") return "warn";
  return "bad";
}

/** The public, PII-free status URL the QR encodes: `<protocol.statusBaseUrl>/r/<receiptId>`.
 *
 *  `statusBaseUrl` is stamped at issuance from the issuer's `DEPLOYMENT_URL` and is the only field in
 *  the document that names a host a phone can actually reach. This deliberately does NOT fall back to
 *  `issuer.domain`: that is a `did:web` identity, not a deployment - the shipped default `gov.example`
 *  is RFC-2606 reserved and NXDOMAIN, so every QR built from it encoded a dead link that still read as
 *  a legitimate verification affordance.
 *
 *  "" when the base or the receipt id is missing - including every document issued before issuers
 *  began stamping the field. The caller renders no QR and says the issuer published no status page,
 *  which is true, rather than printing a URL that resolves to nothing. */
export function publicStatusUrl(doc: WrappedDoc, receiptId: string): string {
  const base = (doc.protocol?.statusBaseUrl || "").trim().replace(/\/+$/, "");
  if (!base || !receiptId) return "";
  return `${base}/r/${receiptId}`;
}

/** Assemble the full receipt model from a held credential (all values already decoded/humanized).
 * Returns null for unsupported record types so callers cannot accidentally render a generic credential
 * as an official receipt. */
export function buildReceipt(doc: WrappedDoc): ReceiptModel | null {
  const all = decodeAll(doc);
  // Subject leaves are addressed WITHOUT the `credentialSubject.` prefix (mirrors the native app).
  const pick = (path: string): string => all[`credentialSubject.${path}`] ?? "";
  const envelope = (path: string): string => all[path] ?? "";
  const isTrue = (path: string): boolean => {
    const v = pick(path).toLowerCase();
    return v === "true" || v === "1" || v === "yes";
  };

  const recordType = (doc.issuer.recordType || "").toUpperCase();
  const isTravel = recordType === "TRAVEL_CLEARANCE";
  const isHealth = recordType === "EU_HEALTH_CERT";
  if (!isTravel && !isHealth) return null;

  // Section A — person importing the animal (PII; a redacted copy drops these, leaving blanks).
  const sectionA: Row[] = [
    { label: "First Name", value: pick("importer.firstName") },
    { label: "Middle Name/Initial", value: pick("importer.middleName") },
    { label: "Last Name", value: pick("importer.lastName") },
    { label: "The person listed above is the", value: humanize(pick("importer.role")) },
    { label: "Identification Type", value: humanize(pick("importer.idType")) },
    { label: "ID Jurisdiction (state/country)", value: pick("importer.idJurisdiction") },
    { label: "Identification No.", value: pick("importer.idNumber") },
    { label: "Importer Date of Birth", value: pick("importer.dateOfBirth") },
    { label: "Email", value: pick("importer.email") },
    { label: "Phone Number", value: pick("importer.phone") },
    { label: "Consignee/Additional Owner", value: pick("consignee.fullName") },
    { label: "Consignee Email", value: pick("consignee.email") },
    { label: "Consignee Identification Type", value: humanize(pick("consignee.idType")) },
  ];

  // Section B — animal information. CDC's "Male Neutered" = sex + neutered combined into one row.
  const sex = pick("animal.sex");
  const sexValue = sex
    ? `${sex[0]!.toUpperCase()}${sex.slice(1)}${isTrue("animal.neutered") ? " Neutered" : ""}`
    : "";
  const sectionB: Row[] = [
    { label: "Animal Name", value: pick("animal.name") },
    { label: "Age - Year(s)", value: pick("animal.ageYears") },
    { label: "Age - Month(s)", value: pick("animal.ageMonths") },
    { label: "Sex", value: sexValue },
    { label: "Dog Breed", value: pick("animal.breed") },
    { label: "Color/Markings", value: pick("animal.colorMarkings") },
    { label: "Microchip Number", value: pick("animal.microchipNumber") },
    { label: "Importation Purpose", value: humanize(pick("animal.importationPurpose")) },
  ];

  // Section C — travel information.
  const sectionC: Row[] = [
    { label: "Travel Type", value: humanize(pick("travel.travelType")) },
    { label: "Country or Area of Departure", value: pick("travel.countryOfDeparture") },
    { label: "Date of Arrival", value: pick("travel.dateOfArrival") },
    { label: "Port of Entry", value: pick("travel.portOfEntry") },
    { label: "Carrier / Flight", value: pick("travel.carrierOrFlight") },
  ];

  // EU_HEALTH_CERT — the Annex-IV clinical block (flat subject).
  const healthRows: Row[] = [
    { label: "Species", value: pick("species") },
    { label: "Microchip Number", value: pick("microchipNumber") },
    { label: "Rabies Vaccination Date", value: pick("rabiesVaccinationDate") },
    { label: "Rabies Valid Until", value: pick("rabiesValidUntil") },
    { label: "Examining Veterinarian", value: pick("examiningVeterinarian") },
    { label: "Clinical Health Status", value: humanize(pick("clinicalHealthStatus")) },
    { label: "Examination Date", value: pick("examinationDate") },
  ];

  const dropBlank = (rows: Row[]): Row[] => rows.filter((r) => r.value !== "");
  const sections: ReceiptSection[] = isTravel
    ? [
        { title: "Section A - Person Importing the Animal", rows: dropBlank(sectionA) },
        { title: "Section B - Animal Information", rows: dropBlank(sectionB) },
        { title: "Section C - Travel Information", rows: dropBlank(sectionC) },
      ]
    : [{ title: "Health Certificate (Annex IV)", rows: dropBlank(healthRows) }];

  const receiptId = pick("receiptId");
  return {
    isTravel,
    title: isTravel ? "Pet Travel Clearance Receipt" : "Pet Health Certificate",
    authorityName: doc.issuer.name || pick("endorsingAuthority") || "Competent Authority",
    receiptId,
    // Issuance DATE is derived from the `validity.issuedOn`/`validFrom` leaf (the authoritative
    // on-chain `issuedAt[R]` is the canonical source; the holder wallet reads the leaf, offline).
    issuanceDate: pick("validity.issuedOn") || pick("validity.validFrom"),
    validUntil: pick("validity.validUntil") || pick("rabiesValidUntil"),
    multipleEntries: isTrue("validity.multipleEntries"),
    departureBinding: pick("validity.countryOfDepartureBinding"),
    dogTagId: pick("dogTagId"),
    legalBasisVersion: envelope("legalBasisVersion"),
    sections: sections.filter((sec) => sec.rows.length > 0),
    obfuscatedCount: doc.privacy?.obfuscated?.length ?? 0,
    root: doc.signature.merkleRoot,
    documentStore: doc.issuer.documentStore || "",
    issuerDomain: doc.issuer.domain || "",
    publicStatusUrl: publicStatusUrl(doc, receiptId),
  };
}
