import { describe, expect, it } from "vitest";
import type { MicrochipCheck, MicrochipNotComparable } from "../src/api/types";
import {
  microchipCheckFromError,
  microchipConfirmsAnimal,
  microchipExplanation,
  microchipHeadline,
  microchipTone,
} from "../src/domain/microchipCheck";

const CHIP = "985141006580319";
const OTHER = "900000000000001";

function notComparable(
  reason: MicrochipNotComparable,
  isFailure: boolean,
  detail = "The two could not be compared.",
): MicrochipCheck {
  return { state: "notComparable", reason, isFailure, detail };
}

/** Every reason the backend can emit, with the fact/failure split it stamps on each. */
const REASONS: Array<[MicrochipNotComparable, boolean]> = [
  ["noCredentialHeld", false],
  ["credentialHasNoMicrochip", false],
  ["petHasNoMicrochip", false],
  ["noLinkedPet", false],
  ["cannotLookUpByFieldElement", true],
  ["couldNotRead", true],
  ["credentialUnreadable", true],
];

describe("microchip cross-check presentation", () => {
  it("renders a match as positive evidence", () => {
    const c: MicrochipCheck = { state: "matched", microchip: CHIP, detail: "ok" };
    expect(microchipTone(c)).toBe("positive");
    expect(microchipConfirmsAnimal(c)).toBe(true);
    expect(microchipExplanation(c)).toContain(CHIP);
  });

  it("renders a mismatch as negative and names both codes", () => {
    const c: MicrochipCheck = {
      state: "mismatch",
      petMicrochip: OTHER,
      credentialMicrochip: CHIP,
      detail: "…",
    };
    expect(microchipTone(c)).toBe("negative");
    const text = microchipExplanation(c);
    expect(text).toContain(OTHER);
    expect(text).toContain(CHIP);
  });

  it("says a mismatch is about the LINK, not about the credential", () => {
    // The credential is genuine and stays valid for everyone; only the pairing is wrong. Copy that
    // reads as an accusation would send an operator to revoke a perfectly good credential.
    const c: MicrochipCheck = {
      state: "mismatch",
      petMicrochip: OTHER,
      credentialMicrochip: CHIP,
      detail: "…",
    };
    const text = microchipExplanation(c).toLowerCase();
    expect(text).toContain("stays valid");
    expect(text).not.toContain("invalid");
    expect(text).not.toContain("forged");
  });

  it("never renders any not-comparable state as a pass", () => {
    // The rule this whole feature exists to hold: a check that did not run is not one that passed.
    for (const [reason, isFailure] of REASONS) {
      const c = notComparable(reason, isFailure);
      expect(microchipConfirmsAnimal(c), reason).toBe(false);
      expect(microchipTone(c), reason).not.toBe("positive");
      expect(microchipHeadline(c), reason).toBe("Microchip not compared");
    }
  });

  it("never renders any not-comparable state as a refusal either", () => {
    // The inverse mistake, and the one THIS surface would make: painting every unchipped cat red
    // makes the field unusable and teaches operators to route around it.
    for (const [reason, isFailure] of REASONS) {
      expect(microchipTone(notComparable(reason, isFailure)), reason).not.toBe("negative");
    }
  });

  it("splits ordinary absences from failures to look", () => {
    // Facts are neutral; failures warn. Getting this backwards either nags the commonest state in
    // the product or hides a genuinely unreadable store.
    expect(microchipTone(notComparable("petHasNoMicrochip", false))).toBe("neutral");
    expect(microchipTone(notComparable("credentialHasNoMicrochip", false))).toBe("neutral");
    expect(microchipTone(notComparable("noCredentialHeld", false))).toBe("neutral");
    expect(microchipTone(notComparable("noLinkedPet", false))).toBe("neutral");
    expect(microchipTone(notComparable("couldNotRead", true))).toBe("warning");
    expect(microchipTone(notComparable("cannotLookUpByFieldElement", true))).toBe("warning");
    expect(microchipTone(notComparable("credentialUnreadable", true))).toBe("warning");
  });

  it("takes the tone from the wire rather than re-deriving it from the reason", () => {
    // A reason string this build has never seen must still get the tone the BACKEND stamped, not a
    // default. Re-deriving from `reason` here is how a newly-added reason silently renders neutral.
    const future = notComparable("somethingNewEntirely" as MicrochipNotComparable, true);
    expect(microchipTone(future)).toBe("warning");
    expect(microchipHeadline(future)).toBe("Microchip not compared");
  });

  it("passes the backend's reason sentence through verbatim", () => {
    // A second copy of seven reason strings in the client is a second thing to keep in step, and the
    // one that drifts is always the one the operator reads.
    const detail = "This pet has no microchip on file, so the two could not be compared.";
    expect(microchipExplanation(notComparable("petHasNoMicrochip", false, detail))).toBe(detail);
  });

  it("recovers the structured check from a refusal body", () => {
    const check: MicrochipCheck = {
      state: "mismatch",
      petMicrochip: OTHER,
      credentialMicrochip: CHIP,
      detail: "…",
    };
    const err = Object.assign(new Error("conflict"), {
      status: 409,
      body: { error: "…", microchipCheck: check },
    });
    expect(microchipCheckFromError(err)).toEqual(check);
  });

  it("does not mislabel an unrelated failure as a microchip problem", () => {
    // `POST /pets/:id/dogtag` also answers 409 when another pet already holds the tag. Reading that
    // as a microchip refusal would show the operator the wrong remedy entirely.
    const tagConflict = Object.assign(new Error("conflict"), {
      status: 409,
      body: { error: "DogTag 4 is already linked to Rex (Alice Tan)." },
    });
    expect(microchipCheckFromError(tagConflict)).toBeNull();
    expect(microchipCheckFromError(new Error("network down"))).toBeNull();
    expect(microchipCheckFromError(null)).toBeNull();
    expect(microchipCheckFromError({ body: { microchipCheck: { state: "weird" } } })).toBeNull();
  });
});

