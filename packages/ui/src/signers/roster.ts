/**
 * Who may sign in this provider's name — LAYER 2 of the two-layer issuance requirement.
 *
 * `DogTagIssuer.issue` requires BOTH the authority's grant and THIS contract's own
 * `issuanceAllowed[msg.sender]`. Layer 1 has a screen; layer 2 had none, in either direction, which
 * is why a correctly registered provider with a correctly attached contract could still not issue
 * and why the crew walking `docs/DEMO_CLICKS.md` had to send the transaction from a terminal.
 *
 * Pure, and everything a row CLAIMS is decided here rather than in the component — because the
 * claims are the point:
 *
 *  * **A read that failed is not an empty list.** {@link RosterRead} has no `entries` member on its
 *    unavailable arm, so no renderer can spread it into one. A provider deciding who may issue
 *    medical records in their name must never be shown "nobody is admitted" when the truth is "we
 *    could not ask".
 *  * **Withdrawn is not never-admitted.** {@link signerStanding} answers three values, not a
 *    boolean, and each carries a WORD. The task this file answers names exactly this distinction as
 *    previously misread on a neighbouring screen, where it was carried by styling alone and a
 *    flattened text dump lost it. Styling is applied on top; it is never the carrier.
 *  * **Admitting is the owner's alone.** Not a preference — `setIssuanceAllowed` gates the admit
 *    direction on `msg.sender == owner()` and deliberately excludes the protocol admin, which also
 *    writes the authority bit and would otherwise hold both layers at once. {@link admitBlock}
 *    COMPOSES that rule against the owner the chain reported; it does not invent a stricter one.
 */

import { describeActionBlock, type ActionBlock } from "../provider/actionAvailability";

/** A 0x-prefixed hex address, lowercase by the time it reaches this module. */
export type Address = `0x${string}`;

/** One address this contract's list has an opinion about, as the backend read it. */
export interface RosterEntry {
  /** Lowercase 0x-hex, so comparing against a connected wallet cannot miss on EIP-55 casing. */
  address: string;
  /** `issuanceAllowed(address)` from STORAGE — never the log's own word. */
  allowed: boolean;
  /** Whether a mined `IssuanceAllowedSet` has ever named this address. */
  everNamed: boolean;
}

/**
 * The backend's answer for one contract.
 *
 * Mirrors `crate::issuance_allowed::RosterRead` on the wire. The unavailable arm carries no
 * `entries`, by construction rather than by convention.
 */
export type RosterRead =
  | {
      state: "resolved";
      /** The contract's `owner()` — the only address that may ADMIT. */
      owner: string;
      entries: RosterEntry[];
      /**
       * Whether THIS deployment's own custody signer may currently anchor here.
       *
       * `null` only when there is no active signer to ask about (custody locked). Branch on it
       * rather than reading `false` into a locked backend.
       */
      activeSignerAllowed: boolean | null;
    }
  | { state: "unavailable"; reason: string };

/** One anchoring contract this deployment issues through. */
export interface IssuerContract {
  recordType: string;
  issuerAddr: string;
  read: RosterRead;
}

export interface IssuanceAllowedResponse {
  /** `null` when custody is locked, so there is no signer to speak about. */
  activeSigner: string | null;
  contracts: IssuerContract[];
}

/**
 * What a row says about one address.
 *
 * THREE values, never a boolean. `withdrawn` and `neverAdmitted` are both "cannot issue" and they
 * are different facts with different remedies — one had the right and lost it, the other never had
 * it. A screen that spells them the same way is the defect this type exists to prevent.
 */
export type SignerStanding = "admitted" | "withdrawn" | "neverAdmitted";

export function signerStanding(entry: RosterEntry): SignerStanding {
  if (entry.allowed) return "admitted";
  return entry.everNamed ? "withdrawn" : "neverAdmitted";
}

/**
 * The word rendered in the row.
 *
 * These are TEXT, and that is the requirement rather than a styling choice: the distinction has to
 * survive a screen reader, a screenshot, and a flattened text extraction of the page. Colour is
 * applied on top of the word and never instead of it.
 */
const STANDING_LABEL: Readonly<Record<SignerStanding, string>> = {
  admitted: "Can issue",
  withdrawn: "Withdrawn",
  neverAdmitted: "Not admitted",
};

