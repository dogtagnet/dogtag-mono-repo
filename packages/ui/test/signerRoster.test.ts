// The pure decisions behind "who may sign in your name" — LAYER 2 of the two-layer issuance
// requirement.
//
// Everything a row CLAIMS is decided here, so this is where those claims are pinned. The mounted
// half (`signerRosterRender.test.tsx`) asserts they actually reach the DOM; a pure assertion cannot
// see a paint, and a mounted one cannot enumerate every branch.

import { describe, expect, it } from "vitest";
import {
  admitBlock,
  backendSignerTone,
  backendSignerVerdict,
  describeActionBlock,
  describeBackendSignerVerdict,
  removeBlock,
  signerStanding,
  signerStandingLabel,
  signerStandingTone,
  validateSignerInput,
  type RosterEntry,
  type RosterRead,
} from "../src/signers";

const OWNER = "0x00000000000000000000000000000000000000a1";
const OURS = "0x00000000000000000000000000000000000000b7";
const OTHER = "0x00000000000000000000000000000000000000c3";
const ZERO = `0x${"0".repeat(40)}`;

const entry = (address: string, allowed: boolean, everNamed: boolean): RosterEntry => ({
  address,
  allowed,
  everNamed,
});

const resolved = (entries: RosterEntry[], activeSignerAllowed: boolean | null = false): RosterRead => ({
  state: "resolved",
  owner: OWNER,
  entries,
  activeSignerAllowed,
});

const unavailable: RosterRead = { state: "unavailable", reason: "eth_getLogs refused" };

const wallet = (account: string | null) => ({
  busy: false,
  connected: !!account,
  account,
  expectedChainId: 135,
  actualChainId: 135 as number | undefined,
});

// -------------------------------------------------------------------------------------------------
// Standing — three values, never a boolean
// -------------------------------------------------------------------------------------------------

describe("a row says which of the three states an address is in", () => {
  it("distinguishes withdrawn from never-admitted, which `allowed: false` spells identically", () => {
    expect(signerStanding(entry(OURS, false, true))).toBe("withdrawn");
    expect(signerStanding(entry(OURS, false, false))).toBe("neverAdmitted");
    expect(signerStanding(entry(OURS, true, true))).toBe("admitted");
  });

  it("carries the distinction as a WORD, so it survives a flattened text dump", () => {
    // The failure this exists to prevent, named in the task: a withdrawn holder read as a current
    // one because the distinction was carried by styling alone.
    expect(signerStandingLabel("withdrawn")).toMatch(/withdrawn/i);
    expect(signerStandingLabel("neverAdmitted")).not.toMatch(/withdrawn/i);
    expect(signerStandingLabel("admitted")).not.toMatch(/withdrawn/i);
    // And the three words are genuinely different, so no two rows can read the same.
    const labels = (["admitted", "withdrawn", "neverAdmitted"] as const).map(signerStandingLabel);
    expect(new Set(labels).size).toBe(3);
  });

  it("reserves the warn tone: a withdrawal is a deliberate act, not a fault", () => {
    expect(signerStandingTone("admitted")).toBe("ok");
    expect(signerStandingTone("withdrawn")).toBe("neutral");
    expect(signerStandingTone("neverAdmitted")).toBe("neutral");
  });
});

// -------------------------------------------------------------------------------------------------
// The headline verdict — the diagnosis the gap was actually about
// -------------------------------------------------------------------------------------------------

describe("whether THIS shop's backend may anchor through a contract", () => {
  it("names the signer and says the attempt will be refused when it is not admitted", () => {
    const v = backendSignerVerdict(OURS, resolved([entry(OURS, false, false)], false));
    expect(v).toEqual({ kind: "notAdmitted", signer: OURS });
    expect(describeBackendSignerVerdict(v)).toContain(OURS);
    expect(backendSignerTone(v)).toBe("bad");
  });

  it("does NOT claim the shop can issue when it can only see layer 2", () => {
    // Layer 1 is a separate grant with its own screen. Claiming both from a read of one would be
    // exactly the over-statement this page exists to remove.
    const v = backendSignerVerdict(OURS, resolved([entry(OURS, true, true)], true));
    expect(v.kind).toBe("canIssue");
    const text = describeBackendSignerVerdict(v);
    expect(text).toMatch(/issue right|authority/i);
  });

  it("an unreadable list is its own state and warns rather than accusing", () => {
    const v = backendSignerVerdict(OURS, unavailable);
    expect(v).toEqual({ kind: "unreadable", reason: "eth_getLogs refused" });
    expect(backendSignerTone(v)).toBe("warn");
    expect(describeBackendSignerVerdict(v)).toContain("eth_getLogs refused");
  });

  it("locked custody has no signer to speak about, and says so rather than answering false", () => {
    const v = backendSignerVerdict(null, resolved([], null));
    expect(v.kind).toBe("noSigner");
    expect(backendSignerTone(v)).toBe("warn");
    expect(describeBackendSignerVerdict(v)).toMatch(/locked/i);
  });

  it("an unreadable list outranks a missing signer - we cannot answer either way", () => {
    expect(backendSignerVerdict(null, unavailable).kind).toBe("unreadable");
  });
});

// -------------------------------------------------------------------------------------------------
// Admitting — the owner's alone, and the reason says why
// -------------------------------------------------------------------------------------------------

