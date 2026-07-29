import type { IssuerDomainBinding } from "../domain/issuerDomainBinding";
import type { LatLng } from "../geo";

/**
 * The directory uses the codebase's existing freshness vocabulary verbatim.
 *
 * Deriving this from `IssuerDomainBinding` is deliberate: directory snapshots and DNS observations
 * must not grow separate names for the same live-read versus stored-replay distinction.
 */
export type DirectoryObservation = NonNullable<IssuerDomainBinding["dnsObservation"]>;

/** The two directory implementations in this slice. */
export type ProviderDirectorySource = "central" | "onchain";

/**
 * The source-neutral provider shape needed by the list consumer.
 *
 * This intentionally excludes central-only wire fields such as `hmacKeyId`, `apiBaseUrl`, and
 * `documentStores`: a future on-chain source could not supply them honestly. `active` is nullable
 * because today's central response has no delisting fact at all; `null` must never be read as
 * `false`. The future on-chain scan can populate its maintained `active` assertion.
 */
export interface DirectoryProvider {
  /** Central `businessId` today; the future KYC-assigned provider id behind the on-chain source. */
  providerId: string;
  kind: string;
  name: string;
  /**
   * `null` is the ordinary location-less-provider case, and it must survive as `null`.
   *
   * A provider may publish contact channels and no address at all. Before this was nullable the
   * only representable answer was a coordinate, so a blank location became `0, 0` - a legal point
   * in the Gulf of Guinea - and every surface drew a pin there. Substituting any fallback
   * coordinate re-creates that defect exactly.
   *
   * Consumers must read it through `providerPosition`, which is what makes "no location" flow into
   * `sortByDistance` (last, `distanceKm: null`) and out of any Directions affordance.
   */
  geo: LatLng | null;
  /**
   * How to reach this provider. Every channel is independently nullable.
   *
   * BUSINESS channels, not personal ones - a provider publishes them so it can be contacted. This
   * is the whole of a location-less provider's usefulness, so it is a required member with
   * nullable fields rather than an optional object: a consumer cannot forget to consider it.
   */
  contacts: ProviderContacts;
  services: readonly string[];
  /** `null` is the ordinary domain-less-provider case. */
  domain: string | null;
  /** `null` means this source supplied no listing-state fact. It is not evidence of delisting. */
  active: boolean | null;
}

/** The contact channels a provider may publish. `null` is "this provider published no such channel". */
export interface ProviderContacts {
  phone: string | null;
  whatsapp: string | null;
  telegram: string | null;
  email: string | null;
  website: string | null;
}

/** The channel keys, in the order a listing should offer them. */
export const PROVIDER_CONTACT_CHANNELS = [
  "phone",
  "whatsapp",
  "telegram",
  "email",
  "website",
] as const satisfies readonly (keyof ProviderContacts)[];

export type ProviderContactChannel = (typeof PROVIDER_CONTACT_CHANNELS)[number];

interface ProviderDirectorySnapshotBase {
  source: ProviderDirectorySource;
  /** A read performed now, or an unexpired snapshot replayed after that read could not complete. */
  observation: DirectoryObservation;
  /**
   * The block the whole snapshot was pinned to.
   *
   * Required but nullable. Today's central response has no chain anchor, so it MUST use `null`; a
   * separately-read chain head would not make the central database response a block-pinned fact.
   */
  blockNumber: number | null;
  /** Unix milliseconds of the original successful source read. Preserved when replayed from cache. */
  readAt: number;
  /**
   * Unix milliseconds at which a cache-managed snapshot stops being usable.
   *
   * `null` means this result came from a directory without the cache wrapper. It does not mean the
   * underlying facts are permanent.
   */
  expiresAt: number | null;
}

/** A successful read containing at least one provider. */
export interface ProviderDirectoryFound extends ProviderDirectorySnapshotBase {
  state: "found";
  providers: readonly [DirectoryProvider, ...DirectoryProvider[]];
}

/** A successful read that established there are no providers in this source. */
export interface ProviderDirectoryEmpty extends ProviderDirectorySnapshotBase {
  state: "empty";
  providers: readonly [];
}

export type ProviderDirectorySnapshot = ProviderDirectoryFound | ProviderDirectoryEmpty;

export type ProviderDirectoryUnavailableReason =
  | "sourceUnavailable"
  | "malformedResponse"
  | "providerRegistryUnavailable"
  | "inconsistentSource"
  | "invalidSnapshot";

/**
 * The source could not answer.
 *
 * There is deliberately no `providers` member. Code that has narrowed to `unavailable` cannot feed
 * an empty array to a list and accidentally claim that no providers exist.
 */
export interface ProviderDirectoryUnavailable {
  state: "unavailable";
  source: ProviderDirectorySource;
  reason: ProviderDirectoryUnavailableReason;
  detail: string;
  /** Unix milliseconds of the failed/unavailable attempt. */
  attemptedAt: number;
}

export type ProviderDirectoryResult = ProviderDirectorySnapshot | ProviderDirectoryUnavailable;

/**
 * A full-set provider directory read.
 *
 * `read` accepts no query at all. In particular, there is nowhere to put a user position: every
 * caller asks the source the same question, then filters and sorts with `packages/ui/src/geo/`. The
 * product has no in-app map, so viewport, bounding-box, region, and geohash query shapes have no
 * caller and must not be added. A list row may hand its already-held destination to the OS maps app.
 * Implementations resolve `unavailable` rather than throwing for source/read failures.
 */
export interface ProviderDirectory {
  readonly source: ProviderDirectorySource;
  /**
   * Stable identity of the configured source, used only to scope cached snapshots.
   *
   * It must distinguish two distinct configured bases - including two central bases that resolve to
   * different origins - and two future chain/registry configurations. It must never contain a user
   * position or a derived region.
   *
   * What it deliberately does NOT promise: two deployments served from the SAME origin whose
   * relative base (`/api`) proxies to different backends are indistinguishable here, because the
   * client has no observable difference to key on. A per-origin store such as `localStorage` is
   * already shared between them, so no namespace could separate them either.
   */
  readonly cacheNamespace: string;
  read(): Promise<ProviderDirectoryResult>;
}
