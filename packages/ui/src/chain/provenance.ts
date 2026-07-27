/**
 * On-chain provenance for AUDIT surfaces (government Oversight, vet/groomer Traceability).
 *
 * An audit table's whole value is that a reader can go check. That requires two things this module
 * supplies, and which every portal previously open-coded (or omitted):
 *
 *  1. **Chain-addressability.** A row may only claim an explorer link when its transaction hash is a
 *     well-formed 32-byte value. This is not a guess about "demo mode" - it is arithmetic: an EVM
 *     transaction hash is `keccak256` output, exactly 32 bytes, so `0x0800` cannot address a
 *     transaction on ANY chain. The indexer composes `txUrl` unconditionally from `EXPLORER_BASE`
 *     (`stacks/indexer/api/src/routes.rs`), so in `INDEXER_DEMO_MODE` its scripted `MemLogSource`
 *     events arrive carrying `https://explorer.roax.net/tx/0x0800` - a link to nothing. Gating on the
 *     hash shape means a synthetic row can never render as a real one, and - crucially - a single
 *     synthetic row inside an otherwise-real feed is still caught, which a header-level demo badge
 *     cannot do.
 *  2. **Labelled, copyable identifiers.** Roots, hashed record-type keys, purpose keys and nullifiers
 *     are all bare 32-byte hex. Truncated for density, they are indistinguishable from one another,
 *     so each must be rendered with a word saying WHAT it is and must hand back the full value.
 *
 * Presentation only: nothing here changes what the indexer records or how anything is verified.
 */

import { roax } from "../wallet/chain";

/** An EVM transaction/block hash, or any 32-byte commitment (root, purpose key, nullifier). */
const HASH32_RE = /^0x[0-9a-fA-F]{64}$/;
/** A 20-byte EVM address. */
const ADDRESS_RE = /^0x[0-9a-fA-F]{40}$/;

/** Whether `v` is a well-formed 32-byte hex value (`0x` + 64 hex digits). */
export function isHash32(v?: string | null): boolean {
  return typeof v === "string" && HASH32_RE.test(v.trim());
}

/** Whether `v` is a well-formed 20-byte EVM address (`0x` + 40 hex digits). */
export function isEvmAddress(v?: string | null): boolean {
  return typeof v === "string" && ADDRESS_RE.test(v.trim());
}

/**
 * Whether an event's identifiers can address a real transaction.
 *
 * `onchain`   - the tx hash is a well-formed 32-byte value, so the explorer link means something.
 * `synthetic` - it is not, so this event did not come from a real transaction. Seeded/demo feeds and
 *               half-populated fixtures land here; so would a malformed indexer row, which is a bug
 *               worth showing rather than papering over.
 */
export type ChainProvenance = "onchain" | "synthetic";

/** The provenance of one indexed event, from the shape of the identifiers it carries. */
export function chainProvenance(ev: { txHash?: string | null }): ChainProvenance {
  return isHash32(ev.txHash) ? "onchain" : "synthetic";
}

/**
 * The explorer link for an event's transaction, or `null` when the event is not chain-addressable.
 *
 * Prefers the indexer-supplied `txUrl` (it knows the deployment's `EXPLORER_BASE`, which need not be
 * the ROAX default) and only composes a URL when the API did not send one. Returning `null` rather
 * than a dead link is the point: callers must render synthetic rows as plain text.
 */
export function txExplorerHref(ev: { txHash?: string | null; txUrl?: string | null }): string | null {
  if (chainProvenance(ev) !== "onchain") return null;
  if (ev.txUrl) return ev.txUrl;
  return `${roax.blockExplorers.default.url}/tx/${ev.txHash}`;
}

/** The explorer link for an address, or `null` when it is not a well-formed EVM address. */
export function addressExplorerHref(addr?: string | null): string | null {
  if (!isEvmAddress(addr)) return null;
  return `${roax.blockExplorers.default.url}/address/${addr}`;
}

/**
 * Middle-truncate a long hex value for a dense table cell. The full value always stays reachable via
 * the copy affordance, never only via a `title` tooltip (tooltips do not survive a screenshot, a
 * touch device, or a copy/paste into an incident report).
 */
