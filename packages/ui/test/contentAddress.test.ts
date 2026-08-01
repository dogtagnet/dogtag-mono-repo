// The content-addressing primitive and the resolution chain (registry-plan S-17).
//
// The rule these pin: **verification is RECOMPUTATION**, so where the bytes came from is not
// evidence about them. A mirror that serves them, a CDN that caches them and a provider that hosts
// them have exactly the same standing, which is none.
//
// The case that decides whether any of this is real is `a valid but WRONG image at the requested
// address`. A merely-corrupt image would be refused by the browser's own decoder, so a test built on
// one passes whether the address check works or not; only a perfectly renderable substitution
// distinguishes a working check from a broken one.

import { keccak256, toHex } from "viem";
import { describe, expect, it } from "vitest";

import {
  buildProfileBlob,
  isContentAddress,
  MULTIHASH_KECCAK_256,
  namesContent,
  parseProfileBlob,
  resolveProviderProfile,
  verifyContentAddress,
  ZERO_ADDRESS,
  type ContentAddress,
  type FetchContentFn,
  type MirrorFetch,
  type ProfileAnchorRef,
} from "../src/mirror";
import { blankContactFields } from "../src/directory/registration";

const digest = (bytes: Uint8Array) => keccak256(bytes);
const utf8 = (text: string) => new TextEncoder().encode(text);
const addressOf = (bytes: Uint8Array): ContentAddress => keccak256(bytes) as ContentAddress;

/** A byte string that is not an image but stands in for one; the resolver never decodes it. */
const REAL_LOGO = utf8("PNG\r\n\n the genuine clinic logo");
const OTHER_LOGO = utf8("PNG\r\n\n a completely valid but DIFFERENT logo");

function mirror(objects: Record<string, { bytes: Uint8Array; mediaType: string }>): FetchContentFn {
  return async (address) => {
    const found = objects[address.toLowerCase()];
    return found
      ? ({ state: "found", bytes: found.bytes, mediaType: found.mediaType } satisfies MirrorFetch)
      : ({ state: "absent" } satisfies MirrorFetch);
  };
}

const CONTACTS = { ...blankContactFields(), phone: "+65 6123 4567" };

function publish(logo: { bytes: Uint8Array; mediaType: "image/png" } | null) {
  const logoAddress = logo ? addressOf(logo.bytes) : null;
  const { blob } = buildProfileBlob(
    CONTACTS,
    logo && logoAddress
      ? {
          digest: logoAddress,
          hashAlgorithm: MULTIHASH_KECCAK_256,
          mediaType: logo.mediaType,
          byteLength: logo.bytes.length,
        }
      : null,
  );
  const blobBytes = utf8(blob);
  const objects: Record<string, { bytes: Uint8Array; mediaType: string }> = {
    [addressOf(blobBytes)]: { bytes: blobBytes, mediaType: "application/json" },
  };
  if (logo && logoAddress) objects[logoAddress] = { bytes: logo.bytes, mediaType: logo.mediaType };
  return { anchorDigest: addressOf(blobBytes), objects, blobBytes };
}

function anchor(digestValue: string, revision = 1n): ProfileAnchorRef {
  return { digest: digestValue, hashAlgorithm: MULTIHASH_KECCAK_256, revision };
}

