import type { CentralClient } from "../api/central";
import type { BusinessesResp, CentralBusiness } from "../api/types";
import type {
  DirectoryProvider,
  ProviderDirectory,
  ProviderDirectoryResult,
  ProviderDirectoryUnavailable,
} from "./types";

export interface CentralDirectoryOptions {
  /** Injected for deterministic tests. Defaults to `Date.now`. */
  now?: () => number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isCentralBusiness(value: unknown): value is CentralBusiness {
  if (!isRecord(value) || !isRecord(value.geo)) return false;
  return (
    typeof value.businessId === "string" &&
    typeof value.type === "string" &&
    typeof value.name === "string" &&
    typeof value.geo.lat === "number" &&
    Number.isFinite(value.geo.lat) &&
    typeof value.geo.lng === "number" &&
    Number.isFinite(value.geo.lng) &&
    isStringArray(value.services) &&
    typeof value.apiBaseUrl === "string" &&
    typeof value.domain === "string" &&
    isStringArray(value.documentStores) &&
    typeof value.hmacKeyId === "string"
  );
}

function isBusinessesResp(value: unknown): value is BusinessesResp {
  return (
    isRecord(value) &&
    Array.isArray(value.businesses) &&
    value.businesses.every(isCentralBusiness)
  );
}

function errorDetail(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return "GET /v1/businesses could not be read";
}

function toDirectoryProvider(business: CentralBusiness): DirectoryProvider {
  return {
    providerId: business.businessId,
    kind: business.type,
    name: business.name,
    geo: { ...business.geo },
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

  return {
    source: "central",
    cacheNamespace: `central:${client.base}`,
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

      if (!isBusinessesResp(response)) {
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
