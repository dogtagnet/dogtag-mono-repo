// Reading what a wallet actually said (registry-plan S-15).
//
// Three live defects, one root shape: reading a wallet's answer at one fixed depth with one fixed
// code, and then presenting whatever came back as though it were an answer about the subject.
//
//   1. The ROAX add fallback fired only on `4902`, MetaMask's spelling. A captain's wallet answered
//      `4100`, the fallback never ran, and he added the network by hand.
//   2. Providers wrap: viem re-throws with the original under `cause`, several extensions nest
//      theirs under `data.originalError`. A top-level `err.code` read misses both.
//   3. `4100 Unauthorized` reads as "not authorized", and it was rendered under a provider id on a
//      page about provider authorization - so it was read as a verdict about the provider while the
//      chain said that provider was active and approved.
//
// Every branch is a wallet-specific failure that cannot be reproduced in a browser without
// installing that wallet and putting it into that exact state, which is why this is pure.
import { describe, expect, it } from "vitest";
import {
  classifySurfaceFault,
  isUnrecognizedChain,
  isUserRejection,
  isWalletUnauthorized,
  walletErrorCodes,
  walletErrorMessage,
} from "../src/wallet/walletError";

describe("a code is found however deeply the provider wrapped it", () => {
  it("finds a top-level code", () => {
    expect(walletErrorCodes({ code: 4902 })).toContain(4902);
  });

  it("finds one under data.originalError, the shape several extensions use", () => {
    // The exact nesting that made a plainly-present 4902 invisible to a top-level read.
    expect(walletErrorCodes({ code: -32603, data: { originalError: { code: 4902 } } })).toContain(
      4902,
    );
  });

  it("finds one under cause, which is how viem re-throws", () => {
    const err = new Error("outer");
    (err as unknown as { cause: unknown }).cause = { code: 4001 };
    expect(walletErrorCodes(err)).toContain(4001);
  });

  it("survives an error that references itself rather than hanging", () => {
    // A provider error with a cycle would otherwise spin in the handler - a worse failure than the
    // bad reporting this walk exists to fix.
    const a: Record<string, unknown> = { code: 4902 };
    a.cause = a;
    expect(walletErrorCodes(a)).toContain(4902);
  });

  it("finds nothing in a value that carries no code, rather than inventing one", () => {
    expect(walletErrorCodes(new Error("plain"))).toEqual([]);
    expect(walletErrorCodes(undefined)).toEqual([]);
  });
});

describe("unrecognized chain is matched in every dialect, not only MetaMask's", () => {
  it("still matches the bare 4902 it always did", () => {
    expect(isUnrecognizedChain({ code: 4902 })).toBe(true);
  });

  it("matches a nested 4902", () => {
    expect(isUnrecognizedChain({ code: -32603, data: { originalError: { code: 4902 } } })).toBe(
      true,
    );
  });

  it("matches a wallet that only says it in words", () => {
    // Several wallets answer a generic -32603 and put the only usable information in the text.
    expect(isUnrecognizedChain(new Error("Unrecognized chain ID. Try adding the chain first."))).toBe(
      true,
    );
    expect(isUnrecognizedChain(new Error("Unknown chain"))).toBe(true);
  });

  it("does NOT treat a user rejection as an unknown chain", () => {
    // The pair that must never merge: adding a chain underneath somebody who just declined is the
    // one thing worse than not adding it at all.
    expect(isUnrecognizedChain({ code: 4001, message: "User rejected the request." })).toBe(false);
  });

  it("does NOT treat an unrelated failure as an unknown chain", () => {
    expect(isUnrecognizedChain(new Error("Internal JSON-RPC error"))).toBe(false);
  });
});

describe("4100 is a statement by a browser extension, never one about the provider", () => {
  it("recognises the code and the wording the captain saw", () => {
    expect(isWalletUnauthorized({ code: 4100 })).toBe(true);
    expect(
      isWalletUnauthorized(
        new Error("The requested method and/or account has not been authorized by the user."),
      ),
    ).toBe(true);
  });

  it("says, in the classified fault, that nothing about the provider was established", () => {
    // THE defect. Without this sentence the words "not been authorized" sit alone on a page about
    // authorization, and a reader supplies the missing subject themselves - wrongly.
    const fault = classifySurfaceFault(
      new Error("The requested method and/or account has not been authorized by the user."),
    );
    expect(fault.kind).toBe("walletUnauthorized");
    expect(fault.established).toMatch(/nothing about your provider record was checked/i);
    expect(fault.established).toMatch(/says nothing about whether you are authorized/i);
  });

  it("gives it a next step that names the multi-wallet case", () => {
    // The case a user cannot see from inside the page, and the one they have done nothing to cause.
    const fault = classifySurfaceFault({ code: 4100 });
    expect(fault.nextStep).toMatch(/more than one wallet extension/i);
  });
});

describe("a classified fault always carries all three parts", () => {
  const CASES: unknown[] = [
    { code: 4001, message: "User rejected the request." },
    { code: 4100, message: "not been authorized" },
    new Error("Internal JSON-RPC error"),
    "a bare string",
    undefined,
  ];

  it("never leaves the denial or the next step empty, whatever was thrown", () => {
    // A fault with no denial is the original defect: a message in a position that implies a subject.
    for (const c of CASES) {
      const f = classifySurfaceFault(c);
      expect(f.established.length, `${String(c)} has no denial`).toBeGreaterThan(20);
      expect(f.nextStep.length, `${String(c)} has no next step`).toBeGreaterThan(20);
      expect(f.detail.length).toBeGreaterThan(0);
    }
  });

  it("tells a cancellation apart from a refusal, because the remedies differ", () => {
    // One is "you pressed cancel"; the other sends somebody into extension settings. A single
    // "wallet error" would send the first user on the second errand.
    expect(classifySurfaceFault({ code: 4001 }).kind).toBe("walletRejected");
    expect(classifySurfaceFault({ code: 4100 }).kind).toBe("walletUnauthorized");
    expect(classifySurfaceFault(new Error("boom")).kind).toBe("walletFault");
  });

  it("keeps only the first line of the wallet's message", () => {
    // Kept for diagnosis, never presented as a verdict - and never as a stack trace.
    const f = classifySurfaceFault(new Error("first line\nsecond line\n    at somewhere"));
    expect(f.detail).toBe("first line");
  });

  it("reports something rather than nothing for a value with no message at all", () => {
    expect(walletErrorMessage({})).toBe("no reason given");
    expect(classifySurfaceFault({}).detail).toBe("no reason given");
  });
});

describe("a user rejection is the person's decision, not a fault", () => {
  it("is recognised by code and by wording", () => {
    expect(isUserRejection({ code: 4001 })).toBe(true);
    expect(isUserRejection(new Error("User denied transaction signature."))).toBe(true);
  });

  it("is not confused with a wallet that refused to answer", () => {
    expect(isUserRejection({ code: 4100 })).toBe(false);
  });
});
