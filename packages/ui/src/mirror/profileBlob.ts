/**
 * The provider profile blob — the document a `ProfileAnchor` digest names.
 *
 * BUILD and PARSE live in one module on purpose. The publisher's shape and the consumer's shape are
 * the same shape, and two definitions of it would be free to drift into a provider publishing
 * something no consumer can read — with nothing anywhere reporting the disagreement, because the
 * digest would still verify perfectly.
 *
 * # Why the logo digest is INSIDE the blob
 *
 * `ProfileAnchor` carries exactly one `digest`. So the logo's own content address is a field of the
 * blob, which puts it under the blob's digest, which is what the chain anchors. That gives one
 * unbroken chain of custody:
 *
 *   chain anchor → blob bytes (verified) → logo address (covered by that verification) → logo bytes
 *   (verified)
 *
 * A logo address read out of an UNVERIFIED blob has no standing at all, which is why
 * `resolveProviderProfile` refuses to fetch one.
 */

import { PROVIDER_CONTACT_CHANNELS, type ContactChannelRecord } from "../directory/channels";
import { isContentAddress, MULTIHASH_KECCAK_256, type ContentAddress } from "./contentAddress";

/**
 * The blob's self-description, and `ProfileAnchor.schema`'s value.
 *
 * **Both spellings move together, always.** The uint32 on chain and the string in the document
 * describe the same document, so changing the blob shape under an unchanged schema is the "one
 * document with two on-chain descriptions of what it is" problem `directoryPlan.ts` already warns
 * about for the anchor's other fields.
 *
 * Bumped 1 → 2 by S-17, which added `logo`. That was free exactly once: `ProviderDirectory` is
 * deployed but `resolverApproved()` is false and every store is empty, so no anchor names a v1
 * document and none ever will. A later change does not get that window.
 */
export const PROVIDER_PROFILE_SCHEMA = 2;
export const PROVIDER_PROFILE_SCHEMA_ID = "dogtag/provider-profile/2";

/**
 * The image media types a published logo may declare, and the allowlist `parseProfileBlob` checks a
 * logo entry against.
 *
 * **HAND-MIRRORED from `mirror.rs::SERVABLE_IMAGE_MEDIA_TYPES`, and the two move together or not at
 * all.** Nothing derives one from the other, so each side pins its own list exhaustively - here by
 * `keeps the servable image set closed, and hand-mirrored with mirror.rs` in
 * `test/contentAddress.test.ts` - and these notes are what carry the pair across the language
 * boundary, because neither test can read the other language.
 *
 * It mirrors the IMAGE half only. `mirror.rs::is_servable_media_type` additionally admits
 * `PROFILE_MEDIA_TYPE`, because the mirror stores the profile blob itself - but that type must never
 * be added HERE, since this list is what a LOGO entry is checked against and a logo declaring
 * `application/json` would name a document `ProviderLogo` then hands to an `<img>`.
 *
 * The two drift directions fail differently. Rust widening alone is the quiet one: the mirror
 * accepts a logo media type this list refuses, and a refused logo entry fails the WHOLE document by
 * design, so that provider's contacts are withheld too. This side widening alone is loud: the mirror
 * refuses the upload and publication is blocked.
 */
export const SERVABLE_IMAGE_MEDIA_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
] as const;
export type ServableImageMediaType = (typeof SERVABLE_IMAGE_MEDIA_TYPES)[number];

/** The profile blob's own media type when mirrored. */
export const PROFILE_MEDIA_TYPE = "application/json";

/**
 * A logo the provider published, as named by the blob.
 *
 * `mediaType` is a CLAIM even when the address verifies. A correct hash proves these are the bytes
 * the provider published; it proves nothing about what they are. The mirror sniffs and allowlists on
 * ingest — SVG is refused outright, because it is an XML document that can carry script and this
 * renders inside operator portals — and the consumer re-checks the claim against that allowlist
 * rather than handing an arbitrary string to the browser.
 */
export interface ProfileLogoRef {
  readonly digest: ContentAddress;
  readonly hashAlgorithm: number;
  readonly mediaType: ServableImageMediaType;
  readonly byteLength: number;
}

export interface ProviderProfile {
  readonly contact: ContactChannelRecord<string | null>;
  /** `null` is the ordinary "this provider published no logo" case, and is never a failure. */
  readonly logo: ProfileLogoRef | null;
}

/** What a logo the provider published, before it is mirrored. */
export interface LogoPublication {
  readonly bytes: Uint8Array;
  readonly mediaType: ServableImageMediaType;
}

function isServableImageMediaType(value: unknown): value is ServableImageMediaType {
  return (
    typeof value === "string" &&
    (SERVABLE_IMAGE_MEDIA_TYPES as readonly string[]).includes(value)
  );
}

