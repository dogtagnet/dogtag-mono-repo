// What a sent transaction is allowed to claim (registry-plan S-15).
//
// THE DEFECT. The flows recorded `Deployed contract: <hash>` straight from `writeContractAsync`,
// which resolves the moment the wallet hands back a hash and does NOT throw when the transaction
// later mines with status 0. So a reverted deploy left a message asserting it had succeeded, on the
// one surface whose whole design principle is that an unestablished fact is never stated as a
// verdict - and the preflight explicitly is not a gate, so a plan can be ready and the write still
// revert (a standing change between Check and Send, a delegate expiring, a registrar repointing).
//
// The cases below are about the two boundaries that are easy to collapse: submitted must not read
// as succeeded, and an unfetchable receipt must read as NEITHER neighbour.
import { describe, expect, it } from "vitest";
import {
  hasNoHash,
  isUnsettled,
  mayContinueAfter,
  outcomeFromReceiptStatus,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  type SendState,
} from "../src/provider";

const HASH = `0x${"ab".repeat(32)}`;

describe("a receipt's status is the only thing that settles an outcome", () => {
  it("success is succeeded and reverted is reverted", () => {
    expect(outcomeFromReceiptStatus("success")).toBe("succeeded");
    expect(outcomeFromReceiptStatus("reverted")).toBe("reverted");
  });

  it("only an established success licenses the next transaction in a sequence", () => {
    // `writeContractAsync` does not throw on a revert, so without this a multi-step publication
    // would send the pin after the anchor had already failed.
    expect(mayContinueAfter("succeeded")).toBe(true);
    for (const state of ["submitted", "reverted", "unknown"] as SendState[]) {
      expect(mayContinueAfter(state)).toBe(false);
    }
  });
});

const ALL_STATES: SendState[] = [
  "awaitingWallet",
  "submitted",
  "succeeded",
  "reverted",
  "unknown",
  "walletSilent",
];

describe("the six states are six states, and none reads as another", () => {
  it("submitted never claims the action happened", () => {
    const label = sendStateLabel("submitted");
    expect(label).toMatch(/not yet known/i);
    // The words a provider would read as a completed action. None of them may appear.
    expect(label).not.toMatch(/succeeded|published|deployed|done|complete/i);
  });

  it("gives every state its own wording", () => {
    expect(new Set(ALL_STATES.map(sendStateLabel)).size).toBe(ALL_STATES.length);
  });

  it("an unestablished outcome is NEITHER neighbour", () => {
    const label = sendStateLabel("unknown");
    expect(label).not.toMatch(/succeeded/i);
    expect(label).not.toMatch(/reverted|failed/i);
    expect(label).toMatch(/could not be established/i);
  });

  it("a silent wallet says the transaction MAY still have been sent", () => {
    // THE state that exists because a deploy mined and the page showed nothing. Saying "failed"
    // here would be a definite claim about a transaction that was, in the captain's case, already
    // on chain; saying nothing is what produced the defect.
    const label = sendStateLabel("walletSilent");
    expect(label).toMatch(/may still have been sent/i);
    expect(label).not.toMatch(/failed|reverted|cancelled|succeeded/i);
  });

  it("keeps the two could-not-tell states apart, because their remedies differ", () => {
    // One has a hash and sends you to the explorer; the other has none and sends you to your wallet.
    // Merging them would tell somebody to look up a hash that does not exist.
    expect(sendStateLabel("unknown")).not.toBe(sendStateLabel("walletSilent"));
    expect(isUnsettled("unknown")).toBe(true);
    expect(isUnsettled("walletSilent")).toBe(true);
    expect(hasNoHash("walletSilent")).toBe(true);
    expect(hasNoHash("unknown")).toBe(false);
  });

  it("waiting for a wallet is not an outcome and is not a failure", () => {
    const label = sendStateLabel("awaitingWallet");
    expect(label).toMatch(/waiting for your wallet/i);
    expect(label).not.toMatch(/failed|error|succeeded/i);
    expect(mayContinueAfter("awaitingWallet")).toBe(false);
    expect(mayContinueAfter("walletSilent")).toBe(false);
  });
});

describe("a record without a wallet answer cannot pretend to have one", () => {
  it("refuses a hash on a state that by definition has none", () => {
    expect(() => sendRecord("s1", "Deploy", "awaitingWallet", { hash: HASH })).toThrow(
      /cannot carry a transaction hash/,
    );
  });

  it("refuses a settled state with no hash", () => {
    expect(() => sendRecord("s1", "Deploy", "succeeded")).toThrow(/must carry its transaction hash/);
  });

  it("requires a silent wallet to state its reason, like every unsettled record", () => {
    expect(() => sendRecord("s1", "Deploy", "walletSilent")).toThrow(/must state its reason/);
    const r = sendRecord("s1", "Deploy", "walletSilent", { unknownReason: "no response" });
    expect(r.hash).toBeUndefined();
    expect(r.unknownReason).toBe("no response");
  });

  it("offers no explorer link when there is no hash to address", () => {
    // A URL built from a missing hash would claim a transaction exists to look at, which is exactly
    // what these states cannot say.
    expect(sendExplorerHref(sendRecord("s1", "Deploy", "awaitingWallet"))).toBeNull();
    expect(
      sendExplorerHref(sendRecord("s1", "Deploy", "walletSilent", { unknownReason: "x" })),
    ).toBeNull();
  });
});

describe("the reason invariant is enforced rather than remembered", () => {
  it("an unknown outcome MUST state its reason", () => {
    expect(() => sendRecord("s1", "Deploy contract", "unknown", { hash: HASH })).toThrow(/must state its reason/);
  });

  it("only an unknown outcome MAY carry a reason", () => {
    expect(() => sendRecord("s1", "Deploy contract", "succeeded", { hash: HASH, unknownReason: "why" })).toThrow(
      /only an unknown outcome/,
    );
  });

  it("a settled record carries no reason key at all", () => {
    expect("unknownReason" in sendRecord("s1", "Deploy contract", "succeeded", { hash: HASH })).toBe(false);
    expect(sendRecord("s1", "Deploy contract", "unknown", { hash: HASH, unknownReason: "timed out" }).unknownReason).toBe(
      "timed out",
    );
  });
});

describe("an explorer link is a CLAIM", () => {
  it("is offered for a hash that can actually address a transaction", () => {
    expect(sendExplorerHref(sendRecord("s1", "Deploy contract", "succeeded", { hash: HASH }))).toContain(HASH);
  });

  it("is withheld for a hash that cannot", () => {
    // A wallet returning something malformed gets its value rendered inert rather than a dead link -
    // the repo's standing rule, and the reason this goes through the shared `txExplorerHref` rather
    // than composing a URL here.
    for (const bad of ["0x", "0x0800", "not-a-hash"]) {
      expect(sendExplorerHref(sendRecord("s1", "Deploy contract", "succeeded", { hash: bad }))).toBeNull();
    }
  });
});
