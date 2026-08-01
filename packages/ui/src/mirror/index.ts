/**
 * The content-addressed profile and logo mirror (registry-plan S-17).
 *
 * `ProviderDirectory.ProfileAnchor` publishes a digest and says nothing about where the bytes live.
 * This is both halves of the answer: the client that fetches them, and the recomputation that
 * decides whether what came back is what was published.
 *
 * The serving half is `stacks/indexer/api/src/mirror.rs`.
 */

export {
  isContentAddress,
  MULTIHASH_KECCAK_256,
  namesContent,
  verifyContentAddress,
  ZERO_ADDRESS,
  type BytesDigestFn,
  type ContentAddress,
  type ContentVerification,
} from "./contentAddress";

export {
  buildProfileBlob,
  logoRef,
  parseProfileBlob,
  PROFILE_MEDIA_TYPE,
  PROVIDER_PROFILE_SCHEMA,
  PROVIDER_PROFILE_SCHEMA_ID,
  SERVABLE_IMAGE_MEDIA_TYPES,
  type LogoPublication,
  type ProfileBlobParse,
  type ProfileLogoRef,
  type ProviderProfile,
  type ServableImageMediaType,
} from "./profileBlob";

export {
  resolveProviderProfile,
  type FetchContentFn,
  type LogoState,
  type MirrorFetch,
  type ProfileAnchorRef,
  type ProfileResolution,
} from "./resolveProfile";

export {
  keccakBytes,
  mirrorContentReader,
  publicationDigest,
  putMirrorContent,
} from "./client";

export { ProviderLogo, type ProviderLogoProps } from "./ProviderLogo";
