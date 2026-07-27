import { describe, expect, it } from "vitest";
import {
  addressExplorerHref,
  chainProvenance,
  emittingCloneName,
  emittingContractRole,
  eventDetailFields,
  formatChainTime,
  formatRelativeTime,
  isEvmAddress,
  isHash32,
  isOpaqueIdentifier,
  joinedDetailContext,
  shortHex,
  shortValue,
  txExplorerHref,
} from "../src/chain/provenance";

/** A well-formed 32-byte transaction hash, as a real ROAX transaction would carry. */
const REAL_TX = `0x${"ab".repeat(32)}`;
/** What the indexer's scripted demo feed actually emits (`INDEXER_DEMO_MODE` MemLogSource). */
const DEMO_TX = "0x0800";

describe("chain-addressability", () => {
  it("accepts only a 0x-prefixed 32-byte hex value as a hash", () => {
    expect(isHash32(REAL_TX)).toBe(true);
    expect(isHash32(REAL_TX.toUpperCase().replace("0X", "0x"))).toBe(true);
    expect(isHash32(DEMO_TX)).toBe(false);
    expect(isHash32(`0x${"ab".repeat(31)}`)).toBe(false); // 31 bytes
    expect(isHash32(`0x${"ab".repeat(33)}`)).toBe(false); // 33 bytes
    expect(isHash32(`0x${"zz".repeat(32)}`)).toBe(false); // not hex
    expect(isHash32("ab".repeat(32))).toBe(false); // missing 0x
    expect(isHash32(undefined)).toBe(false);
  });

  it("accepts only a 0x-prefixed 20-byte hex value as an address", () => {
    expect(isEvmAddress("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F")).toBe(true);
    expect(isEvmAddress(REAL_TX)).toBe(false); // a hash is not an address
    expect(isEvmAddress("0x0800")).toBe(false);
    expect(isEvmAddress(null)).toBe(false);
  });
});

describe("chainProvenance", () => {
  it("calls a well-formed transaction hash on-chain", () => {
    expect(chainProvenance({ txHash: REAL_TX })).toBe("onchain");
  });

  it("calls the demo indexer's scripted hashes synthetic", () => {
    expect(chainProvenance({ txHash: DEMO_TX })).toBe("synthetic");
    expect(chainProvenance({ txHash: "0xgovtx1111" })).toBe("synthetic");
    expect(chainProvenance({})).toBe("synthetic");
  });
});

describe("txExplorerHref", () => {
  it("prefers the indexer-supplied txUrl for a real transaction", () => {
    const href = txExplorerHref({ txHash: REAL_TX, txUrl: "https://other.example/tx/x" });
    expect(href).toBe("https://other.example/tx/x");
  });

  it("composes a ROAX explorer URL only when the API sent none", () => {
    expect(txExplorerHref({ txHash: REAL_TX })).toBe(`https://explorer.roax.net/tx/${REAL_TX}`);
  });

  it("refuses to link a synthetic event even when the API supplied a txUrl", () => {
    // This is the live failure: the indexer composes txUrl unconditionally, so demo rows arrive
    // carrying an explorer link to a transaction that cannot exist.
    expect(
      txExplorerHref({ txHash: DEMO_TX, txUrl: "https://explorer.roax.net/tx/0x0800" }),
    ).toBeNull();
    expect(txExplorerHref({})).toBeNull();
  });
});

describe("addressExplorerHref", () => {
  it("links a well-formed address and refuses anything else", () => {
    expect(addressExplorerHref("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F")).toBe(
      "https://explorer.roax.net/address/0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F",
    );
    expect(addressExplorerHref("0x08")).toBeNull();
    expect(addressExplorerHref(undefined)).toBeNull();
  });
});

describe("shortHex", () => {
  it("middle-truncates long values and leaves short ones intact", () => {
    expect(shortHex(REAL_TX)).toBe(`0xabababab…ababab`);
    expect(shortHex("0x0800")).toBe("0x0800");
    expect(shortHex(undefined)).toBe("—");
  });
});

describe("isOpaqueIdentifier", () => {
  it("accepts 0x-hex and all-digit field elements", () => {
    expect(isOpaqueIdentifier(REAL_TX)).toBe(true);
    expect(isOpaqueIdentifier("0x0800")).toBe(true);
    expect(isOpaqueIdentifier("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F")).toBe(true);
    // The indexer renders the on-chain dogTagId as a decimal U256.
    expect(isOpaqueIdentifier("12345678901234567890")).toBe(true);
  });

  it("rejects human text, however long", () => {
    expect(isOpaqueIdentifier("TRAVEL_CLEARANCE")).toBe(false);
    expect(isOpaqueIdentifier("SERVICE_ATTESTATION")).toBe(false);
    expect(isOpaqueIdentifier("grooming_checkin")).toBe(false);
    expect(isOpaqueIdentifier("DogTag Government Authority")).toBe(false);
    expect(isOpaqueIdentifier(undefined)).toBe(false);
  });
});

