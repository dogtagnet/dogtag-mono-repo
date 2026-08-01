// The captain's location ruling, pinned (registry-plan S-15).
//
//   "Location is OPTIONAL. A provider may publish contact details only. A provider with no location
//    must not appear in the mobile nearby list at all, and must never be given a placeholder
//    coordinate - rather than a false coordinate like 0 0."
//
// THE ASSERTION THAT CARRIES THAT RULE is not "the pin's coordinates are absent" - it is that NO PIN
// STEP EXISTS in the plan. A step carrying nulls would still be a step somebody could send, and a
// plan that emitted one for a contact-only provider would be one careless `?? 0` away from the Gulf
// of Guinea. `ProviderDirectory` itself cannot help here: 0,0 is a real coordinate, so a pin at the
// origin is byte-for-byte a genuine one - `ProviderSelfService.t.sol`'s
// `test_the_chain_cannot_tell_a_placeholder_from_a_real_coordinate` pins that absence deliberately,
// which is what makes this file the load-bearing copy rather than a second opinion.
import { describe, expect, it } from "vitest";
import {
  contactBlob,
  planDirectoryPublication,
  toContractCoordinate,
  type Address,
  type DigestFn,
  type HexWord,
  type ProviderChainReader,
} from "../src/provider";

const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const CALLER: Address = "0x2222222222222222222222222222222222222222";

const digest: DigestFn = (utf8) =>
  `0x${Array.from(utf8).reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 0xffffffff, 7).toString(16).padStart(64, "0")}` as HexWord;

function reader(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  const unscripted = (name: string) => async () => {
    throw new Error(`the test did not script ${name}`);
  };
  return {
    isFactoryClone: unscripted("isFactoryClone"),
    cloneOwner: unscripted("cloneOwner"),
    service: unscripted("service"),
    effectiveService: unscripted("effectiveService"),
    provider: unscripted("provider"),
    currentService: unscripted("currentService"),
    canCreateService: unscripted("canCreateService"),
    predictIssuer: unscripted("predictIssuer"),
    domainClaimStanding: unscripted("domainClaimStanding"),
    canWriteDomain: unscripted("canWriteDomain"),
    directoryIsLiveFor: async () => true,
    canWriteProviderRecord: async () => true,
    ...overrides,
  } as ProviderChainReader;
}

const CONTACTS = {
  phone: "+65 6123 4567",
  whatsapp: "",
  telegram: "",
  email: "hello@clinic.example.sg",
  website: "",
};

const base = {
  providerId: PROVIDER,
  caller: CALLER,
  contacts: CONTACTS,
  locationKind: 1,
  locationActive: true,
};

describe("a contact-only provider publishes no location, and none is invented", () => {
  it("emits NO pin step at all when both location fields are blank", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );

    expect(plan.verdict).toBe("ready");
    expect(plan.canPublish).toBe(true);
    expect(plan.contactOnly).toBe(true);
    // The rule, stated the only way that cannot be softened later.
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    expect(plan.steps).toHaveLength(1);
    expect(plan.steps[0]!.kind).toBe("profileAnchor");
  });

  it("NEVER produces a 0,0 coordinate from a blank field - `Number(\"\")` is 0, not NaN", async () => {
    // The exact mechanism that put two admin register forms' blank providers in the Gulf of Guinea.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );
    const serialized = JSON.stringify(plan.steps);
    expect(plan.steps.some((s) => s.kind === "pin")).toBe(false);
    // Belt and braces on the shape itself: no step carries a coordinate key at all.
    expect(serialized).not.toContain('"lat"');
    expect(serialized).not.toContain('"lng"');
  });

  it("tells the provider they are LISTED, not that they are missing", async () => {
    // A provider who reads "you will not appear in nearby" as "you will not appear" is exactly the
    // provider who invents a coordinate to fix it.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );
    expect(plan.checks.find((c) => c.id === "directory-location")!.outcome).toBe("pass");
    expect(plan.checks.find((c) => c.id === "directory-location")!.finding).toMatch(
      /normal listing, not an incomplete one/i,
    );
    expect(plan.nextStep).toMatch(/will not appear in the nearby list/i);
  });

  it("an absent location is a PASS, not a failure - contact-only is first class", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "  ", lngInput: "  " },
      reader(),
      digest,
    );
    expect(plan.checks.find((c) => c.id === "directory-location")!.outcome).toBe("pass");
    expect(plan.verdict).toBe("ready");
  });
});

