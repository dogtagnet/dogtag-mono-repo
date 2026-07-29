import { custom, type Transport } from "viem";
import { ROAX_CHAIN_ID } from "../wallet/chain";

export const DEFAULT_ROAX_RPC_URL = "https://devrpc.roax.net";
export const ROAX_RPC_STORAGE_KEY = "dogtag.roax-rpc-url.v1";

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export type RpcFetch = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

export interface RoaxRpcPreference {
  /** The endpoint reads should try first. */
  rpcUrl: string;
  /** The app-bundled endpoint used when no preference exists and as the guarded fallback. */
  defaultRpcUrl: string;
  isCustom: boolean;
}

export type RpcEndpointFailure =
  | { kind: "invalid-url"; message: string }
  | { kind: "unreachable"; message: string }
  | { kind: "wrong-chain"; chainId: number; message: string };

export interface ResolvedRoaxRpcEndpoint {
  url: string;
  source: "preferred" | "default";
  preferredFailure?: RpcEndpointFailure;
}

export interface ResolveRoaxRpcEndpointOptions {
  preferredUrl?: string;
  defaultUrl?: string;
  expectedChainId?: number;
  fetchFn?: RpcFetch;
  timeoutMs?: number;
}

const preferenceListeners = new Set<() => void>();
let preferenceRevision = 0;
let listeningForStorage = false;
let nextRpcId = 1;

function notifyPreferenceChanged() {
  preferenceRevision += 1;
  preferenceListeners.forEach((listener) => listener());
}

function onPreferenceStorage(event: StorageEvent) {
  if (event.key === ROAX_RPC_STORAGE_KEY) notifyPreferenceChanged();
}

