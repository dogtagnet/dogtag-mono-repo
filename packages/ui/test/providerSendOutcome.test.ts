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

describe("the four states are four states, and none reads as another", () => {
  it("submitted never claims the action happened", () => {
    const label = sendStateLabel("submitted");
    expect(label).toMatch(/not yet known/i);
    // The words a provider would read as a completed action. None of them may appear.
    expect(label).not.toMatch(/succeeded|published|deployed|done|complete/i);
  });

  it("gives every state its own wording", () => {
    const labels = (["submitted", "succeeded", "reverted", "unknown"] as SendState[]).map(
      sendStateLabel,
    );
    expect(new Set(labels).size).toBe(4);
  });

  it("an unestablished outcome is NEITHER neighbour", () => {
    const label = sendStateLabel("unknown");
    expect(label).not.toMatch(/succeeded/i);
    expect(label).not.toMatch(/reverted|failed/i);
    expect(label).toMatch(/could not be established/i);
  });
});

describe("the reason invariant is enforced rather than remembered", () => {
  it("an unknown outcome MUST state its reason", () => {
    expect(() => sendRecord(HASH, "Deploy contract", "unknown")).toThrow(/must state its reason/);
  });

  it("only an unknown outcome MAY carry a reason", () => {
    expect(() => sendRecord(HASH, "Deploy contract", "succeeded", "why")).toThrow(
      /only an unknown outcome/,
    );
  });

  it("a settled record carries no reason key at all", () => {
    expect("unknownReason" in sendRecord(HASH, "Deploy contract", "succeeded")).toBe(false);
    expect(sendRecord(HASH, "Deploy contract", "unknown", "timed out").unknownReason).toBe(
      "timed out",
    );
  });
});

describe("an explorer link is a CLAIM", () => {
  it("is offered for a hash that can actually address a transaction", () => {
    expect(sendExplorerHref(sendRecord(HASH, "Deploy contract", "succeeded"))).toContain(HASH);
  });

  it("is withheld for a hash that cannot", () => {
    // A wallet returning something malformed gets its value rendered inert rather than a dead link -
    // the repo's standing rule, and the reason this goes through the shared `txExplorerHref` rather
    // than composing a URL here.
    for (const bad of ["0x", "0x0800", "not-a-hash"]) {
      expect(sendExplorerHref(sendRecord(bad, "Deploy contract", "succeeded"))).toBeNull();
    }
  });
});