describe("a microchip this system cannot READ", () => {
  // The state that exists because of how this check first shipped: it read one key path no real
  // issuer emits, so it was INERT on every real credential — and nothing said so, because "the
  // credential has no microchip" is an ordinary, quiet, benign answer and a broken reader produced
  // exactly that. So this gets its OWN state, and these cases pin that it can never be rendered as
  // the unchipped animal's.
  const unreadable: MicrochipCheck = {
    state: "unrecognisedCredentialLeaf",
    keyPaths: ["credentialSubject.chipDetails.microchipIdentifier"],
    detail:
      "The microchip could NOT be checked: this credential carries microchip data at " +
      "credentialSubject.chipDetails.microchipIdentifier, which this system does not know how to read.",
  };

  it("is never neutral and never reads as 'not compared'", () => {
    // Both are the unchipped animal's treatment, and on a credential that IS carrying a microchip
    // they are indistinguishable from success — which is the whole camouflage.
    expect(microchipTone(unreadable)).toBe("warning");
    expect(microchipHeadline(unreadable)).not.toBe("Microchip not compared");
    expect(microchipHeadline(unreadable)).toBe("Microchip check could not run");
  });

  it("is not positive evidence and is not an accusation", () => {
    // Loud about OUR defect, but it says nothing about the animal or the credential, so it is not
    // the red reserved for a genuine wrong pairing.
    expect(microchipConfirmsAnimal(unreadable)).toBe(false);
    expect(microchipTone(unreadable)).not.toBe("positive");
    expect(microchipTone(unreadable)).not.toBe("negative");
  });

  it("names the key paths, because the path IS the remedy", () => {
    expect(microchipExplanation(unreadable)).toContain(
      "credentialSubject.chipDetails.microchipIdentifier",
    );
  });

  it("survives the refusal-body parser rather than being dropped", () => {
    // The allowlist in `microchipCheckFromError` drops any state it does not name, so a state added
    // to the backend and not to that list vanishes on exactly the path an operator is looking at.
    const err = Object.assign(new Error("conflict"), {
      status: 409,
      body: { error: "…", microchipCheck: unreadable },
    });
    expect(microchipCheckFromError(err)).toEqual(unreadable);
  });
});