function browserStorage(): StorageLike | undefined {
  if (typeof window === "undefined") return undefined;
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/**
 * Canonicalise an absolute HTTP(S) endpoint. Anything else is rejected before it can be persisted or
 * contacted: relative URLs would silently follow the portal origin, while non-HTTP schemes are not
 * JSON-RPC endpoints this browser client can safely dispatch to.
 */
export function normalizeRpcUrl(value: string): string | null {
  try {
    const url = new URL(value.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    // Fragments are client-side only and are never sent by fetch. Persisting one would display an
    // endpoint different from the peer actually contacted.
    if (url.hash) return null;
    return url.href;
  } catch {
    return null;
  }
}

/** Read the per-browser endpoint choice, falling back to the app's bundled endpoint. */
export function getRoaxRpcPreference(
  defaultRpcUrl: string = DEFAULT_ROAX_RPC_URL,
  storage: StorageLike | undefined = browserStorage(),
): RoaxRpcPreference {
  const normalizedDefault = normalizeRpcUrl(defaultRpcUrl) ?? DEFAULT_ROAX_RPC_URL;
  let stored: string | null = null;
  try {
    stored = storage?.getItem(ROAX_RPC_STORAGE_KEY) ?? null;
  } catch {
    // A blocked/failed localStorage read is the same as no preference: use the bundled endpoint.
  }
  const normalizedStored = stored ? normalizeRpcUrl(stored) : null;
  const isCustom = Boolean(normalizedStored && normalizedStored !== normalizedDefault);
  return {
    rpcUrl: isCustom ? normalizedStored! : normalizedDefault,
    defaultRpcUrl: normalizedDefault,
    isCustom,
  };
}

/**
 * Persist a custom endpoint. Passing the bundled endpoint removes the override so a later app
 * deployment can change its default without an old copy shadowing it forever.
 */
export function setRoaxRpcPreference(
  value: string,
  defaultRpcUrl: string = DEFAULT_ROAX_RPC_URL,
  storage: StorageLike | undefined = browserStorage(),
): RoaxRpcPreference {
  const normalized = normalizeRpcUrl(value);
  if (!normalized) {
    throw new Error("Enter an absolute http:// or https:// JSON-RPC URL.");
  }
  const normalizedDefault = normalizeRpcUrl(defaultRpcUrl);
  if (!normalizedDefault) {
    throw new Error("The bundled blockchain endpoint is not a valid absolute HTTP(S) URL.");
  }
  try {
    if (normalized === normalizedDefault) {
      storage?.removeItem(ROAX_RPC_STORAGE_KEY);
    } else {
      storage?.setItem(ROAX_RPC_STORAGE_KEY, normalized);
    }
  } catch {
    throw new Error("This browser did not allow the blockchain endpoint preference to be saved.");
  }
  notifyPreferenceChanged();
  return getRoaxRpcPreference(normalizedDefault, storage);
}

export function resetRoaxRpcPreference(
  defaultRpcUrl: string = DEFAULT_ROAX_RPC_URL,
  storage: StorageLike | undefined = browserStorage(),
): RoaxRpcPreference {
  try {
    storage?.removeItem(ROAX_RPC_STORAGE_KEY);
  } catch {
    throw new Error("This browser did not allow the blockchain endpoint preference to be reset.");
  }
  notifyPreferenceChanged();
  return getRoaxRpcPreference(defaultRpcUrl, storage);
}

/** Subscribe to same-tab saves and cross-tab localStorage changes. */
export function subscribeRoaxRpcPreference(listener: () => void): () => void {
  preferenceListeners.add(listener);
  if (typeof window !== "undefined" && !listeningForStorage) {
    window.addEventListener("storage", onPreferenceStorage);
    listeningForStorage = true;
  }
  return () => {
    preferenceListeners.delete(listener);
    if (
      typeof window !== "undefined" &&
      listeningForStorage &&
      preferenceListeners.size === 0
    ) {
      window.removeEventListener("storage", onPreferenceStorage);
      listeningForStorage = false;
    }
  };
}

/** Monotonic version used to stop an in-flight validation overwriting a newer tab/user choice. */
export function getRoaxRpcPreferenceRevision(): number {
  return preferenceRevision;
}

class JsonRpcResponseError extends Error {}

async function jsonRpcRequest(
  url: string,
  method: string,
  params: readonly unknown[] | undefined,
  fetchFn: RpcFetch,
  timeoutMs: number,
): Promise<unknown> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetchFn(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: nextRpcId++,
        method,
        params: params ?? [],
      }),
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    const body = (await response.json()) as unknown;
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      throw new Error("malformed JSON-RPC response");
    }
    const envelope = body as { result?: unknown; error?: { code?: unknown; message?: unknown } };
    if (envelope.error) {
      const code =
        typeof envelope.error.code === "number" ? ` (${envelope.error.code})` : "";
      const message =
        typeof envelope.error.message === "string"
          ? envelope.error.message
          : "JSON-RPC request failed";
      throw new JsonRpcResponseError(`${message}${code}`);
    }
    if (!Object.prototype.hasOwnProperty.call(envelope, "result")) {
      throw new Error("malformed JSON-RPC response");
    }
    return envelope.result;
  } finally {
    clearTimeout(timeout);
  }
}

function parseChainId(result: unknown): number | null {
  if (typeof result !== "string" || !/^0x[0-9a-f]+$/i.test(result)) return null;
  try {
    const chainId = BigInt(result);
    if (chainId < 0n || chainId > BigInt(Number.MAX_SAFE_INTEGER)) return null;
    return Number(chainId);
  } catch {
    return null;
  }
}

async function probeChain(
  url: string,
  expectedChainId: number,
  fetchFn: RpcFetch,
  timeoutMs: number,
): Promise<{ ok: true } | { ok: false; failure: RpcEndpointFailure }> {
  try {
    const result = await jsonRpcRequest(url, "eth_chainId", [], fetchFn, timeoutMs);
    const chainId = parseChainId(result);
    if (chainId === null) {
      return {
        ok: false,
        failure: {
          kind: "unreachable",
          message: "The endpoint returned a malformed eth_chainId response.",
        },
      };
    }
    if (chainId !== expectedChainId) {
      return {
        ok: false,
        failure: {
          kind: "wrong-chain",
          chainId,
          message: `The endpoint reports chain ${chainId}; DogTag's bundled contracts are for chain ${expectedChainId}.`,
        },
      };
    }
    return { ok: true };
  } catch (cause) {
    return {
      ok: false,
      failure: {
        kind: "unreachable",
        message:
          cause instanceof Error
            ? `The endpoint could not establish chain ${expectedChainId}: ${cause.message}`
            : `The endpoint could not establish chain ${expectedChainId}.`,
      },
    };
  }
}

