/**
 * Resolve a provider's anchored profile, and its logo, through the content-addressed mirror.
 *
 * The chain of custody, and every link is checked:
 *
 *   chain `ProfileAnchor` → blob bytes fetched from the mirror → RECOMPUTED against the anchored
 *   digest → parsed → the logo's own address (covered by that same verification) → logo bytes
 *   fetched → RECOMPUTED against that address.
 *
 * A link that does not verify stops the chain. In particular a blob that fails verification does not
 * merely cost the logo: its contacts are not published facts either, and this module refuses to
 * return them. The mirror is what makes a contact-only provider contactable at all, so serving
 * unverified contacts would put the whole point of it back at risk to save one branch.
 *
 * Pure but for the two injected seams (`fetchContent`, `digest`), so the whole ladder is testable
 * with no network and no DOM.
 */

import {
  namesContent,
  verifyContentAddress,
  ZERO_ADDRESS,
  type BytesDigestFn,
  type ContentAddress,
} from "./contentAddress";
import {
  parseProfileBlob,
  type ProfileLogoRef,
  type ProviderProfile,
} from "./profileBlob";

/** The `ProfileAnchor` fields a resolution needs. Read from chain by the caller. */
export interface ProfileAnchorRef {
  /** `ProfileAnchor.digest`. The zero word means nothing is anchored. */
  readonly digest: string;
  readonly hashAlgorithm: number;
  /** `ProfileAnchor.revision`. `0n` means never published; higher with a zero digest is withdrawn. */
  readonly revision: bigint;
}

/** What one mirror read established. Mirrors the three outcomes the mirror route answers. */
export type MirrorFetch =
  | { readonly state: "found"; readonly bytes: Uint8Array; readonly mediaType: string }
  /** 404 — the mirror holds nothing here. A FACT about its contents. */
  | { readonly state: "absent" }
  /** Anything else — 503, a network failure, a timeout. NEVER an absence. */
  | { readonly state: "unavailable"; readonly reason: string };

/** Fetch one address from the mirror. Resolves rather than throwing; a throw is `unavailable`. */
export type FetchContentFn = (address: ContentAddress) => Promise<MirrorFetch>;

/**
 * Whether the logo may be rendered, and — when it may not — why, in words a person can act on.
 *
 * THREE states, and the middle one is the whole slice: an unverified logo renders NOTHING. Not a
 * placeholder, not a broken-image icon, not an initials block. A logo is the strongest visual claim
 * of legitimacy a provider makes, so a stand-in for one that cannot be verified is precisely how a
 * forged provider comes to look real.
 *
 * `notPublished` is NORMAL and must read as normal. `unverified` is a failure and must read as one.
 * Both render no image, so that asymmetry in tone is what makes them distinguishable — this repo's
 * standing rule that facts render neutral and failures warn, applied to the one place where the two
 * would otherwise look identical.
 *
 * `reason` is present if and ONLY if the state is `unverified`.
 */
export type LogoState =
  | { readonly state: "verified"; readonly bytes: Uint8Array; readonly mediaType: string }
  | { readonly state: "unverified"; readonly reason: string }
  | { readonly state: "notPublished" };

/**
 * The resolved profile.
 *
 * `contact` is present only when the blob verified. `logo` always carries a state, including when
 * the blob itself failed — an unverifiable blob means the logo is unverifiable too, and saying so is
 * more honest than reporting no logo.
 */
export type ProfileResolution =
  | {
      readonly state: "resolved";
      readonly profile: ProviderProfile;
      readonly logo: LogoState;
      readonly address: ContentAddress;
    }
  /** Nothing is anchored. Ordinary for a provider that has published nothing. */
  | { readonly state: "notPublished"; readonly withdrawn: boolean }
  /** The anchor names content that could not be verified, or could not be reached. */
  | { readonly state: "unverified"; readonly reason: string; readonly logo: LogoState };

async function fetchAndVerify(
  address: ContentAddress,
  hashAlgorithm: number,
  fetchContent: FetchContentFn,
  digest: BytesDigestFn,
  what: string,
): Promise<
  | { ok: true; bytes: Uint8Array; mediaType: string }
  | { ok: false; reason: string }