/**
 * The canonical blob bytes.
 *
 * Channels are emitted in the shared list's order and a blank channel is OMITTED rather than carried
 * as `""` — an omitted channel is absent, a present-but-empty one is a published emptiness, and the
 * digest must not make those the same document. `logo` is omitted entirely when there is none, for
 * the same reason.
 *
 * Content addressing gives integrity, never privacy: anyone holding the address can fetch what it
 * names, so only publication-safe values belong in it.
 */
export function buildProfileBlob(
  contacts: Readonly<ContactChannelRecord<string>>,
  logo: ProfileLogoRef | null,
): { blob: string; channelsPublished: number } {
  const published: [string, string][] = [];
  for (const channel of PROVIDER_CONTACT_CHANNELS) {
    const value = (contacts[channel] ?? "").trim();
    if (value) published.push([channel, value]);
  }
  const document: Record<string, unknown> = {
    schema: PROVIDER_PROFILE_SCHEMA_ID,
    contact: Object.fromEntries(published),
  };
  if (logo) {
    document.logo = {
      digest: logo.digest,
      hashAlgorithm: logo.hashAlgorithm,
      mediaType: logo.mediaType,
      byteLength: logo.byteLength,
    };
  }
  return { blob: JSON.stringify(document), channelsPublished: published.length };
}

export type ProfileBlobParse =
  | { readonly ok: true; readonly profile: ProviderProfile }
  | { readonly ok: false; readonly reason: string };

/**
 * Parse a blob whose bytes have ALREADY been verified against the anchored digest.
 *
 * Never call this on unverified bytes: everything it returns — the contacts a portal renders as
 * published facts, and the logo address a renderer would fetch — inherits its standing from that
 * verification and has none without it.
 *
 * A malformed logo entry makes the whole parse fail rather than degrading to `logo: null`. Silently
 * dropping it would render as "this provider published no logo", which is a false statement about a
 * provider that published one; the honest answer is that the document could not be read.
 */
export function parseProfileBlob(text: string): ProfileBlobParse {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, reason: "the profile document is not valid JSON" };
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { ok: false, reason: "the profile document is not an object" };
  }
  const doc = raw as Record<string, unknown>;
  if (doc.schema !== PROVIDER_PROFILE_SCHEMA_ID) {
    return {
      ok: false,
      reason: `the profile document declares schema ${JSON.stringify(doc.schema)}, which this build cannot read`,
    };
  }

  const contactRaw = doc.contact;
  if (typeof contactRaw !== "object" || contactRaw === null || Array.isArray(contactRaw)) {
    return { ok: false, reason: "the profile document has no contact object" };
  }
  const contactSource = contactRaw as Record<string, unknown>;
  const contact = Object.fromEntries(
    PROVIDER_CONTACT_CHANNELS.map((channel) => {
      const value = contactSource[channel];
      return [channel, typeof value === "string" && value.trim() ? value.trim() : null];
    }),
  ) as ContactChannelRecord<string | null>;

  if (doc.logo === undefined || doc.logo === null) {
    return { ok: true, profile: { contact, logo: null } };
  }
  if (typeof doc.logo !== "object" || Array.isArray(doc.logo)) {
    return { ok: false, reason: "the profile document's logo entry is not an object" };
  }
  const logo = doc.logo as Record<string, unknown>;
  if (!isContentAddress(logo.digest as string)) {
    return { ok: false, reason: "the profile document's logo names no valid content address" };
  }
  if (typeof logo.hashAlgorithm !== "number" || !Number.isInteger(logo.hashAlgorithm)) {
    return { ok: false, reason: "the profile document's logo declares no hash algorithm" };
  }
  if (!isServableImageMediaType(logo.mediaType)) {
    return {
      ok: false,
      reason: `the profile document's logo declares media type ${JSON.stringify(logo.mediaType)}, which is not rendered here`,
    };
  }
  if (typeof logo.byteLength !== "number" || !Number.isInteger(logo.byteLength) || logo.byteLength < 0) {
    return { ok: false, reason: "the profile document's logo declares no byte length" };
  }
  return {
    ok: true,
    profile: {
      contact,
      logo: {
        digest: (logo.digest as string).toLowerCase() as ContentAddress,
        hashAlgorithm: logo.hashAlgorithm,
        mediaType: logo.mediaType,
        byteLength: logo.byteLength,
      },
    },
  };
}

/** Build the `logo` entry a publication would anchor, from the bytes and their address. */
export function logoRef(
  address: ContentAddress,
  publication: LogoPublication,
): ProfileLogoRef {
  return {
    digest: address,
    hashAlgorithm: MULTIHASH_KECCAK_256,
    mediaType: publication.mediaType,
    byteLength: publication.bytes.length,
  };
}