describe("shortValue", () => {
  // The dense table cells render at head=8, which is where blind middle-truncation did its damage.
  const HEAD = 8;

  it("middle-truncates an opaque identifier exactly as before", () => {
    expect(shortValue(`0x${"33".repeat(32)}`, HEAD)).toBe("0x333333…333333");
    expect(shortValue("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F", HEAD)).toBe("0xB5D665…6c5F9F");
    // A decimal dogTagId is opaque too - head and tail identify it, the middle is noise.
    expect(shortValue("12345678901234567890", HEAD)).toBe("12345678…567890");
    expect(shortValue(REAL_TX)).toBe("0xabababab…ababab");
  });

  it("NEVER middle-truncates a human label - the middle is where the meaning is", () => {
    // The regression this guards: at head=8 these rendered as "TRAVEL_C…ARANCE" and
    // "SERVICE_…TATION", which read as corruption on the cell that names the kind of record.
    expect(shortValue("TRAVEL_CLEARANCE", HEAD)).toBe("TRAVEL_CLEARANCE");
    expect(shortValue("SERVICE_ATTESTATION", HEAD)).toBe("SERVICE_ATTESTATION");
    expect(shortValue("DogTag Government Authority", HEAD)).toBe("DogTag Government Authority");
    for (const label of ["TRAVEL_CLEARANCE", "SERVICE_ATTESTATION", "DogTag Government Authority"]) {
      expect(shortValue(label, HEAD)).not.toContain("…");
    }
  });

  it("leaves short values and absent ones alone", () => {
    expect(shortValue("VACCINATION", HEAD)).toBe("VACCINATION");
    expect(shortValue("0x0800", HEAD)).toBe("0x0800");
    expect(shortValue(undefined)).toBe("—");
  });
});

describe("time formatting", () => {
  it("renders an absolute timestamp and an em dash when absent", () => {
    expect(formatChainTime(0)).toBe("—");
    expect(formatChainTime(undefined)).toBe("—");
    expect(formatChainTime(1_700_000_000)).toContain("2023");
  });

  it("renders a coarse relative distance in both directions", () => {
    const now = 1_700_000_000_000;
    expect(formatRelativeTime(1_700_000_000 - 30, now)).toBe("30s ago");
    expect(formatRelativeTime(1_700_000_000 - 600, now)).toBe("10m ago");
    expect(formatRelativeTime(1_700_000_000 - 7200, now)).toBe("2h ago");
    expect(formatRelativeTime(1_700_000_000 - 172_800, now)).toBe("2d ago");
    expect(formatRelativeTime(1_700_000_000 + 3600, now)).toBe("in 1h");
    expect(formatRelativeTime(undefined, now)).toBe("");
  });
});

describe("emittingContractRole", () => {
  it("names the contract that actually emitted each event kind", () => {
    // The emitting contract is NOT always the issuer clone - that distinction is the whole point.
    expect(emittingContractRole("issuerCreated")).toBe("issuer factory");
    expect(emittingContractRole("whitelisted")).toBe("issuer registry");
    expect(emittingContractRole("delisted")).toBe("issuer registry");
    expect(emittingContractRole("verified")).toBe("verification registry");
    expect(emittingContractRole("rootIssued")).toBe("issuer clone");
    expect(emittingContractRole("rootRevoked")).toBe("issuer clone");
    expect(emittingContractRole("somethingNew")).toBe("contract");
  });

  it("attributes rootRegistered to the factory, which is what emits it", () => {
    // `stacks/indexer/api/src/chain.rs` decodes RootRegistered ONLY when the emitting address equals
    // ctx.factory, so `ev.contract` on such a row is the factory - calling it a clone is a false
    // answer to "which smart contract" on an audit surface.
    expect(emittingContractRole("rootRegistered")).toBe("issuer factory");
  });
});

describe("emittingCloneName", () => {
  const CLONE = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F";
  const FACTORY = "0xED20269E1234567890abcdefABCDEF1234567890";

  it("names the clone when the clone is what emitted the event", () => {
    expect(
      emittingCloneName({ contract: CLONE, clone: CLONE, cloneName: "DogTag Government Authority" }),
    ).toBe("DogTag Government Authority");
    // The indexer lower-cases addresses; a checksummed twin must still match.
    expect(
      emittingCloneName({ contract: CLONE.toLowerCase(), clone: CLONE, cloneName: "Clinic A" }),
    ).toBe("Clinic A");
  });

  it("withholds the clone name when the FACTORY emitted the event", () => {
    // issuerCreated/rootRegistered: `clone` is the clone acted ON, so printing its name under the
    // emitting address would present the factory as that clone.
    expect(
      emittingCloneName({ contract: FACTORY, clone: CLONE, cloneName: "DogTag Government Authority" }),
    ).toBeNull();
  });

  it("returns null when there is no name or no clone to compare against", () => {
    expect(emittingCloneName({ contract: CLONE, clone: CLONE })).toBeNull();
    expect(emittingCloneName({ contract: CLONE, cloneName: "Clinic A" })).toBeNull();
    expect(emittingCloneName({ clone: CLONE, cloneName: "Clinic A" })).toBeNull();
  });
});

