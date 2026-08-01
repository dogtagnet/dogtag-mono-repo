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
import { buildProfileBlob, logoRef } from "../src/mirror";
import { mirrorPublicationRefusal } from "../src/provider";
import {
  MAX_SCANNED_LOCATION_NUMBERS,
  planDirectoryPublication,
  toContractCoordinate,
  ZERO_WORD,
  type Address,
  type DigestFn,
  type DirectoryPin,
  type HexWord,
  type ProviderChainReader,
} from "../src/provider";

const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const CALLER: Address = "0x2222222222222222222222222222222222222222";

/** A provider that has published contacts before, and one location at number 0. */
function withOnePin(pin: Partial<DirectoryPin> = {}): Partial<ProviderChainReader> {
  const stored: DirectoryPin = {
    locationNo: 0,
    lat: 1_290_270,
    lng: 103_851_959,
    kind: 1,
    active: true,
    ...pin,
  };
  return {
    providerPinCount: async () => 1,
    providerNextLocationNumber: async () => 1,
    providerHasPin: async (_p, no) => no === stored.locationNo,
    providerPin: async () => stored,
  };
}

// Takes TEXT or BYTES, exactly like the real `DigestFn`. A string-only double could not model a
// publication carrying a logo at all - the logo's address is digested from raw bytes - so it would
// have made every logo case unreachable rather than merely unasserted.
const digest: DigestFn = (content) => {
  const codes =
    typeof content === "string"
      ? Array.from(content).map((c) => c.charCodeAt(0))
      : Array.from(content);
  return `0x${codes.reduce((h, c) => (h * 31 + c) % 0xffffffff, 7).toString(16).padStart(64, "0")}` as HexWord;
};

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
    canWriteServiceRepoint: unscripted("canWriteServiceRepoint"),
    canWriteDomain: unscripted("canWriteDomain"),
    directoryIsLiveFor: async () => true,
    canWriteProviderRecord: async () => true,
    // A provider that has published nothing yet: no anchor, no pins. The `nextLocationNumber` and
    // per-pin reads are left UNSCRIPTED on purpose - a case that reaches them without saying so has
    // scanned when it should not have, and should fail loudly rather than silently.
    providerProfileAnchor: async () => ({
      digest: ZERO_WORD,
      schema: 0,
      codec: 0,
      hashAlgorithm: 0,
      revision: 0n,
    }),
    providerPinCount: async () => 0,
    providerNextLocationNumber: unscripted("providerNextLocationNumber"),
    providerHasPin: unscripted("providerHasPin"),
    providerPin: unscripted("providerPin"),
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
    // The profile document is mirrored, then anchored. Two steps, and the upload is first.
    expect(plan.steps.map((s) => s.kind)).toEqual(["mirrorUpload", "profileAnchor"]);
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
    // Order matters, and the reason changed with S-17: the anchor is the IRREVERSIBLE half, so
    // every mirror upload precedes it. An anchor naming content the mirror does not hold reads, to
    // every consumer, exactly like a provider who published nothing.
    expect(plan.steps.map((s) => s.kind)).toEqual([
      "mirrorUpload",
      "profileAnchor",
      "pin",
    ]);
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
    expect(plan.steps.find((s) => s.kind === "pin")).toMatchObject({ lat: 0, lng: 0 });
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

describe("the profile anchor carries every argument setProfileAnchor needs", () => {
  it("never emits a zero schema or hash algorithm - the contract reverts BadProfileAnchor on either", async () => {
    // These live in the STEP rather than at the call site so a second sender cannot invent different
    // values for the same blob, which is how one document acquires two on-chain descriptions of what
    // it is.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );
    const anchor = plan.steps.find((s) => s.kind === "profileAnchor");
    expect(anchor).toBeDefined();
    if (anchor?.kind !== "profileAnchor") throw new Error("unreachable");
    expect(anchor.schema).toBeGreaterThan(0);
    expect(anchor.hashAlgorithm).toBeGreaterThan(0);
    // keccak-256's multicodec code. Naming sha2-256 (0x12) here because it is the commoner constant
    // would be a false statement about which function produced the digest, and a verifier that
    // believed it would recompute the wrong hash and call a genuine blob altered.
    expect(anchor.hashAlgorithm).toBe(0x1b);
    // No contenthash while S-17 does not exist: publishing a location nothing serves would be a
    // worse claim than publishing none.
    expect(anchor.contenthash).toBe("0x");
    expect(anchor.digest).toBe(digest(anchor.blob));
  });
});

