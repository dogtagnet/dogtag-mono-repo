import type { CentralClient } from "../api/central";
import type { BusinessContact, CentralBusiness } from "../api/types";
import { isValidLatLng, type LatLng } from "../geo";
import { PROVIDER_CONTACT_CHANNELS as CONTACT_CHANNELS } from "./types";
import type {
  DirectoryProvider,
  ProviderContacts,
  ProviderDirectory,
  ProviderDirectoryResult,
  ProviderDirectoryUnavailable,
} from "./types";

export interface CentralDirectoryOptions {
  /** Injected for deterministic tests. Defaults to `Date.now`. */
  now?: () => number;
  /**
   * Absolute origin a relative API base is resolved against when deriving `cacheNamespace`.
   *
   * Defaults to the ambient document origin. It never affects which URL is fetched.
   */
  documentOrigin?: string;
}

/**
 * The subset of a `GET /v1/businesses` row this seam consumes.
 *
 * `apiBaseUrl`, `documentStores` and `hmacKeyId` are deliberately absent: none of them reaches a
 * `DirectoryProvider`, so requiring them would let a wire change with nothing to do with discovery -
 * dropping `hmacKeyId` from a public, unauthenticated endpoint, say - take the whole directory to
 * `unavailable`.
 */
type DirectoryRow = Pick<
  CentralBusiness,
  "businessId" | "type" | "name" | "geo" | "contact" | "services" | "domain"
>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

/**
 * Is this row's `geo` acceptable?
 *
 * THREE cases, and the split is the point of this whole seam:
 *
 * - ABSENT or `null` → accepted as "this provider published no location". A location-less provider
 *   is first class, so its row must not fail. Absence is accepted in both spellings deliberately:
 *   our own server emits an explicit `null`, but a serializer that omits nulls is an ordinary wire
 *   difference and must not take the entire directory to `unavailable`.
 * - a valid `{lat, lng}` → accepted, checked with the same `isValidLatLng` the on-device
 *   distance/sort path uses, so a coordinate that could never be presented as a distance is
 *   rejected here rather than ranked as one.
 * - anything else (out-of-range numbers, a string, a number, `[]`) → REJECTED, which under
 *   `hasDirectoryRows` still takes the whole batch to `unavailable`. That is unchanged and
 *   intentional: absence is a fact this source can state, malformed is a response we cannot trust.
 *
 * The previous version had only the middle case, so one location-less row blanked the directory for
 * every consumer - an all-or-nothing failure hiding inside a per-row validator.
 */
function hasAcceptableGeo(value: Record<string, unknown>): boolean {
  const geo = value.geo;
  if (geo === undefined || geo === null) return true;
  if (!isRecord(geo)) return false;
  return isValidLatLng(geo as unknown as LatLng);
}

/**
 * Absent, `null`, or a string. Anything else is a wire-shape violation, not an absent channel.
 *
 * Contact channels are decoration for trust purposes, but a number or an object where a string
 * belongs still says this response is not what we asked for - and absence, the only case a real
 * provider produces, can never reach the rejecting branch.
 */
function isOptionalString(value: unknown): boolean {
  return value === undefined || value === null || typeof value === "string";
}

function hasAcceptableContact(value: Record<string, unknown>): boolean {
  const contact = value.contact;
  if (contact === undefined || contact === null) return true;
  if (!isRecord(contact)) return false;
  return CONTACT_CHANNELS.every((channel) => isOptionalString(contact[channel]));
}

function isDirectoryRow(value: unknown): value is DirectoryRow {
  if (!isRecord(value)) return false;
  return (
    typeof value.businessId === "string" &&
    typeof value.type === "string" &&
    typeof value.name === "string" &&
    hasAcceptableGeo(value) &&
    hasAcceptableContact(value) &&
    isStringArray(value.services) &&
    typeof value.domain === "string"
  );
}

/**
 * All-or-nothing on purpose. Dropping the rows that fail would let a wholly malformed response
 * degrade into a successful `empty`, which is the one outcome this seam exists to prevent.
 */
function hasDirectoryRows(value: unknown): value is { businesses: DirectoryRow[] } {
  return (
    isRecord(value) &&
    Array.isArray(value.businesses) &&
    value.businesses.every(isDirectoryRow)
  );
}

function errorDetail(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return "GET /v1/businesses could not be read";
}

/** A configured base with nothing to resolve. It must not fall back to the current page URL. */
const UNRESOLVED_BASE = "unresolved-base";

function ambientOrigin(): string | undefined {
  const location = (globalThis as { location?: unknown }).location;
  if (typeof location !== "object" || location === null) return undefined;
  const origin = (location as { origin?: unknown }).origin;
  return typeof origin === "string" && origin ? origin : undefined;
}

/**
 * Resolution is against the ORIGIN, never the current href: a path-relative base resolved against a
 * full href would namespace one deployment differently per route.
 */
