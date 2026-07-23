// CONSENT RECEIPTS - the owner's own consent history, derived from what the owner ALREADY holds
// plus the public owner-blind chain events. Two inputs, nothing else:
//
//   1. Held credentials supply the owner's tag ids: `credentialSubject.dogTagId` is the off-chain
//      decimal handle; its canonical on-chain form is the SDK field hash
//      `fieldOfValue(Integer(handle))` - the exact value `mintCustodial(id, R)` sealed and the
//      consent circuit exposes as `pub[0]` (crates/dogtag-standard-rs consent_assemble §"The one
//      canonical dogTagId field").
//   2. `VerificationRegistryConsent.Verified` logs, filtered by those ids. The event is owner-blind
//      (no subject); only the holder, who knows their own tag ids, can assemble this view. The
//      lookup is a read-only browser→RPC call - no backend, no server-side owner state.
//
// Every receipt is a POINT-IN-TIME grant: the proof was approved once on the owner's device, its
// nullifier is consumed, and the on-chain record is permanent history. There is deliberately no
// "revoke" concept anywhere on this surface - an elapsed window is displayed as closed, not as
// something to cancel (captain decision 2026-07-21: consent is never revocable).
import { keccak256, toBytes } from "viem";
import { FIELD_P, TypeTag, fieldOfValue, toHex32 } from "@dogtag/standard";
import { fetchConsentRecordType, fetchVerifiedLogs } from "./chain";
import { summarize } from "./credential";
import type { StoredCredential } from "./store";

/** `keccak256(label) % r` - the BN254 representation of a purpose/recordType label carried by the
 *  consent proof (mirrors `stacks/vet/api/src/verify.rs::purpose_key`). */
export function labelField(label: string): bigint {
  return BigInt(keccak256(toBytes(label))) % FIELD_P;
}

/** The canonical on-chain `dogTagId` for a credential's decimal handle, or null when the leaf is
 *  not a canonical integer handle (nothing on-chain can match it then). */
export function dogTagIdField(handle: string): bigint | null {
  if (!/^(0|[1-9][0-9]*)$/.test(handle)) return null;
  return fieldOfValue({ tag: TypeTag.Integer, value: handle });
}

// The purpose/recordType labels in circulation across the portals (vet, groomer, government, demo
// data). The chain only carries the field hash; an unknown hash renders as its hex, never hidden.
const PURPOSE_LABELS = [
  "boarding_intake",
  "travel_check",
  "grooming_intake",
  "daycare_access",
  "service_animal",
];
const RECORD_TYPE_LABELS = [
  "VACCINATION",
  "DOG_PROFILE",
  "TRAVEL_CLEARANCE",
  "EU_HEALTH_CERT",
  "GROOMING_INTAKE",
  "SERVICE_ATTESTATION",
  "TITER",
  "HealthAttestation",
];

function reverseMap(labels: string[]): Map<string, string> {
  return new Map(labels.map((l) => [toHex32(labelField(l)), l]));
}
const PURPOSE_BY_FIELD = reverseMap(PURPOSE_LABELS);
const RECORD_TYPE_BY_FIELD = reverseMap(RECORD_TYPE_LABELS);

/** "boarding_intake" → "Boarding intake"; ALL_CAPS/CamelCase labels pass through unchanged. */
export function humanizePurpose(label: string): string {
  if (!label.includes("_") || label !== label.toLowerCase()) return label;
  const spaced = label.replace(/_/g, " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** One rendered consent receipt - a single owner-blind `Verified` log attributed to a held tag. */
export interface ConsentReceipt {
  /** The one-time consent nullifier - globally unique (the registry consumes it), the detail key. */
  nullifier: string;
  /** Canonical on-chain tag id (0x…32-byte field). */
  dogTagIdHex: string;
  /** The off-chain decimal handle the held credential discloses. */
  handle: string;
  /** Pet name from the held credential(s) for this tag, when disclosed. */
  petName: string | null;
  /** The relayer (verifier business signer) the consent named - who the owner consented to. */
  relayer: `0x${string}`;
  purposeHex: string;
  /** Known purpose label, or null → render the hex. */
  purposeLabel: string | null;
  /** recordType public signal recovered from calldata; null when unreadable. */
  recordTypeHex: string | null;
  recordTypeLabel: string | null;
  /** Consent window end (Unix seconds) - the latest the signed grant could be submitted. */
  deadline: number;
  /** When the verification was recorded on-chain (Unix seconds, the event's `ts`). */
  grantedAt: number;
  blockNumber: number;
  txHash: string;
}

/** True while the receipt's consent window is still open (`now` defaults to the wall clock).
 *  Display metadata about a past grant, NOT a live permission - the proof itself is already spent. */
export function windowOpen(receipt: Pick<ConsentReceipt, "deadline">, nowMs = Date.now()): boolean {
  return Math.floor(nowMs / 1000) <= receipt.deadline;
}

interface HeldTag {
  handle: string;
  field: bigint;
  petName: string | null;
}

/** Distinct tag ids across the held credentials (a wallet may hold several credentials per tag). */
function heldTags(credentials: StoredCredential[]): HeldTag[] {
  const byHandle = new Map<string, HeldTag>();
  for (const c of credentials) {
    const s = summarize(c.wrappedDoc);
    if (!s.dogTagId) continue;
    const field = dogTagIdField(s.dogTagId);
    if (field === null) continue;
    const existing = byHandle.get(s.dogTagId);
    if (existing) {
      if (!existing.petName && s.petName) existing.petName = s.petName;
    } else {
      byHandle.set(s.dogTagId, { handle: s.dogTagId, field, petName: s.petName });
    }
  }
  return [...byHandle.values()];
}

/**
 * Assemble the owner's consent history: one receipt per `Verified` log on the owner's held tags,
 * newest first. recordType is recovered per-log from the verifying tx's calldata; a single
 * unreadable tx degrades that one receipt's recordType to null rather than failing the history.
 */
export async function loadConsentReceipts(credentials: StoredCredential[]): Promise<ConsentReceipt[]> {
  const tags = heldTags(credentials);
  if (tags.length === 0) return [];
  const byField = new Map(tags.map((t) => [toHex32(t.field), t]));

  const logs = await fetchVerifiedLogs(tags.map((t) => t.field));
  const recordTypes = await Promise.all(logs.map((l) => fetchConsentRecordType(l.txHash)));

  const receipts = logs.map((log, i): ConsentReceipt => {
    const dogTagIdHex = toHex32(log.dogTagId);
    const tag = byField.get(dogTagIdHex);
    const purposeHex = log.purpose.toLowerCase();
    const recordTypeField = recordTypes[i] ?? null;
    const recordTypeHex = recordTypeField === null ? null : toHex32(recordTypeField);
    return {
      nullifier: log.nullifier.toLowerCase(),
      dogTagIdHex,
      handle: tag?.handle ?? "",
      petName: tag?.petName ?? null,
      relayer: log.relayer,
      purposeHex,
      purposeLabel: PURPOSE_BY_FIELD.get(purposeHex) ?? null,
      recordTypeHex,
      recordTypeLabel: recordTypeHex === null ? null : (RECORD_TYPE_BY_FIELD.get(recordTypeHex) ?? null),
      deadline: Number(log.deadline),
      grantedAt: Number(log.ts),
      blockNumber: Number(log.blockNumber),
      txHash: log.txHash,
    };
  });
  return receipts.sort((a, b) => b.grantedAt - a.grantedAt || b.blockNumber - a.blockNumber);
}