describe("the content address IS the hash", () => {
  it("verifies bytes that hash to the address they were requested under", () => {
    const address = addressOf(REAL_LOGO);
    expect(verifyContentAddress(REAL_LOGO, address, MULTIHASH_KECCAK_256, digest)).toEqual({
      state: "verified",
      address,
    });
  });

  it("refuses bytes that do not, and says what they actually were", () => {
    const requested = addressOf(REAL_LOGO);
    const result = verifyContentAddress(OTHER_LOGO, requested, MULTIHASH_KECCAK_256, digest);
    expect(result.state).toBe("mismatch");
    expect(result).toMatchObject({ requested, computed: addressOf(OTHER_LOGO) });
  });

  it("accepts an uppercase address, because hex case is a wire difference and not an attack", () => {
    const lower = addressOf(REAL_LOGO);
    const upper = `0x${lower.slice(2).toUpperCase()}`;
    expect(verifyContentAddress(REAL_LOGO, upper, MULTIHASH_KECCAK_256, digest).state).toBe(
      "verified",
    );
  });

  it("READS the hash algorithm rather than assuming keccak", () => {
    // Recomputing with the wrong function would report a genuine blob as altered - the exact failure
    // `directoryPlan.ts` already names for the publish side.
    const address = addressOf(REAL_LOGO);
    const result = verifyContentAddress(REAL_LOGO, address, 0x12 /* sha2-256 */, digest);
    expect(result).toEqual({ state: "unsupportedAlgorithm", hashAlgorithm: 0x12 });
  });

  it("treats the zero word as an absence, never as a content address", () => {
    expect(isContentAddress(ZERO_ADDRESS)).toBe(true);
    expect(namesContent(ZERO_ADDRESS)).toBe(false);
    expect(namesContent(addressOf(REAL_LOGO))).toBe(true);
  });
});

describe("the profile blob round-trips exactly", () => {
  it("builds a document the parser reads back", () => {
    const { blob } = buildProfileBlob(CONTACTS, null);
    const parsed = parseProfileBlob(blob);
    expect(parsed.ok).toBe(true);
    expect(parsed.ok && parsed.profile.contact.phone).toBe("+65 6123 4567");
    expect(parsed.ok && parsed.profile.logo).toBeNull();
  });

  it("refuses a document whose logo entry is malformed rather than reporting no logo", () => {
    // Degrading to `logo: null` would render as "this provider published no logo", which is a false
    // statement about a provider that published one. The honest answer is that it could not be read.
    const parsed = parseProfileBlob(
      JSON.stringify({
        schema: "dogtag/provider-profile/2",
        contact: {},
        logo: { digest: "0xnope", hashAlgorithm: 27, mediaType: "image/png", byteLength: 1 },
      }),
    );
    expect(parsed.ok).toBe(false);
  });

  it("refuses an SVG logo entry, because SVG can carry script and this renders in a portal", () => {
    const parsed = parseProfileBlob(
      JSON.stringify({
        schema: "dogtag/provider-profile/2",
        contact: {},
        logo: {
          digest: addressOf(REAL_LOGO),
          hashAlgorithm: 27,
          mediaType: "image/svg+xml",
          byteLength: 1,
        },
      }),
    );
    expect(parsed.ok).toBe(false);
  });

  it("refuses a document declaring a schema this build cannot read", () => {
    const parsed = parseProfileBlob(
      JSON.stringify({ schema: "dogtag/provider-contact/1", contact: {} }),
    );
    expect(parsed.ok).toBe(false);
  });
});

