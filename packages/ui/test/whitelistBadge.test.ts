// The operator's whitelist matrix has THREE states on the wire, and the third must not be spelled the
// same way as either of the other two.
//
// `GET /issuer/signers` used to answer a bare bool that defaulted to `false` on any read failure, so an
// unreachable RPC told the operator "this signer is not approved" - their own transport fault rendered
// as a permissions problem. The backend now answers `null` for that case, and this is the decision that
// carries the distinction to the screen.
//
// The decision is a pure function rather than JSX so it can be pinned without mounting `StatusPanel`,
// which pulls in wagmi's `useAccount`/`useBalance` and would need a DOM plus mocked hooks. What matters
// is that the three states render DIFFERENTLY, and that is a property of this function.
import { describe, expect, it } from "vitest";
import { whitelistBadge } from "../src/domain/StatusPanel";

const row = (whitelisted: boolean | null) => ({ recordType: "VACCINATION", whitelisted });

describe("whitelistBadge", () => {
  it("distinguishes all three states - the null case shares nothing with either neighbour", () => {
    const granted = whitelistBadge(row(true));
    const refused = whitelistBadge(row(false));
    const unknown = whitelistBadge(row(null));

    // A test asserting only that null is "not the granted case" would pass while null still rendered
    // as the definite red X, which is the exact defect this exists to prevent. So it is compared
    // against BOTH neighbours, on BOTH axes a badge has.
    for (const other of [granted, refused]) {
      expect(unknown.tone).not.toBe(other.tone);
      expect(unknown.label).not.toBe(other.label);
      expect(unknown.icon).not.toBe(other.icon);
    }
    // …and the two definite states stay distinct from each other too.
    expect(granted.tone).not.toBe(refused.tone);
    expect(granted.icon).not.toBe(refused.icon);
  });

  it("names WHY the third state is third, in text rather than only in colour", () => {
    const unknown = whitelistBadge(row(null));
    // Colour alone is legible only to someone who already knows the vocabulary, and a hover title
    // survives neither a screenshot nor a touch device. The words have to be in the badge.
    expect(unknown.label).toContain("could not check");
    expect(unknown.label).toContain("VACCINATION");
    // Amber, the tone this repo already spells "unresolved" with - never the definite-refusal tone.
    expect(unknown.tone).toBe("warning");
    expect(unknown.tone).not.toBe("neutral");
    expect(unknown.tone).not.toBe("success");
  });

  it("keeps the two definite answers definite", () => {
    expect(whitelistBadge(row(true))).toEqual({
      tone: "success",
      label: "VACCINATION",
      icon: "granted",
    });
    const refused = whitelistBadge(row(false));
    expect(refused.icon).toBe("refused");
    // A definite refusal says so in words too, so it cannot be mistaken for the unresolved chip on a
    // grayscale screenshot.
    expect(refused.label).toContain("not approved");
    expect(refused.label).not.toContain("could not check");
  });

  it("treats an ABSENT field like an unresolved one, never like a refusal", () => {
    // A row from a backend too old to carry the field states nothing about the signer. `undefined` is
    // falsy exactly like `null`, so a renderer that keyed on truthiness would call it "not approved".
    const missing = whitelistBadge({ recordType: "VACCINATION" } as never);
    expect(missing).toEqual(whitelistBadge(row(null)));
  });
});