function centralNamespaceKey(base: string, documentOrigin?: string): string {
  const configured = base.trim();
  if (!configured) return UNRESOLVED_BASE;
  const origin = documentOrigin ?? ambientOrigin();
  try {
    const url = origin === undefined ? new URL(configured) : new URL(configured, origin);
    return `${url.origin}${url.pathname}`;
  } catch {
    return configured;
  }
}

/** Trim, and read a blank string as the absent channel it means. */
function contactValue(value: string | undefined | null): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function toProviderContacts(contact: BusinessContact | undefined | null): ProviderContacts {
  return {
    phone: contactValue(contact?.phone),
    whatsapp: contactValue(contact?.whatsapp),
    telegram: contactValue(contact?.telegram),
    email: contactValue(contact?.email),
    website: contactValue(contact?.website),
  };
}

function toDirectoryProvider(business: DirectoryRow): DirectoryProvider {
  return {
    providerId: business.businessId,
    kind: business.type,
    name: business.name,
    // Written as a conditional rather than the previous `{ ...business.geo }` because object spread
    // of `null` yields `{}` at runtime, NOT `null` - which would hand a location-less provider an
    // empty object: not a usable position, but not `null` either, so a downstream `geo !== null`
    // guard would pass on it. Checked rather than assumed: with `geo: LatLng | null` the spread is
    // in fact REJECTED by tsc (spreading a possibly-null value makes the members optional, and
    // `LatLng` requires them). The guarantee is still pinned behaviourally - `providerNoLocation`
    // asserts `provider.geo === null` by identity - because the type only happens to catch it here,
    // and a future `geo?: LatLng` or an `as` cast anywhere on this path would silently restore it.
    geo: business.geo ? { lat: business.geo.lat, lng: business.geo.lng } : null,
    contacts: toProviderContacts(business.contact),
    services: [...business.services],
    domain: business.domain.trim() || null,
    // `/v1/businesses` has no delisting/whitelist fact. Inventing `true` here would turn discovery
    // data into a claim about current standing that the source never made.
    active: null,
  };
}

/**
 * Today's provider directory: one full, position-free `GET /v1/businesses`.
 *
 * The central response is not a chain read and carries no block, so every successful result is
 * honestly unanchored (`blockNumber: null`). Network, HTTP, and response-shape failures resolve to
 * `unavailable`; none of them are converted to an empty provider list.
 */
export function centralDirectory(
  client: Pick<CentralClient, "base" | "listBusinesses">,
  options: CentralDirectoryOptions = {},
): ProviderDirectory {
  const now = options.now ?? Date.now;

  const cacheNamespace = `central:${centralNamespaceKey(client.base, options.documentOrigin)}`;

  return {
    source: "central",
    cacheNamespace,
    async read(): Promise<ProviderDirectoryResult> {
      let response: unknown;
      try {
        // No argument is load-bearing: it makes this a full fetch and leaves no request-shaped place
        // for the caller's coordinates. `listBusinesses` itself has a second wire-level regression
        // guard in `businessesQueryNoPosition.test.ts`.
        response = await client.listBusinesses();
      } catch (error) {
        const unavailable: ProviderDirectoryUnavailable = {
          state: "unavailable",
          source: "central",
          reason: "sourceUnavailable",
          detail: errorDetail(error),
          attemptedAt: now(),
        };
        return unavailable;
      }

      if (!hasDirectoryRows(response)) {
        return {
          state: "unavailable",
          source: "central",
          reason: "malformedResponse",
          detail:
            "GET /v1/businesses returned an invalid response; it was not treated as an empty directory",
          attemptedAt: now(),
        };
      }

      const providers = response.businesses.map(toDirectoryProvider);
      const [first, ...rest] = providers;
      const readAt = now();
      if (first === undefined) {
        return {
          state: "empty",
          source: "central",
          providers: [],
          observation: "live",
          blockNumber: null,
          readAt,
          expiresAt: null,
        };
      }

      return {
        state: "found",
        source: "central",
        providers: [first, ...rest],
        observation: "live",
        blockNumber: null,
        readAt,
        expiresAt: null,
      };
    },
  };
}

export interface OnchainDirectoryOptions {
  /** Injected for deterministic tests. Defaults to `Date.now`. */
  now?: () => number;
}

/**
 * The future paged on-chain provider directory.
 *
 * The provider registry, its packed page ABI, and its deployment do not exist yet. This stub makes
 * no RPC call and returns `unavailable` every time. Returning `empty` here would assert that a real
 * registry was read and contained no providers, which is a materially false claim.
 */
export function onchainDirectory(options: OnchainDirectoryOptions = {}): ProviderDirectory {
  const now = options.now ?? Date.now;
  return {
    source: "onchain",
    // The stub has no chain/registry configuration. The real implementation must include both here.
    cacheNamespace: "onchain:provider-registry-unavailable",
    async read() {
      return {
        state: "unavailable",
        source: "onchain",
        reason: "providerRegistryUnavailable",
        detail:
          "The on-chain provider directory is unavailable: its provider registry and paged-read ABI are not designed or deployed yet",
        attemptedAt: now(),
      };
    },
  };
}