describe("resolving a provider profile through the mirror", () => {
  it("resolves the logo when every link in the chain verifies", async () => {
    const { anchorDigest, objects } = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    const resolution = await resolveProviderProfile(anchor(anchorDigest), mirror(objects), digest);

    expect(resolution.state).toBe("resolved");
    expect(resolution.state === "resolved" && resolution.logo.state).toBe("verified");
    expect(resolution.state === "resolved" && resolution.profile.contact.phone).toBe(
      "+65 6123 4567",
    );
  });

  it("reports notPublished - never a failure - when the provider published no logo", async () => {
    const { anchorDigest, objects } = publish(null);
    const resolution = await resolveProviderProfile(anchor(anchorDigest), mirror(objects), digest);

    expect(resolution.state).toBe("resolved");
    expect(resolution.state === "resolved" && resolution.logo).toEqual({ state: "notPublished" });
  });

  it("REFUSES a valid but WRONG image served at the published address", async () => {
    // The discriminator. A corrupt image would be dropped by the browser's decoder whether or not
    // the address check works; only a perfectly renderable substitution can tell the two apart.
    const { anchorDigest, objects } = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    const logoAddress = addressOf(REAL_LOGO);
    objects[logoAddress] = { bytes: OTHER_LOGO, mediaType: "image/png" };

    const resolution = await resolveProviderProfile(anchor(anchorDigest), mirror(objects), digest);
    expect(resolution.state).toBe("resolved");
    expect(resolution.state === "resolved" && resolution.logo.state).toBe("unverified");
    expect(
      resolution.state === "resolved" &&
        resolution.logo.state === "unverified" &&
        resolution.logo.reason,
    ).toContain("does not match the address");
  });

  it("reports unverified - never absent - when the mirror cannot be reached", async () => {
    const { anchorDigest } = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    const unavailable: FetchContentFn = async () => ({
      state: "unavailable",
      reason: "mirror responded 503",
    });

    const resolution = await resolveProviderProfile(anchor(anchorDigest), unavailable, digest);
    expect(resolution.state).toBe("unverified");
    expect(resolution.state === "unverified" && resolution.logo.state).toBe("unverified");
  });

  it("reports unverified when a seam THROWS rather than resolving", async () => {
    const { anchorDigest } = publish(null);
    const throwing: FetchContentFn = async () => {
      throw new Error("connection reset");
    };
    const resolution = await resolveProviderProfile(anchor(anchorDigest), throwing, digest);
    expect(resolution.state).toBe("unverified");
    expect(resolution.state === "unverified" && resolution.reason).toContain("connection reset");
  });

  it("does not report contacts as published facts when the blob itself failed to verify", async () => {
    // A broken blob poisons the logo AND the contacts: the logo address is a field of a document
    // whose integrity is unestablished, and so are the contact strings beside it.
    const { anchorDigest } = publish(null);
    const substituted: FetchContentFn = async () => ({
      state: "found",
      bytes: utf8('{"schema":"dogtag/provider-profile/2","contact":{"phone":"+65 9999 9999"}}'),
      mediaType: "application/json",
    });

    const resolution = await resolveProviderProfile(anchor(anchorDigest), substituted, digest);
    expect(resolution.state).toBe("unverified");
    expect(resolution).not.toHaveProperty("profile");
  });

  it("refuses a blob that VERIFIES but cannot be parsed, and returns no contacts", async () => {
    // A distinct branch from the mismatch case above, and it was reachable only by a document whose
    // address is CORRECT: an old-schema blob a provider published before this build existed, or a
    // truncated one. Verification passing does not make an unreadable document readable, and the
    // contacts inside it are not published facts just because the bytes are the published bytes.
    const stale = utf8(JSON.stringify({ schema: "dogtag/provider-contact/1", contact: {} }));
    const staleAddress = addressOf(stale);
    const resolution = await resolveProviderProfile(
      anchor(staleAddress),
      mirror({ [staleAddress]: { bytes: stale, mediaType: "application/json" } }),
      digest,
    );

    expect(resolution.state).toBe("unverified");
    expect(resolution).not.toHaveProperty("profile");
    expect(resolution.state === "unverified" && resolution.logo.state).toBe("unverified");
    expect(resolution.state === "unverified" && resolution.reason).toContain("schema");
  });

  it("refuses a blob that verifies but is not valid UTF-8", async () => {
    // Reached the same way: correct address, unreadable content. `TextDecoder` is `fatal: true` so
    // this surfaces as a refusal rather than as replacement characters silently parsed as JSON.
    const invalid = new Uint8Array([0xff, 0xfe, 0xfd, 0xfc]);
    const address = addressOf(invalid);
    const resolution = await resolveProviderProfile(
      anchor(address),
      mirror({ [address]: { bytes: invalid, mediaType: "application/json" } }),
      digest,
    );

    expect(resolution.state).toBe("unverified");
    expect(resolution.state === "unverified" && resolution.reason).toContain("UTF-8");
  });

  it("tells never-published from withdrawn, because the revision advances on a clear", async () => {
    const never = await resolveProviderProfile(anchor(ZERO_ADDRESS, 0n), mirror({}), digest);
    expect(never).toEqual({ state: "notPublished", withdrawn: false });

    const withdrawn = await resolveProviderProfile(anchor(ZERO_ADDRESS, 4n), mirror({}), digest);
    expect(withdrawn).toEqual({ state: "notPublished", withdrawn: true });
  });

  it("reports unverified when the mirror holds nothing at an anchored address", async () => {
    // The chain says something is published, so this is NOT the notPublished case. It is a failure
    // to reach what was published, and a portal must not render it as "no logo".
    const { anchorDigest } = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    const resolution = await resolveProviderProfile(anchor(anchorDigest), mirror({}), digest);
    expect(resolution.state).toBe("unverified");
    expect(resolution.state === "unverified" && resolution.reason).toContain("holds no");
  });

  it("refuses a logo served as a type other than the one the provider published", async () => {
    // A correct hash proves these are the published bytes; it proves nothing about what they ARE.
    const { anchorDigest, objects } = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    objects[addressOf(REAL_LOGO)] = { bytes: REAL_LOGO, mediaType: "image/svg+xml" };

    const resolution = await resolveProviderProfile(anchor(anchorDigest), mirror(objects), digest);
    expect(resolution.state === "resolved" && resolution.logo.state).toBe("unverified");
  });

  it("carries a reason if and ONLY if the logo state is unverified", async () => {
    const verified = await resolveProviderProfile(
      anchor(publish({ bytes: REAL_LOGO, mediaType: "image/png" }).anchorDigest),
      mirror(publish({ bytes: REAL_LOGO, mediaType: "image/png" }).objects),
      digest,
    );
    expect(verified.state === "resolved" && verified.logo).not.toHaveProperty("reason");

    const absent = await resolveProviderProfile(
      anchor(publish(null).anchorDigest),
      mirror(publish(null).objects),
      digest,
    );
    expect(absent.state === "resolved" && absent.logo).toEqual({ state: "notPublished" });
  });
});