export function shortHex(v?: string | null, head = 10, tail = 6): string {
  if (!v) return "—";
  const s = v.trim();
  return s.length > head + tail + 1 ? `${s.slice(0, head)}…${s.slice(-tail)}` : s;
}

/**
 * The absolute, unambiguous rendering of an on-chain timestamp - the form an audit surface must
 * always offer. Includes the timezone abbreviation, because "14:03" is not evidence without one.
 */
export function formatChainTime(secs?: number | null): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString(undefined, { timeZoneName: "short" });
}

/** A coarse "2h ago" for scanning. Always shown ALONGSIDE the absolute time, never instead of it. */
export function formatRelativeTime(secs?: number | null, nowMs: number = Date.now()): string {
  if (!secs) return "";
  const deltaSec = Math.round(nowMs / 1000 - secs);
  const ago = deltaSec >= 0;
  const d = Math.abs(deltaSec);
  const unit =
    d < 60
      ? `${d}s`
      : d < 3600
        ? `${Math.floor(d / 60)}m`
        : d < 86400
          ? `${Math.floor(d / 3600)}h`
          : `${Math.floor(d / 86400)}d`;
  return ago ? `${unit} ago` : `in ${unit}`;
}

/** The subset of an indexed event the detail labeller needs (government + vet/groomer feeds alike). */
export interface ChainEventLike {
  type: string;
  recordType?: string | null;
  root?: string | null;
  dogTagId?: string | null;
  purpose?: string | null;
  nullifier?: string | null;
  /** `issuerCreated`: the human name the clone was registered under. */
  name?: string | null;
}

/** One labelled identifier to render for an event. */
export interface ChainDetailField {
  label: string;
  value?: string | null;
}

/**
 * The identifiers an event carries, each with a word saying WHAT it is.
 *
 * Every value here is 32 bytes of hex except the human record-type labels and the decimal dogTagId, so
 * rendering any of them bare - as these tables used to - leaves the reader unable to tell a credential
 * root from a hashed record-type key from a nullifier. `recordType` arrives EITHER as a human label
 * (`TRAVEL_CLEARANCE`) or as its keccak key, depending on whether the indexer's directory could
 * reverse it, so the label follows the shape of the value rather than assuming one.
 */
export function eventDetailFields(ev: ChainEventLike): ChainDetailField[] {
  const recordTypeField: ChainDetailField = {
    label: isHash32(ev.recordType) ? "record type key" : "record type",
    value: ev.recordType,
  };
  switch (ev.type) {
    case "rootIssued":
    case "rootRevoked":
    case "rootRegistered":
      return [recordTypeField, { label: "credential root", value: ev.root }];
    case "issuerCreated":
      return [{ label: "issuer name", value: ev.name }, recordTypeField];
    case "whitelisted":
    case "delisted":
      return [recordTypeField];
    case "verified":
      // Owner-blind payload: an opaque tag id, a hashed purpose key and an unlinkable nullifier.
      // No subject exists downstream, so these are the only identifiers there are.
      return [
        { label: "dog tag id", value: ev.dogTagId },
        { label: "purpose key", value: ev.purpose },
        { label: "nullifier", value: ev.nullifier },
      ];
    default:
      return [recordTypeField, { label: "credential root", value: ev.root }];
  }
}

/**
 * What the contract that EMITTED an event is, given the event kind. The indexer's `contract` is the
 * emitting address and is NOT always the issuer clone: `issuerCreated` comes from the factory and
 * `whitelisted`/`delisted` from the IssuerRegistry, while `verified` comes from the verification
 * registry. "Which smart contract" is unanswerable without this distinction.
 */
export function emittingContractRole(type: string): string {
  switch (type) {
    case "issuerCreated":
      return "issuer factory";
    case "whitelisted":
    case "delisted":
      return "issuer registry";
    case "verified":
      return "verification registry";
    case "rootIssued":
    case "rootRevoked":
    case "rootRegistered":
      return "issuer clone";
    default:
      return "contract";
  }
}