> {
  let read: MirrorFetch;
  try {
    read = await fetchContent(address);
  } catch (error) {
    // A seam that throws instead of resolving is still a failure to reach, never an absence.
    return { ok: false, reason: `the ${what} could not be reached (${describe(error)})` };
  }
  if (read.state === "absent") {
    return { ok: false, reason: `the mirror holds no ${what} at the address this provider published` };
  }
  if (read.state === "unavailable") {
    return { ok: false, reason: `the ${what} could not be reached (${read.reason})` };
  }
  const verification = verifyContentAddress(read.bytes, address, hashAlgorithm, digest);
  if (verification.state === "unsupportedAlgorithm") {
    return {
      ok: false,
      reason: `the ${what} is published under hash algorithm ${verification.hashAlgorithm}, which this build cannot compute`,
    };
  }
  if (verification.state === "mismatch") {
    return {
      ok: false,
      reason: `the ${what} does not match the address this provider published`,
    };
  }
  return { ok: true, bytes: read.bytes, mediaType: read.mediaType };
}

function describe(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return "no reason given";
}

async function resolveLogo(
  logo: ProfileLogoRef,
  fetchContent: FetchContentFn,
  digest: BytesDigestFn,
): Promise<LogoState> {
  const fetched = await fetchAndVerify(logo.digest, logo.hashAlgorithm, fetchContent, digest, "logo");
  if (!fetched.ok) return { state: "unverified", reason: fetched.reason };

  // A correct hash proves these are the published bytes; it proves nothing about what they ARE. The
  // mirror sniffs on ingest, but this consumer does not take the serving side's word for that any
  // more than it takes its word for the bytes.
  const servedType = (fetched.mediaType.split(";")[0] ?? "").trim().toLowerCase();
  if (servedType !== logo.mediaType) {
    return {
      state: "unverified",
      reason: `the logo was served as ${fetched.mediaType || "an unstated type"} but this provider published it as ${logo.mediaType}`,
    };
  }
  if (fetched.bytes.length !== logo.byteLength) {
    // Unreachable while the digest verifies — different lengths give different digests — so this is
    // a consistency guard on the DOCUMENT rather than on the bytes: it catches a blob whose declared
    // length disagrees with the content it names.
    return {
      state: "unverified",
      reason: "the logo's published length disagrees with the content at its address",
    };
  }
  return { state: "verified", bytes: fetched.bytes, mediaType: logo.mediaType };
}

export async function resolveProviderProfile(
  anchor: ProfileAnchorRef,
  fetchContent: FetchContentFn,
  digest: BytesDigestFn,
): Promise<ProfileResolution> {
  if (!namesContent(anchor.digest)) {
    // A zero digest at revision 0 was never published; at a higher revision it was withdrawn. Both
    // are ordinary, and `ProviderDirectory.clearProfileAnchor` advances the revision precisely so a
    // consumer can tell them apart.
    const withdrawn =
      anchor.revision > 0n && anchor.digest.toLowerCase() === ZERO_ADDRESS;
    return { state: "notPublished", withdrawn };
  }
  const address = anchor.digest.toLowerCase() as ContentAddress;

  const fetched = await fetchAndVerify(
    address,
    anchor.hashAlgorithm,
    fetchContent,
    digest,
    "profile document",
  );
  if (!fetched.ok) {
    return {
      state: "unverified",
      reason: fetched.reason,
      logo: { state: "unverified", reason: fetched.reason },
    };
  }

  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(fetched.bytes);
  } catch {
    const reason = "the profile document is not valid UTF-8";
    return { state: "unverified", reason, logo: { state: "unverified", reason } };
  }

  const parsed = parseProfileBlob(text);
  if (!parsed.ok) {
    return {
      state: "unverified",
      reason: parsed.reason,
      logo: { state: "unverified", reason: parsed.reason },
    };
  }

  const logo = parsed.profile.logo
    ? await resolveLogo(parsed.profile.logo, fetchContent, digest)
    : ({ state: "notPublished" } as const);

  return { state: "resolved", profile: parsed.profile, logo, address };
}
