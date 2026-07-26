// Hermetic coverage for the custody lock-detection + unlock-routing helpers (src/custody/lock.ts).
// These decide when a portal yanks the operator onto /unlock and where it puts them afterwards, so
// the failure modes worth pinning are the ones that STRAND someone: mis-reading an overloaded 409 as
// "locked", redirecting on an unknown state, or letting `?next=` escape the origin.
import { describe, expect, it } from "vitest";
import {
  buildUnlockPath,
  custodyStateFromSigners,
  isCustodyLockedError,
  isLockExemptPath,
  isNoSealError,
  sanitizeNextPath,
  UNLOCK_PATH,
} from "../src/custody/lock";

/** Shape of the ApiError the shared client throws (message + status + parsed body). */
function apiError(status: number, error: string) {
  const e = new Error(error) as Error & { status: number; body: unknown };
  e.status = status;
  e.body = { error };
  return e;
}

describe("isCustodyLockedError", () => {
  it("matches the 409 route guards", () => {
    expect(isCustodyLockedError(apiError(409, "not unlocked"))).toBe(true);
  });

  it("matches CustodyError::Locked surfaced as a 500", () => {
    // custody.rs renders the same text; a status-only rule would miss this one.
    expect(isCustodyLockedError(apiError(500, "not unlocked"))).toBe(true);
  });

  it("does NOT match the other 409s the custody routes emit", () => {
    // 409 is overloaded — treating any of these as "locked" would bounce the operator to /unlock
    // where no passphrase can help.
    for (const msg of ["not initialized", "already initialized", "no pending genesis"]) {
      expect(isCustodyLockedError(apiError(409, msg))).toBe(false);
    }
  });

  it("ignores unrelated failures", () => {
    expect(isCustodyLockedError(apiError(401, "invalid operator session"))).toBe(false);
    expect(isCustodyLockedError(new Error("Failed to fetch"))).toBe(false);
    expect(isCustodyLockedError(null)).toBe(false);
    expect(isCustodyLockedError(undefined)).toBe(false);
  });

  it("reads a plain Error message when no ApiError body is present", () => {
    expect(isCustodyLockedError(new Error("not unlocked"))).toBe(true);
  });
});

describe("isNoSealError", () => {
  it("matches /admin/unlock's uninitialized refusal and nothing else", () => {
    expect(isNoSealError(apiError(409, "not initialized"))).toBe(true);
    expect(isNoSealError(apiError(409, "not unlocked"))).toBe(false);
  });
});

describe("custodyStateFromSigners", () => {
  it("reads the locked short-circuit shape", () => {
    expect(custodyStateFromSigners({ signers: [] })).toBe("locked");
  });

  it("reads the unlocked shape", () => {
    expect(custodyStateFromSigners({ activeSigner: "0xabc", matrix: [] })).toBe("unlocked");
  });

  it("treats an EMPTY activeSigner as unlocked, not locked", () => {
    // active_address() falls back to "" — a false "locked" would trap the operator in a redirect
    // loop, while a false "unlocked" self-corrects on the first real request.
    expect(custodyStateFromSigners({ activeSigner: "", matrix: [] })).toBe("unlocked");
  });

  it("stays unknown for anything it does not recognise", () => {
    for (const junk of [null, undefined, "", 0, "not unlocked"]) {
      expect(custodyStateFromSigners(junk)).toBe("unknown");
    }
  });
});

describe("sanitizeNextPath", () => {
  const FALLBACK = "/issue";

  it("keeps a same-origin path with its query", () => {
    expect(sanitizeNextPath("/records?status=issued", FALLBACK)).toBe("/records?status=issued");
  });

  it("decodes an encoded destination", () => {
    expect(sanitizeNextPath("%2Frecords%3Fstatus%3Dissued", FALLBACK)).toBe(
      "/records?status=issued",
    );
  });

  it("rejects off-origin destinations", () => {
    for (const evil of ["//evil.example", "/\\evil.example", "https://evil.example", "evil"]) {
      expect(sanitizeNextPath(evil, FALLBACK)).toBe(FALLBACK);
    }
  });

  it("rejects the unlock route itself so a success cannot bounce back", () => {
    expect(sanitizeNextPath(UNLOCK_PATH, FALLBACK)).toBe(FALLBACK);
    expect(sanitizeNextPath(`${UNLOCK_PATH}?next=%2Frecords`, FALLBACK)).toBe(FALLBACK);
  });

  it("falls back on an absent destination", () => {
    expect(sanitizeNextPath(null, FALLBACK)).toBe(FALLBACK);
    expect(sanitizeNextPath("", FALLBACK)).toBe(FALLBACK);
  });
});

describe("buildUnlockPath", () => {
  it("round-trips through sanitizeNextPath", () => {
    const url = buildUnlockPath("/records?status=issued");
    expect(url).toBe("/unlock?next=%2Frecords%3Fstatus%3Dissued");
    const next = new URLSearchParams(url.split("?")[1]).get("next");
    expect(sanitizeNextPath(next, "/issue")).toBe("/records?status=issued");
  });

  it("omits the param when the origin is not restorable", () => {
    expect(buildUnlockPath("https://evil.example")).toBe(UNLOCK_PATH);
    expect(buildUnlockPath(UNLOCK_PATH)).toBe(UNLOCK_PATH);
  });
});

describe("isLockExemptPath", () => {
  it("exempts the unlock page and Setup (which owns genesis)", () => {
    expect(isLockExemptPath("/unlock")).toBe(true);
    expect(isLockExemptPath("/setup")).toBe(true);
  });

  it("gates everything else", () => {
    for (const p of ["/issue", "/records", "/settings", "/dashboard", "/"]) {
      expect(isLockExemptPath(p)).toBe(false);
    }
  });

  it("does not exempt by prefix accident", () => {
    expect(isLockExemptPath("/setup-wizard")).toBe(false);
    expect(isLockExemptPath("/unlocked")).toBe(false);
  });
});
