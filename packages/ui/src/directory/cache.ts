import type {
  ProviderDirectory,
  ProviderDirectoryResult,
  ProviderDirectorySnapshot,
} from "./types";

export interface ProviderDirectoryCacheEntry {
  /**
   * Concrete source identity, not merely `"central" | "onchain"`.
   *
   * Prevents snapshots crossing central origins or future chain/registry configurations.
   */
  namespace: string;
  /** The block anchor travels with the cached result, inside `snapshot`. */
  snapshot: ProviderDirectorySnapshot & { expiresAt: number };
  /** Unix milliseconds. The entry is usable only while `now < expiresAt`. */
  expiresAt: number;
}

/**
 * Storage for one full-set directory snapshot.
 *
 * It is intentionally not keyed by a filter, geohash, or position. The source read has no query and
 * the cache stores that same universal result.
 */
export interface ProviderDirectoryCache {
  read(): ProviderDirectoryCacheEntry | null;
  write(entry: ProviderDirectoryCacheEntry): void;
  clear(): void;
}

/** Small in-memory default. A later persistent adapter can implement the same three methods. */
export function createMemoryProviderDirectoryCache(): ProviderDirectoryCache {
  let entry: ProviderDirectoryCacheEntry | null = null;
  return {
    read: () => entry,
    write: (next) => {
      entry = next;
    },
    clear: () => {
      entry = null;
    },
  };
}

export interface ProviderDirectoryCacheOptions {
  cache: ProviderDirectoryCache;
  /** Hard lifetime of a stored snapshot in milliseconds. Must be finite and greater than zero. */
  ttlMs: number;
  /** Injected for deterministic tests. Defaults to `Date.now`. */
  now?: () => number;
}

/**
 * Re-check a directory, replaying its last successful snapshot only when the live read is
 * unavailable and the hard TTL has not elapsed.
 *
 * A successful LIVE read replaces the cache. A stored replay never renews its hard deadline. An
 * entry at its exact expiry is expired. Missing, wrong-namespace, and expired entries never turn
 * `unavailable` into `empty`.
 */
export function withProviderDirectoryCache(
  directory: ProviderDirectory,
  options: ProviderDirectoryCacheOptions,
): ProviderDirectory {
  if (!Number.isFinite(options.ttlMs) || options.ttlMs <= 0) {
    throw new RangeError("provider directory cache ttlMs must be finite and greater than zero");
  }
  const now = options.now ?? Date.now;

  return {
    source: directory.source,
    cacheNamespace: directory.cacheNamespace,
    async read(): Promise<ProviderDirectoryResult> {
      const result = await directory.read();
      const currentTime = now();

      if (result.source !== directory.source) {
        options.cache.clear();
        return {
          state: "unavailable",
          source: directory.source,
          reason: "inconsistentSource",
          detail: `The directory identified itself as ${result.source} while configured as ${directory.source}`,
          attemptedAt: currentTime,
        };
      }

      if (
        result.state !== "unavailable" &&
        (!Number.isFinite(result.readAt) ||
          result.readAt < 0 ||
          (result.blockNumber !== null &&
            (!Number.isSafeInteger(result.blockNumber) || result.blockNumber < 0)) ||
          (result.expiresAt !== null && !Number.isFinite(result.expiresAt)))
      ) {
        options.cache.clear();
        return {
          state: "unavailable",
          source: directory.source,
          reason: "invalidSnapshot",
          detail: "The directory returned invalid timestamp or block-anchor metadata",
          attemptedAt: currentTime,
        };
      }

      if (result.state !== "unavailable" && result.observation === "stored") {
        // A replay is not a successful refresh. Passing it through preserves the original hard
        // deadline; writing it here would let nested cache wrappers renew stale data indefinitely.
        if (
          result.expiresAt === null ||
          !Number.isFinite(result.expiresAt) ||
          currentTime >= result.expiresAt
        ) {
          return {
            state: "unavailable",
            source: directory.source,
            reason: "invalidSnapshot",
            detail: "The directory returned a stored snapshot without an unexpired hard TTL",
            attemptedAt: currentTime,
          };
        }
        return result;
      }

      if (result.state !== "unavailable") {
        // TTL begins at the original source observation, not cache insertion. A long paged read or
        // an outer wrapper must never silently lengthen the hard maximum age.
        const localExpiry = result.readAt + options.ttlMs;
        const expiresAt =
          result.expiresAt === null ? localExpiry : Math.min(localExpiry, result.expiresAt);
        const snapshot: ProviderDirectorySnapshot & { expiresAt: number } = {
          ...result,
          expiresAt,
        };
        if (!Number.isFinite(expiresAt) || currentTime >= expiresAt) {
          options.cache.clear();
          return {
            state: "unavailable",
            source: directory.source,
            reason: "invalidSnapshot",
            detail: "The live directory snapshot was already outside its hard TTL",
            attemptedAt: currentTime,
          };
        }
        options.cache.write({
          namespace: directory.cacheNamespace,
          snapshot,
          expiresAt,
        });
        return snapshot;
      }

      const cached = options.cache.read();
      if (
        cached?.namespace !== directory.cacheNamespace ||
        cached?.snapshot.source !== directory.source
      ) {
        // A source swap is a trust-boundary change, not a cache hit.
        if (cached) options.cache.clear();
        return result;
      }
      if (
        !Number.isFinite(cached.expiresAt) ||
        cached.snapshot.expiresAt !== cached.expiresAt ||
        currentTime >= cached.expiresAt
      ) {
        options.cache.clear();
        return result;
      }

      return {
        ...cached.snapshot,
        observation: "stored",
        expiresAt: cached.expiresAt,
      };
    },
  };
}