/**
 * Establish a chain-correct endpoint before any address-bound request.
 *
 * A wrong, malformed or unreachable preferred endpoint receives only `eth_chainId`; the bundled
 * default is then independently guarded. If neither proves it is the bundled chain, this throws and
 * callers retain their existing unavailable/error semantics instead of reading an address on the
 * wrong chain.
 */
export async function resolveRoaxRpcEndpoint(
  options: ResolveRoaxRpcEndpointOptions,
): Promise<ResolvedRoaxRpcEndpoint> {
  const expectedChainId = options.expectedChainId ?? ROAX_CHAIN_ID;
  const timeoutMs = options.timeoutMs ?? 10_000;
  const fetchFn = options.fetchFn ?? globalThis.fetch.bind(globalThis);
  const rawDefault = options.defaultUrl ?? DEFAULT_ROAX_RPC_URL;
  const defaultUrl = normalizeRpcUrl(rawDefault);
  if (!defaultUrl) {
    throw new Error("The bundled blockchain endpoint is not a valid absolute HTTP(S) URL.");
  }

  const rawPreferred = options.preferredUrl?.trim() || defaultUrl;
  const preferredUrl = normalizeRpcUrl(rawPreferred);
  let preferredFailure: RpcEndpointFailure | undefined;

  if (!preferredUrl) {
    preferredFailure = {
      kind: "invalid-url",
      message: "The chosen endpoint is not an absolute HTTP(S) URL.",
    };
  } else {
    const preferredProbe = await probeChain(
      preferredUrl,
      expectedChainId,
      fetchFn,
      timeoutMs,
    );
    if (preferredProbe.ok) {
      return {
        url: preferredUrl,
        source: preferredUrl === defaultUrl ? "default" : "preferred",
      };
    }
    preferredFailure = preferredProbe.failure;
  }

  // Do not probe the same failed endpoint twice when the preference already was the default.
  if (preferredUrl !== defaultUrl) {
    const defaultProbe = await probeChain(defaultUrl, expectedChainId, fetchFn, timeoutMs);
    if (defaultProbe.ok) {
      return { url: defaultUrl, source: "default", preferredFailure };
    }
  }

  const preferredMessage = preferredFailure?.message ?? "The chosen endpoint was unavailable.";
  throw new Error(
    `${preferredMessage} The bundled endpoint also could not establish ROAX chain ${expectedChainId}; no blockchain read was sent.`,
  );
}

export interface ValidateAndSaveRoaxRpcPreferenceOptions
  extends Omit<ResolveRoaxRpcEndpointOptions, "preferredUrl" | "defaultUrl"> {
  defaultUrl?: string;
  storage?: StorageLike;
  /** Reject the write if another local/cross-tab preference change happened during the probe. */
  expectedRevision?: number;
  /** Component operation-generation/unmount guard, checked before any preference mutation. */
  shouldApply?: () => boolean;
}

export class RpcPreferenceOperationSupersededError extends Error {
  constructor() {
    super("Blockchain endpoint check was superseded by a newer settings action.");
    this.name = "RpcPreferenceOperationSupersededError";
  }
}

function preferenceOperationIsCurrent(
  options: ValidateAndSaveRoaxRpcPreferenceOptions,
): boolean {
  return (
    (options.expectedRevision === undefined ||
      options.expectedRevision === preferenceRevision) &&
    (options.shouldApply?.() ?? true)
  );
}

/**
 * Validate a candidate and persist it only when it establishes the bundled chain. Every rejection
 * clears an older override immediately, so an invalid edit can never leave a previously chosen peer
 * silently active behind an error message.
 */