export function signerStandingLabel(standing: SignerStanding): string {
  return STANDING_LABEL[standing];
}

/** The fuller sentence beside the label, so a reader need not know what the word implies. */
const STANDING_DETAIL: Readonly<Record<SignerStanding, string>> = {
  admitted: "On this contract's issuance list. It still also needs the authority's issue right.",
  withdrawn:
    "Was on this contract's issuance list and has been removed. It cannot anchor anything new; what it already anchored stays anchored and stays revocable by it.",
  neverAdmitted: "This contract's issuance list has never held this address.",
};

export function signerStandingDetail(standing: SignerStanding): string {
  return STANDING_DETAIL[standing];
}

/** Tone, applied ON TOP of the word above. Facts render neutral; only a failure warns. */
export function signerStandingTone(standing: SignerStanding): "ok" | "warn" | "neutral" {
  if (standing === "admitted") return "ok";
  // Withdrawn is a deliberate act by the provider, not a fault — so it is neutral, like every other
  // ordinary state in this repo. `neverAdmitted` is likewise ordinary for any address that simply is
  // not staff. Neither is amber: reserving warn for a genuine could-not-establish keeps the one
  // colour that means "look at this" meaningful.
  return "neutral";
}

/** `0x` + 40 hex. Case-insensitive, because a wallet hands back EIP-55. */
export function isAddress(value: string): boolean {
  return /^0x[0-9a-fA-F]{40}$/.test(value.trim());
}

export function normalizeAddress(value: string): string {
  return value.trim().toLowerCase();
}

const ZERO_ADDRESS = `0x${"0".repeat(40)}`;

/**
 * Why the address typed into the admit field cannot be used.
 *
 * `null` means it is well-formed and not obviously refusable. The chain still decides.
 */
export function validateSignerInput(raw: string, read: RosterRead): string | null {
  const value = raw.trim();
  if (!value) return "Enter the address that will sign.";
  if (!isAddress(value)) {
    return "That is not an address. It should be 0x followed by 40 hexadecimal characters.";
  }
  if (normalizeAddress(value) === ZERO_ADDRESS) {
    // `setIssuanceAllowed` reverts `NotLocallyAllowed()` for it, since the zero address can never
    // sign. Caught here so the provider gets a sentence rather than an opaque revert.
    return "The zero address can never sign, so this contract refuses it.";
  }
  if (read.state === "unavailable") {
    // Not a refusal of the address — a refusal to guess. Without the current list we cannot tell
    // whether this write would be a no-op, and `setIssuanceAllowed` reverts `NoChange()` on one.
    return "This contract's current list could not be read, so it is not known whether this address is already on it.";
  }
  if (read.entries.some((e) => e.address === normalizeAddress(value) && e.allowed)) {
    return "This address is already on the list, and this contract refuses a write that would change nothing.";
  }
  return null;
}

interface WalletContext {
  /** An action is already in flight anywhere on this page. */
  busy: boolean;
  connected: boolean;
  /** The connected wallet's address, lowercase. */
  account: string | null;
  expectedChainId: number;
  /** `undefined` means the connector did not report one — NOT the wrong chain. */
  actualChainId: number | undefined;
}

function walletBlock(ctx: WalletContext): ActionBlock | null {
  if (ctx.busy) return { kind: "busy" };
  if (!ctx.connected || !ctx.account) return { kind: "notConnected" };
  if (ctx.actualChainId !== undefined && ctx.actualChainId !== ctx.expectedChainId) {
    return { kind: "wrongChain", expected: ctx.expectedChainId, actual: ctx.actualChainId };
  }
  return null;
}

/**
 * Why ADMIT is unavailable.
 *
 * The owner term COMPOSES the contract's own rule rather than restating a stricter one: the chain
 * really does gate this direction on `msg.sender == owner()` and nothing else, so refusing a
 * non-owner here refuses exactly what the chain would.
 */
