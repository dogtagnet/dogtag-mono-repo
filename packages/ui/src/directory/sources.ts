import type { CentralClient } from "../api/central";
import { isValidLatLng, type LatLng } from "../geo";
import type {
  DirectoryProvider,
  DirectoryProviderContact,
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
 * `unavailable`. `geo` and `contact` are optional because a contact-only provider is a valid
 * directory entry even though it cannot appear in a distance-ranked Nearby list.
 */
interface DirectoryContactRow {
  phone?: string | null;
  whatsapp?: string | null;
  telegram?: string | null;
  email?: string | null;
}

interface DirectoryRow {
  businessId: string;
  type: string;
  name: string;
  geo?: LatLng | null;
  services: string[];
  domain: string;
  contact?: DirectoryContactRow | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

const CONTACT_FIELDS = ["phone", "whatsapp", "telegram", "email"] as const;

function isOptionalContact(value: unknown): value is DirectoryContactRow | null | undefined {
  if (value === undefined || value === null) return true;
  if (!isRecord(value)) return false;
  return CONTACT_FIELDS.every((field) => {
    const contactValue = value[field];
    return (
      contactValue === undefined ||
      contactValue === null ||
      typeof contactValue === "string"
    );
  });
}

/**
 * Present coordinates are checked with the same `isValidLatLng` the on-device distance/sort path
 * uses. Missing/null is the valid contact-only case; any other malformed or out-of-range value makes
 * the response unavailable rather than manufacturing a coordinate or silently dropping the row.
 */
function isDirectoryRow(value: unknown): value is DirectoryRow {
  if (!isRecord(value)) return false;
  const geoIsValid =
    value.geo === undefined ||
    value.geo === null ||
    (isRecord(value.geo) && isValidLatLng(value.geo as unknown as LatLng));
  return (
    typeof value.businessId === "string" &&
    typeof value.type === "string" &&
    typeof value.name === "string" &&
    geoIsValid &&
    isStringArray(value.services) &&
    typeof value.domain === "string" &&
    isOptionalContact(value.contact)
  );
}

/**
 * All-or-nothing on purpose. Dropping the rows that fail would let a wholly malformed response
 * degrade into a successful `empty`, which is the one outcome this seam exists to prevent.
 *
 * A repeated `businessId` is malformed for the same reason: the id is the consumer's list identity,
 * so silently keeping both rows corrupts rendering rather than reporting a bad response.
 */
function hasDirectoryRows(value: unknown): value is { businesses: DirectoryRow[] } {
  if (!isRecord(value) || !Array.isArray(value.businesses)) return false;
  if (!value.businesses.every(isDirectoryRow)) return false;
  const ids = new Set<string>();
  return (value.businesses as DirectoryRow[]).every((row) => {
    if (ids.has(row.businessId)) return false;
    ids.add(row.businessId);
    return true;
  });
}

function errorDetail(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return "GET /v1/businesses could not be read";
}

function normalizedContactValue(value: string | null | undefined): string | null {
  return typeof value === "string" ? value.trim() || null : null;
}

function normalizeContact(
  contact: DirectoryContactRow | null | undefined,
): DirectoryProviderContact {
  return {
    phone: normalizedContactValue(contact?.phone),
    whatsapp: normalizedContactValue(contact?.whatsapp),
    telegram: normalizedContactValue(contact?.telegram),
    email: normalizedContactValue(contact?.email),
  };
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

function toDirectoryProvider(business: DirectoryRow): DirectoryProvider {
  const domain = business.domain.trim() || null;

  return {
    providerId: business.businessId,
    kind: business.type,
    name: business.name,
    // Absence is preserved. In particular, it is never translated to the real Atlantic coordinate
    // `{ lat: 0, lng: 0 }`, which would make a contact-only listing look placeable.
    geo: business.geo === undefined || business.geo === null ? null : { ...business.geo },
    services: [...business.services],
    contact: normalizeContact(business.contact),
    domain,
    // The central list does not execute the issuer↔domain binding check, and reads no chain state at
    // all. A non-empty published domain is therefore unavailable, never verified; a blank one says
    // only that this listing carries no domain, which is not the on-chain fact `noDomainClaimed`.
    bindingState: domain === null ? "noDomainListed" : "unavailable",
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