describe("the contact blob", () => {
  it("omits a blank channel rather than publishing an empty one", () => {
    const { blob, channelsPublished } = buildProfileBlob(CONTACTS, null);
    expect(channelsPublished).toBe(2);
    const parsed = JSON.parse(blob) as { contact: Record<string, string> };
    expect(Object.keys(parsed.contact)).toEqual(["phone", "email"]);
    expect(parsed.contact).not.toHaveProperty("whatsapp");
  });

  it("is stable for the same input, so re-publishing does not move the digest", () => {
    expect(buildProfileBlob(CONTACTS, null).blob).toBe(buildProfileBlob({ ...CONTACTS }, null).blob);
  });

  it("distinguishes an omitted channel from a published empty one", () => {
    // If these produced the same document the digest could not tell "I did not publish a website"
    // from "I published an empty website".
    const withWebsite = buildProfileBlob({ ...CONTACTS, website: "https://clinic.example.sg" }, null);
    expect(withWebsite.blob).not.toBe(buildProfileBlob(CONTACTS, null).blob);
    expect(withWebsite.channelsPublished).toBe(3);
  });

  it("REFUSES a provider publishing neither contacts nor a location, rather than only warning", async () => {
    // This used to report `canPublish: true` while its own next step said "add at least one so
    // people can find you" - a plan contradicting its own advice. An empty anchor is a published
    // emptiness and the listing sequence is append-only, so it is worth refusing rather than
    // recording.
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
    expect(plan.verdict).toBe("refused");
    expect(plan.canPublish).toBe(false);
    expect(plan.checks.find((c) => c.id === "directory-contents")!.outcome).toBe("fail");
    // The specific remedy is stated, rather than deferring to "the failed checks say why" - that
    // would send the provider to read a sentence this one already contains.
    expect(plan.nextStep).toMatch(/neither contact details nor a location/i);
  });
});