export async function validateAndSaveRoaxRpcPreference(
  value: string,
  options: ValidateAndSaveRoaxRpcPreferenceOptions = {},
): Promise<RoaxRpcPreference> {
  const defaultUrl = options.defaultUrl ?? DEFAULT_ROAX_RPC_URL;
  try {
    if (!preferenceOperationIsCurrent(options)) {
      throw new RpcPreferenceOperationSupersededError();
    }
    const candidate = normalizeRpcUrl(value);
    if (!candidate) {
      throw new Error("Enter an absolute http:// or https:// JSON-RPC URL without a fragment.");
    }
    const resolved = await resolveRoaxRpcEndpoint({
      ...options,
      preferredUrl: candidate,
      defaultUrl,
    });
    if (resolved.url !== candidate) {
      throw new Error(resolved.preferredFailure?.message ?? "That endpoint could not be used.");
    }
    if (!preferenceOperationIsCurrent(options)) {
      throw new RpcPreferenceOperationSupersededError();
    }
    return setRoaxRpcPreference(candidate, defaultUrl, options.storage ?? browserStorage());
  } catch (cause) {
    if (
      cause instanceof RpcPreferenceOperationSupersededError ||
      !preferenceOperationIsCurrent(options)
    ) {
      throw new RpcPreferenceOperationSupersededError();
    }
    resetRoaxRpcPreference(defaultUrl, options.storage ?? browserStorage());
    const message = cause instanceof Error ? cause.message : "The endpoint could not be checked.";
    throw new Error(
      `${message} The custom endpoint was removed; blockchain reads use the bundled default.`,
    );
  }
}

export interface GuardedRoaxRpcRequestOptions extends ResolveRoaxRpcEndpointOptions {}

/**
 * EIP-1193 request function used under viem. Every request re-runs the immediate chain guard; a node
 * that changes networks after an earlier successful read therefore cannot inherit stale trust.
 */
export function createGuardedRoaxRpcRequest(options: GuardedRoaxRpcRequestOptions) {
  const expectedChainId = options.expectedChainId ?? ROAX_CHAIN_ID;
  const timeoutMs = options.timeoutMs ?? 10_000;
  const fetchFn = options.fetchFn ?? globalThis.fetch.bind(globalThis);
  return async ({
    method,
    params,
  }: {
    method: string;
    params?: readonly unknown[];
  }): Promise<unknown> => {
    const endpoint = await resolveRoaxRpcEndpoint({
      ...options,
      expectedChainId,
      timeoutMs,
      fetchFn,
    });
    // The probe already obtained this exact answer. Returning it avoids a second, unguarded request.
    if (method === "eth_chainId") return `0x${expectedChainId.toString(16)}`;
    try {
      return await jsonRpcRequest(endpoint.url, method, params, fetchFn, timeoutMs);
    } catch (cause) {
      // A legitimate JSON-RPC error (for example a contract revert) is an answer from this chain,
      // not endpoint failure; retrying elsewhere would mask it. Transport/HTTP/malformed-envelope
      // failures from a custom peer do fail over, after independently guarding the default.
      if (endpoint.source === "default" || cause instanceof JsonRpcResponseError) throw cause;
      const guardedDefault = await resolveRoaxRpcEndpoint({
        ...options,
        preferredUrl: options.defaultUrl ?? DEFAULT_ROAX_RPC_URL,
        defaultUrl: options.defaultUrl ?? DEFAULT_ROAX_RPC_URL,
        expectedChainId,
        timeoutMs,
        fetchFn,
      });
      return jsonRpcRequest(guardedDefault.url, method, params, fetchFn, timeoutMs);
    }
  };
}

/** A viem transport whose first network action for every read is the chain guard above. */
export function guardedRoaxTransport(
  preferredUrl?: string,
  defaultUrl: string = DEFAULT_ROAX_RPC_URL,
): Transport {
  return custom(
    {
      request: createGuardedRoaxRpcRequest({ preferredUrl, defaultUrl }),
    },
    {
      key: `dogtag-roax-guard:${preferredUrl ?? defaultUrl}:${defaultUrl}`,
      name: "DogTag ROAX chain-guarded JSON-RPC",
      retryCount: 0,
    },
  );
}