describe("admitting is the contract owner's alone", () => {
  it("refuses a wallet that is not the owner, names the owner, and says why the admin cannot", () => {
    const block = admitBlock({
      ...wallet(OTHER),
      read: resolved([]),
      inputProblem: null,
    });
    expect(block).not.toBeNull();
    const why = describeActionBlock(block!);
    expect(why).toContain(OWNER);
    // The security claim, in the copy: a registrar that could admit would hold both layers at once.
    expect(why).toMatch(/both layers|issue right/i);
  });

  it("allows the owner", () => {
    expect(
      admitBlock({ ...wallet(OWNER), read: resolved([]), inputProblem: null }),
    ).toBeNull();
  });

  it("matches the owner regardless of EIP-55 casing, which is what a wallet returns", () => {
    const checksummed = OWNER.toUpperCase().replace("0X", "0x");
    expect(
      admitBlock({ ...wallet(checksummed), read: resolved([]), inputProblem: null }),
    ).toBeNull();
  });

  it("sends nothing on a guess when the contract could not be read", () => {
    const block = admitBlock({ ...wallet(OWNER), read: unavailable, inputProblem: null });
    expect(block).not.toBeNull();
    expect(describeActionBlock(block!)).toMatch(/could not be read/i);
  });

  it("reports a wrong chain rather than a permissions problem", () => {
    const block = admitBlock({
      ...wallet(OWNER),
      actualChainId: 1,
      read: resolved([]),
      inputProblem: null,
    });
    expect(block?.kind).toBe("wrongChain");
  });

  it("a connector that reports NO chain is not accused of being on the wrong one", () => {
    expect(
      admitBlock({
        ...wallet(OWNER),
        actualChainId: undefined,
        read: resolved([]),
        inputProblem: null,
      }),
    ).toBeNull();
  });
});

// -------------------------------------------------------------------------------------------------
// The address field
// -------------------------------------------------------------------------------------------------

describe("what may be typed into the admit field", () => {
  it("refuses the zero address, which the contract itself refuses", () => {
    expect(validateSignerInput(ZERO, resolved([]))).toMatch(/zero address/i);
  });

  it("refuses an address already on the list, because the write would revert NoChange", () => {
    const problem = validateSignerInput(OTHER, resolved([entry(OTHER, true, true)]));
    expect(problem).toMatch(/already on the list/i);
  });

  it("ACCEPTS an address that was withdrawn - re-admitting is a real change", () => {
    expect(validateSignerInput(OTHER, resolved([entry(OTHER, false, true)]))).toBeNull();
  });

  it("declines to guess when the list could not be read, rather than refusing the address", () => {
    const problem = validateSignerInput(OTHER, unavailable);
    expect(problem).toMatch(/could not be read/i);
    // It must not read as an accusation about the address the provider typed.
    expect(problem).not.toMatch(/already on the list/i);
  });

  it("refuses something that is not an address at all", () => {
    expect(validateSignerInput("not-an-address", resolved([]))).toMatch(/not an address/i);
    expect(validateSignerInput("", resolved([]))).toMatch(/enter the address/i);
  });
});

// -------------------------------------------------------------------------------------------------
// Removing
// -------------------------------------------------------------------------------------------------

describe("removing a signer", () => {
  it("is offered to the owner for an address that is currently on the list", () => {
    expect(
      removeBlock({ ...wallet(OWNER), read: resolved([]), entry: entry(OTHER, true, true) }),
    ).toBeNull();
  });

  it("is refused for an address already off the list, which would revert NoChange", () => {
    const block = removeBlock({
      ...wallet(OWNER),
      read: resolved([]),
      entry: entry(OTHER, false, true),
    });
    expect(describeActionBlock(block!)).toMatch(/already off the list/i);
  });

  it("tells a non-owner that the protocol admin can also remove, from its own console", () => {
    // Removal only ever narrows, so the chain admits the owner OR the protocol admin. This page is
    // the owner's surface; the copy must not imply removal is impossible when an owner key is lost.
    const block = removeBlock({
      ...wallet(OTHER),
      read: resolved([]),
      entry: entry(OURS, true, true),
    });
    const why = describeActionBlock(block!);
    expect(why).toContain(OWNER);
    expect(why).toMatch(/protocol admin can also remove/i);
  });
});

// -------------------------------------------------------------------------------------------------
// Shared preconditions
// -------------------------------------------------------------------------------------------------

describe("every control says why it cannot be used", () => {
  it("names the missing wallet rather than going dead in silence", () => {
    const block = admitBlock({ ...wallet(null), read: resolved([]), inputProblem: null });
    expect(block?.kind).toBe("notConnected");
    expect(describeActionBlock(block!)).toMatch(/connect your wallet/i);
  });

  it("reports an in-flight action ahead of anything else, because it is transient", () => {
    const block = admitBlock({
      ...wallet(null),
      busy: true,
      read: resolved([]),
      inputProblem: null,
    });
    expect(block?.kind).toBe("busy");
  });

  it("gives each situation its own sentence", () => {
    const reasons = [
      admitBlock({ ...wallet(null), read: resolved([]), inputProblem: null }),
      admitBlock({ ...wallet(OWNER), actualChainId: 1, read: resolved([]), inputProblem: null }),
      admitBlock({ ...wallet(OTHER), read: resolved([]), inputProblem: null }),
      admitBlock({ ...wallet(OWNER), read: unavailable, inputProblem: null }),
      admitBlock({ ...wallet(OWNER), read: resolved([]), inputProblem: "typed badly" }),
    ].map((b) => describeActionBlock(b!));
    expect(new Set(reasons).size).toBe(reasons.length);
  });
});