describe("a located provider does publish a pin", () => {
  it("appends exactly one pin step, scaled to the contract's 1e6 integer", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.290270", lngInput: "103.851959" },
      reader(),
      digest,
    );

    expect(plan.contactOnly).toBe(false);
    const pins = plan.steps.filter((s) => s.kind === "pin");
    expect(pins).toHaveLength(1);
    expect(pins[0]).toMatchObject({ lat: 1_290_270, lng: 103_851_959, locationKind: 1, active: true });
    // Order matters: the anchor lists the provider, so it goes first.
    expect(plan.steps[0]!.kind).toBe("profileAnchor");
  });

  it("0,0 typed DELIBERATELY is published, because it is a real place", async () => {
    // The counterpart that keeps the rule honest. The refusal is of a PLACEHOLDER produced from a
    // blank field, never of the origin as a coordinate - a rule that refused 0,0 outright would
    // refuse a genuine provider there and would be wrong in the other direction.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "0", lngInput: "0" },
      reader(),
      digest,
    );
    expect(plan.contactOnly).toBe(false);
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(1);
    expect(plan.steps[1]).toMatchObject({ lat: 0, lng: 0 });
  });

  it("rounds rather than truncates, so southern and western pins are not biased", () => {
    expect(toContractCoordinate(-1.2345678)).toBe(-1_234_568);
    expect(toContractCoordinate(1.2345678)).toBe(1_234_568);
    expect(toContractCoordinate(-0.0000004)).toBe(-0);
  });
});

describe("a half-filled or unusable location is refused, and publishes nothing", () => {
  it("one coordinate is not a place", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.29", lngInput: "" },
      reader(),
      digest,
    );
    expect(plan.verdict).toBe("refused");
    expect(plan.canPublish).toBe(false);
    expect(plan.checks.find((c) => c.id === "directory-location")!.outcome).toBe("fail");
  });

  it("a refused location adds NO pin step - it must not be published as if understood", async () => {
    for (const [lat, lng] of [
      ["1.29", ""],
      ["999", "103.8"],
      ["abc", "def"],
    ]) {
      const plan = await planDirectoryPublication(
        { ...base, latInput: lat!, lngInput: lng! },
        reader(),
        digest,
      );
      expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
      expect(plan.verdict).toBe("refused");
    }
  });
});

describe("the contact blob", () => {
  it("omits a blank channel rather than publishing an empty one", () => {
    const { blob, channelsPublished } = contactBlob(CONTACTS);
    expect(channelsPublished).toBe(2);
    const parsed = JSON.parse(blob) as { contact: Record<string, string> };
    expect(Object.keys(parsed.contact)).toEqual(["phone", "email"]);
    expect(parsed.contact).not.toHaveProperty("whatsapp");
  });

  it("is stable for the same input, so re-publishing does not move the digest", () => {
    expect(contactBlob(CONTACTS).blob).toBe(contactBlob({ ...CONTACTS }).blob);
  });

  it("distinguishes an omitted channel from a published empty one", () => {
    // If these produced the same document the digest could not tell "I did not publish a website"
    // from "I published an empty website".
    const withWebsite = contactBlob({ ...CONTACTS, website: "https://clinic.example.sg" });
    expect(withWebsite.blob).not.toBe(contactBlob(CONTACTS).blob);
    expect(withWebsite.channelsPublished).toBe(3);
  });

  it("warns when a provider would publish neither contacts nor a location", async () => {
    const plan = await planDirectoryPublication(
      {
        ...base,
        latInput: "",
        lngInput: "",
        contacts: { phone: "", whatsapp: "", telegram: "", email: "", website: "" },
      },
      reader(),
      digest,
    );
    expect(plan.nextStep).toMatch(/neither contact details nor a location/i);
  });
});

describe("an unreadable directory is indeterminate, never a refusal", () => {
  it("reports could-not-run with the reason and publishes nothing", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader({
        directoryIsLiveFor: async () => {
          throw new Error("HTTP request failed: 502");
        },
      }),
      digest,
    );
    expect(plan.verdict).toBe("indeterminate");
    expect(plan.canPublish).toBe(false);
    const check = plan.checks.find((c) => c.id === "directory-resolver-live")!;
    expect(check.outcome).toBe("could-not-run");
    expect(check.couldNotRunReason).toContain("502");
  });

  it("a directory that is deployed but NOT YET APPROVED is a definite refusal, not a non-answer", async () => {
    // The live state today: C-7 deployed the resolver, and `setResolverApproved` is registrar work
    // that has not happened. `isLiveFor` answering false is an ANSWER about the chain, so it is a
    // `fail` - which is a different thing from being unable to ask, and must stay different.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader({ directoryIsLiveFor: async () => false }),
      digest,
    );
    expect(plan.verdict).toBe("refused");
    expect(plan.checks.find((c) => c.id === "directory-resolver-live")!.outcome).toBe("fail");
  });
});