describe("re-publishing UPDATES the location it is correcting, and never adds a second", () => {
  it("rewrites the existing pin by its number instead of appending", async () => {
    // `publishPin` issues a FRESH location number every call, so a provider correcting a mistyped
    // coordinate by pressing Publish again would leave BOTH pins live and active in the scan - and
    // the mobile nearby list is built from exactly that scan, so they would appear at two places at
    // once. The contract has `updatePin`; the plan has to reach for it.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader(withOnePin()),
      digest,
    );

    const pins = plan.steps.filter((s) => s.kind === "pin");
    expect(pins).toHaveLength(1);
    expect(pins[0]).toMatchObject({
      op: "update",
      locationNo: 0,
      lat: 1_352_100,
      lng: 103_819_800,
    });
    expect(plan.canPublish).toBe(true);
  });

  it("emits `publish` only for a provider with NO published location", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader(),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "pin")[0]).toMatchObject({ op: "publish" });
  });

  it("sends NO pin transaction when the published location is already exactly this", async () => {
    // `updatePin` reverts `NoChange` on an identical word, so sending it would be an opaque failure
    // rather than a no-op.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.290270", lngInput: "103.851959" },
      reader(withOnePin()),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    // The contacts are new, so there is still something to send - this is not the nothing-to-do case.
    expect(plan.steps.filter((s) => s.kind === "profileAnchor")).toHaveLength(1);
    expect(plan.canPublish).toBe(true);
  });

  it("REFUSES rather than appending when it cannot tell which of several pins is meant", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader({ providerPinCount: async () => 3 }),
      digest,
    );
    expect(plan.verdict).toBe("refused");
    expect(plan.canPublish).toBe(false);
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    const state = plan.checks.find((c) => c.id === "directory-listing-state")!;
    expect(state.outcome).toBe("fail");
    expect(state.finding).toMatch(/two places at once/i);
  });

  it("that refusal is SCOPED to publishing a location - contacts can still be updated", async () => {
    // The over-broad version of the fix would strand a provider with several legacy pins, unable to
    // change its phone number ever again. A refusal in the other direction is still a refusal.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader({ providerPinCount: async () => 3 }),
      digest,
    );
    expect(plan.verdict).toBe("ready");
    expect(plan.canPublish).toBe(true);
    expect(plan.checks.find((c) => c.id === "directory-listing-state")!.outcome).toBe("pass");
  });

  it("an unreadable listing state publishes NO pin and is indeterminate, never an append", async () => {
    // Appending is only safe when we know there is nothing to replace. "We could not ask what
    // exists" cannot license a write whose safety depends on knowing.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader({
        providerPinCount: async () => {
          throw new Error("HTTP request failed: 502");
        },
      }),
      digest,
    );
    expect(plan.verdict).toBe("indeterminate");
    expect(plan.canPublish).toBe(false);
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    const state = plan.checks.find((c) => c.id === "directory-listing-state")!;
    expect(state.outcome).toBe("could-not-run");
    expect(state.couldNotRunReason).toContain("502");
    // And it is NOT reported as a contact-only publication: the provider gave a location.
    expect(plan.contactOnly).toBe(false);
  });

  it("abandons the scan past its bound rather than reporting 'no pin found', which would append", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader({
        providerPinCount: async () => 1,
        providerNextLocationNumber: async () => MAX_SCANNED_LOCATION_NUMBERS + 1,
      }),
      digest,
    );
    expect(plan.verdict).toBe("indeterminate");
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    expect(plan.checks.find((c) => c.id === "directory-listing-state")!.couldNotRunReason).toMatch(
      /past the 64 this page will scan/i,
    );
  });
});