describe("the blob's digest covers the logo address", () => {
  it("moves the anchor digest when the logo changes, so the chain commits to both", async () => {
    // This is the chain of custody in one assertion: chain anchor -> blob -> logo address. If the
    // logo address were outside the blob, a logo could be swapped with the anchor unmoved.
    const withReal = publish({ bytes: REAL_LOGO, mediaType: "image/png" });
    const withOther = publish({ bytes: OTHER_LOGO, mediaType: "image/png" });
    expect(withReal.anchorDigest).not.toBe(withOther.anchorDigest);

    // And a blob committing to one logo cannot be resolved into the other, even served correctly.
    const swapped = { ...withReal.objects };
    swapped[addressOf(OTHER_LOGO)] = { bytes: OTHER_LOGO, mediaType: "image/png" };
    delete swapped[addressOf(REAL_LOGO)];

    const resolution = await resolveProviderProfile(
      anchor(withReal.anchorDigest),
      mirror(swapped),
      digest,
    );
    expect(resolution.state === "resolved" && resolution.logo.state).toBe("unverified");
  });
});

describe("the publication digest hashes text as its UTF-8 bytes", () => {
  it("agrees with what the mirror hashes when the same blob arrives as a request body", async () => {
    // A disagreement here would make every publication refuse itself: the client would compute one
    // address and the mirror another over the identical bytes.
    const { blob } = buildProfileBlob(CONTACTS, null);
    const { publicationDigest } = await import("../src/mirror");
    expect(publicationDigest(blob)).toBe(keccak256(utf8(blob)));
    expect(publicationDigest(blob)).toBe(keccak256(toHex(blob)));
  });
});