describe("eventDetailFields", () => {
  const RT_KEY = `0x${"cd".repeat(32)}`;
  const verified = {
    type: "verified",
    dogTagId: "12345678901234567890",
    purpose: `0x${"ef".repeat(32)}`,
    nullifier: `0x${"11".repeat(32)}`,
  };

  it("labels a record type by the SHAPE of the value, key vs human label", () => {
    expect(eventDetailFields({ type: "rootIssued", recordType: RT_KEY })[0].label).toBe(
      "record type key",
    );
    expect(
      eventDetailFields({ type: "rootIssued", recordType: "TRAVEL_CLEARANCE" })[0].label,
    ).toBe("record type");
  });

  it("prefers the joined session's readable purpose over the hashed key", () => {
    const joined = eventDetailFields(verified, { purpose: "grooming_checkin" });
    expect(joined).toContainEqual({ label: "purpose", value: "grooming_checkin" });
    // Without a join there is only the key, and it is labelled as one.
    expect(eventDetailFields(verified)).toContainEqual({
      label: "purpose key",
      value: verified.purpose,
    });
  });

  it("falls back to the joined record type, since a verified event carries none", () => {
    // The owner-hidden `Verified` event binds no recordType - the local join is the only source.
    expect(eventDetailFields(verified, { recordType: "VACCINATION" })).toContainEqual({
      label: "record type",
      value: "VACCINATION",
    });
    expect(eventDetailFields(verified).find((f) => f.label.startsWith("record type"))?.value).toBe(
      null,
    );
  });

  it("suppresses the field-hashed dog tag id when the row already names that tag", () => {
    // Showing both renders ONE tag two ways: the readable handle in the join cell and its field hash
    // here. The nullifier and purpose still identify the event.
    const labels = eventDetailFields(verified, { dogTagNamedElsewhere: true }).map((f) => f.label);
    expect(labels).not.toContain("dog tag id");
    expect(eventDetailFields(verified).map((f) => f.label)).toContain("dog tag id");
  });

  it("never lets a joined record type override the chain's own", () => {
    // The guard below may only ever suppress a LOCALLY INFERRED record type. A chain-attested one
    // outranks any join, so withholding can never hide what the event itself asserts.
    expect(
      eventDetailFields(
        { type: "whitelisted", recordType: RT_KEY },
        { recordType: "SOMETHING_ELSE" },
      ),
    ).toContainEqual({ label: "record type key", value: RT_KEY });
  });
});

describe("joinedDetailContext", () => {
  it("passes a root/tx-joined record type through, since that join is an exact match", () => {
    // The chain's RootIssued/RootRevoked logs carry no recordType at all, so the operator's own
    // root-joined record is the ONLY source - and it describes this very event.
    expect(joinedDetailContext({ kind: "issuance", recordType: "TRAVEL_CLEARANCE" })).toEqual({
      purpose: undefined,
      recordType: "TRAVEL_CLEARANCE",
      dogTagNamedElsewhere: false,
    });
  });

  it("withholds the record type of a TAG-granular join", () => {
    // `joinedBy: "dogTagId"` proves only that the tag is one this operator credentialed. The
    // owner-hidden Verified event binds no root, so WHICH credential was verified is unknowable -
    // lending it this credential's record type would assert exactly that unknown.
    expect(
      joinedDetailContext({ recordType: "TRAVEL_CLEARANCE", joinedBy: "dogTagId" }),
    ).toEqual({ purpose: undefined, recordType: null, dogTagNamedElsewhere: true });
  });

  it("keeps a readable joined purpose, which the owner-blind chain payload never carries", () => {
    expect(joinedDetailContext({ purpose: "grooming_checkin" }).purpose).toBe("grooming_checkin");
  });

  it("yields an empty context for an unjoined event", () => {
    expect(joinedDetailContext(null)).toEqual({});
    expect(joinedDetailContext(undefined)).toEqual({});
  });

  it("drives both consoles to the same fields for one event", () => {
    // The defect this replaces: government and vet rendered the SAME rootIssued event differently
    // because each assembled the context itself. One helper, one answer.
    const rootIssued = { type: "rootIssued", root: `0x${"33".repeat(32)}` };
    const govLocal = { kind: "issuance", recordType: "TRAVEL_CLEARANCE", dogTagId: "7" };
    const vetLocal = { kind: "issuance", recordType: "TRAVEL_CLEARANCE", recordId: "rec-7" };
    expect(eventDetailFields(rootIssued, joinedDetailContext(govLocal))).toEqual(
      eventDetailFields(rootIssued, joinedDetailContext(vetLocal)),
    );
  });
});