describe("withdrawal is deliberate, and the anchor is not rewritten for nothing", () => {
  it("offers withdrawal only when exactly one pin was actually read", async () => {
    const withPin = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(withOnePin()),
      digest,
    );
    expect(withPin.canWithdrawPin).toBe(true);
    expect(withPin.listing?.onlyPin?.locationNo).toBe(0);

    const none = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );
    expect(none.canWithdrawPin).toBe(false);
  });

  it("does NOT offer withdrawal when this key's authority over the record was not established", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader({
        ...withOnePin(),
        canWriteProviderRecord: async () => {
          throw new Error("timeout");
        },
      }),
      digest,
    );
    expect(plan.canWithdrawPin).toBe(false);
  });

  it("a blank coordinate field is NOT a withdrawal - the published pin stays and is said to stay", async () => {
    // Reading a cleared field as "take my location down" would make a typo destructive.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(withOnePin()),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    expect(plan.checks.find((c) => c.id === "directory-listing-state")!.finding).toMatch(
      /does not withdraw/i,
    );
  });

  it("omits the anchor when the contacts on chain are already exactly these", async () => {
    // `setProfileAnchor` has NO `NoChange` guard and the contract is frozen, so a redundant write
    // bumps the anchor revision - and every bump makes `coversCurrentAddressText` false for any
    // registrar address confirmation the provider holds. The portal is the only place to avoid it.
    const { blob } = buildProfileBlob(CONTACTS, null);
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.3521", lngInput: "103.8198" },
      reader({
        ...withOnePin(),
        providerProfileAnchor: async () => ({
          digest: digest(blob),
          schema: 1,
          codec: 0,
          hashAlgorithm: 0x1b,
          revision: 4n,
        }),
      }),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "profileAnchor")).toHaveLength(0);
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(1);
  });

  it("still RE-PUBLISHES to the mirror when the chain is already right, and sends no transaction", async () => {
    // The repair path, and the exact sequence the review reported: the mirror is ephemeral, so a
    // restart empties it while the chain still holds the digest. This used to emit nothing at all
    // and report "nothing to send" about content it held the sole remaining copy of. Uploading is
    // idempotent by construction - same bytes, same address - so it is emitted unconditionally,
    // while the anchor transaction stays conditional because a needless one bumps the revision.
    const { blob } = buildProfileBlob(CONTACTS, null);
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.290270", lngInput: "103.851959" },
      reader({
        ...withOnePin(),
        providerProfileAnchor: async () => ({
          digest: digest(blob),
          schema: 1,
          codec: 0,
          hashAlgorithm: 0x1b,
          revision: 4n,
        }),
      }),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "profileAnchor")).toHaveLength(0);
    expect(plan.steps.filter((s) => s.kind === "pin")).toHaveLength(0);
    expect(plan.steps.filter((s) => s.kind === "mirrorUpload")).toHaveLength(1);
    // Offerable: an uploads-only publication is a real one, not "nothing to do".
    expect(plan.canPublish).toBe(true);
    expect(plan.checks.find((c) => c.id === "directory-contents")!.finding).toMatch(
      /send no transaction/i,
    );
  });

  it("re-publishes the LOGO too, not only the profile document - BOTH uploads are unconditional", async () => {
    // The gap the mutation gate found: with the logo upload alone re-gated on `anchorUnchanged`
    // every existing case still passed, because none of them published a logo AND had a matching
    // anchor. A provider whose logo the mirror has lost is exactly who needs the repair path.
    const logo = { bytes: new Uint8Array([9, 9, 9]), mediaType: "image/png" as const };
    const { blob } = buildProfileBlob(CONTACTS, logoRef(digest(logo.bytes), logo));
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.290270", lngInput: "103.851959", logo },
      reader({
        ...withOnePin(),
        providerProfileAnchor: async () => ({
          digest: digest(blob),
          schema: 1,
          codec: 0,
          hashAlgorithm: 0x1b,
          revision: 4n,
        }),
      }),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "profileAnchor")).toHaveLength(0);
    const uploads = plan.steps.filter((s) => s.kind === "mirrorUpload");
    expect(uploads.map((s) => (s.kind === "mirrorUpload" ? s.what : ""))).toEqual([
      "logo",
      "profile",
    ]);
    expect(plan.canPublish).toBe(true);
  });

  it("describes an uploads-only publication honestly rather than as '0 transactions: .'", async () => {
    const { blob } = buildProfileBlob(CONTACTS, null);
    const plan = await planDirectoryPublication(
      { ...base, latInput: "1.290270", lngInput: "103.851959" },
      reader({
        ...withOnePin(),
        providerProfileAnchor: async () => ({
          digest: digest(blob),
          schema: 1,
          codec: 0,
          hashAlgorithm: 0x1b,
          revision: 4n,
        }),
      }),
      digest,
    );
    const finding = plan.checks.find((c) => c.id === "directory-contents")!.finding;
    expect(finding).not.toMatch(/0 transactions/);
    expect(finding).toMatch(/re-published to the content mirror/i);
  });

  it("still sends the anchor when the listing state could not be read - that is what was asked for", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader({
        providerProfileAnchor: async () => {
          throw new Error("HTTP request failed: 502");
        },
      }),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "profileAnchor")).toHaveLength(1);
    // But it is not offered: the state is unknown, and unknown is not permission.
    expect(plan.canPublish).toBe(false);
    expect(plan.verdict).toBe("indeterminate");
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