export function admitBlock(
  ctx: WalletContext & { read: RosterRead; inputProblem: string | null },
): ActionBlock | null {
  const wallet = walletBlock(ctx);
  if (wallet) return wallet;
  if (ctx.read.state === "unavailable") {
    return {
      kind: "otherwiseBlocked",
      why: "This contract could not be read, so neither its owner nor its current list is known. Nothing is sent on a guess.",
    };
  }
  if (normalizeAddress(ctx.account ?? "") !== normalizeAddress(ctx.read.owner)) {
    return {
      kind: "otherwiseBlocked",
      why: `Only this contract's owner can admit a signer, and that is ${ctx.read.owner}. Connect that wallet. The protocol admin deliberately cannot do this for you: it also grants the authority's issue right, so a registrar that could admit would hold both layers at once.`,
    };
  }
  if (ctx.inputProblem) return { kind: "otherwiseBlocked", why: ctx.inputProblem };
  return null;
}

/**
 * Why REMOVE is unavailable for a given row.
 *
 * The chain admits the owner OR the protocol admin here, because removal only ever narrows. This
 * page is the OWNER's surface and gates on the owner accordingly — the admin's copy of this lever
 * is its own console, and the reason below says so rather than implying removal is impossible when
 * an owner key is lost.
 */
export function removeBlock(
  ctx: WalletContext & { read: RosterRead; entry: RosterEntry },
): ActionBlock | null {
  const wallet = walletBlock(ctx);
  if (wallet) return wallet;
  if (ctx.read.state === "unavailable") {
    return {
      kind: "otherwiseBlocked",
      why: "This contract could not be read, so its owner is not known. Nothing is sent on a guess.",
    };
  }
  if (!ctx.entry.allowed) {
    return {
      kind: "otherwiseBlocked",
      why: "This address is already off the list, and this contract refuses a write that would change nothing.",
    };
  }
  if (normalizeAddress(ctx.account ?? "") !== normalizeAddress(ctx.read.owner)) {
    return {
      kind: "otherwiseBlocked",
      why: `Only this contract's owner can remove a signer here, and that is ${ctx.read.owner}. Connect that wallet. The protocol admin can also remove one — removal only ever narrows — but it does that from its own console, not this page.`,
    };
  }
  return null;
}

export { describeActionBlock };

/**
 * The headline sentence for one contract: can this deployment's own backend anchor through it?
 *
 * This is the diagnosis the gap was actually about. Layer 1 had a screen and layer 2 had none, so
 * "I am approved, my contract is attached, and issuing still fails" had no answer anywhere in the
 * product. Four states, and the two that mean "we cannot tell" are kept apart from the two that are
 * answers.
 */
export type BackendSignerVerdict =
  | { kind: "canIssue" }
  | { kind: "notAdmitted"; signer: string }
  | { kind: "noSigner" }
  | { kind: "unreadable"; reason: string };

export function backendSignerVerdict(
  activeSigner: string | null,
  read: RosterRead,
): BackendSignerVerdict {
  if (read.state === "unavailable") return { kind: "unreadable", reason: read.reason };
  if (!activeSigner || read.activeSignerAllowed === null) return { kind: "noSigner" };
  return read.activeSignerAllowed
    ? { kind: "canIssue" }
    : { kind: "notAdmitted", signer: activeSigner };
}

export function describeBackendSignerVerdict(v: BackendSignerVerdict): string {
  switch (v.kind) {
    case "canIssue":
      // Deliberately not "this shop can issue" — layer 1 is a separate grant with its own screen,
      // and claiming both from a read of one would be exactly the over-statement this page exists
      // to remove.
      return "This shop's signing key is on this contract's issuance list. If issuing still fails, the missing half is the authority's issue right, which the DogTag admin grants.";
    case "notAdmitted":
      return `This shop signs with ${v.signer}, and this contract's issuance list does not admit it — so every attempt to issue through this contract will be refused, however the DogTag admin has set the authority's issue right. Admit it below.`;
    case "noSigner":
      return "Custody is locked, so this shop's signing key cannot be derived and it is not known whether it may issue. Unlock custody and reload.";
    case "unreadable":
      return `This contract's issuance list could not be read, so whether this shop may issue through it is not known: ${v.reason}`;
  }
}

/** Tone for the headline. The two could-not-tell states warn; only a real refusal is an error. */
export function backendSignerTone(v: BackendSignerVerdict): "ok" | "warn" | "bad" {
  switch (v.kind) {
    case "canIssue":
      return "ok";
    case "notAdmitted":
      return "bad";
    case "noSigner":
    case "unreadable":
      return "warn";
  }
}