describe("the plan is answerable about what it judged", () => {
  it("carries the provider id it was computed for, so a send addresses THAT", async () => {
    // The other half of "a send acts on what was checked": the component invalidates a plan when its
    // inputs change AND addresses the plan's own captured values, so the plan has to carry them.
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "" },
      reader(),
      digest,
    );
    expect(plan.providerId).toBe(PROVIDER);
  });
});

describe("the mirror settings are checked BEFORE the first step, not during the loop", () => {
  const uploadStep = {
    kind: "mirrorUpload" as const,
    what: "profile" as const,
    address: "0x11" as `0x${string}`,
    bytes: new Uint8Array([1]),
    mediaType: "application/json",
  };
  const pinStep = {
    kind: "pin" as const,
    op: "publish" as const,
    lat: 1,
    lng: 2,
    locationKind: 1,
    active: true,
  };

  it("names the missing base rather than letting the first upload throw mid-sequence", () => {
    const reason = mirrorPublicationRefusal([uploadStep], "", "tok");
    expect(reason).toMatch(/VITE_CONTENT_MIRROR_BASE/);
  });

  it("names the missing TOKEN too - the case that used to surface as 'missing bearer token'", () => {
    // Aborting inside the loop left the anchor unsent and told the operator nothing actionable.
    const reason = mirrorPublicationRefusal([uploadStep], "http://mirror", "");
    expect(reason).toMatch(/VITE_CONTENT_MIRROR_TOKEN/);
  });

  it("does not refuse a checked plan that publishes no content at all", () => {
    // A pin-only correction needs neither setting, so refusing it would be over-broad.
    expect(mirrorPublicationRefusal([pinStep], "", "")).toBeNull();
  });

  it("answers for an UNCHECKED form too, because every publication uploads its profile document", () => {
    // What lets the surface state a configuration problem before the provider fills the form in,
    // rather than after they press Publish.
    expect(mirrorPublicationRefusal(undefined, "", "")).toMatch(/VITE_CONTENT_MIRROR_BASE/);
    expect(mirrorPublicationRefusal(undefined, "http://mirror", "tok")).toBeNull();
  });

  it("is silent when both are configured", () => {
    expect(mirrorPublicationRefusal([uploadStep], "http://mirror", "tok")).toBeNull();
  });
});

describe("a logo the provider chose is validated with a VISIBLE reason, never dropped", () => {
  it("publishes a logo under its own address, uploaded BEFORE the anchor", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "", logo: { bytes: new Uint8Array([1, 2, 3]), mediaType: "image/png" } },
      reader({}),
      digest,
    );
    const kinds = plan.steps.map((s) => s.kind);
    expect(kinds.indexOf("mirrorUpload")).toBeLessThan(kinds.indexOf("profileAnchor"));
    const uploads = plan.steps.filter((s) => s.kind === "mirrorUpload");
    expect(uploads).toHaveLength(2);
    expect(uploads[0]).toMatchObject({ what: "logo", mediaType: "image/png" });
  });

  it("moves the anchor digest, so a logo swap cannot ride an already-checked plan", async () => {
    const withA = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "", logo: { bytes: new Uint8Array([1]), mediaType: "image/png" } },
      reader({}),
      digest,
    );
    const withB = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "", logo: { bytes: new Uint8Array([2]), mediaType: "image/png" } },
      reader({}),
      digest,
    );
    const anchorOf = (p: typeof withA) =>
      p.steps.find((s) => s.kind === "profileAnchor") as { digest: string };
    expect(anchorOf(withA).digest).not.toBe(anchorOf(withB).digest);
  });

  it("no logo is ORDINARY: no upload for one, and nothing reads as a failure", async () => {
    const plan = await planDirectoryPublication(
      { ...base, latInput: "", lngInput: "", logo: null },
      reader({}),
      digest,
    );
    expect(plan.steps.filter((s) => s.kind === "mirrorUpload")).toHaveLength(1);
    expect(plan.checks.every((c) => c.outcome !== "fail")).toBe(true);
  });
});
