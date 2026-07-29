import { describe, expect, it, vi } from "vitest";
import {
  ROAX_RPC_STORAGE_KEY,
  createGuardedRoaxRpcRequest,
  getRoaxRpcPreference,
  getRoaxRpcPreferenceRevision,
  normalizeRpcUrl,
  resetRoaxRpcPreference,
  setRoaxRpcPreference,
  validateAndSaveRoaxRpcPreference,
  type RpcFetch,
  type StorageLike,
  RpcPreferenceOperationSupersededError,
} from "../src/chain/rpcEndpoint";

class MemoryStorage implements StorageLike {
  readonly values = new Map<string, string>();
  getItem(key: string) {
    return this.values.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
  removeItem(key: string) {
    this.values.delete(key);
  }
}

function rpcResponse(id: number, result: unknown): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, result }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function rpcError(id: number, code: number, message: string): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function requestBody(init?: RequestInit): {
  id: number;
  method: string;
  params: unknown[];
} {
  return JSON.parse(String(init?.body)) as {
    id: number;
    method: string;
    params: unknown[];
  };
}

describe("ROAX RPC preference", () => {
  it("persists only canonical absolute HTTP(S) URLs and reset restores the bundled default", () => {
    const storage = new MemoryStorage();
    const defaultUrl = "https://default.rpc";

    expect(normalizeRpcUrl("/relative")).toBeNull();
    expect(normalizeRpcUrl("ws://rpc.example")).toBeNull();
    expect(normalizeRpcUrl("https://rpc.example/#hidden")).toBeNull();
    expect(() => setRoaxRpcPreference("file:///tmp/rpc", defaultUrl, storage)).toThrow(
      /absolute http/i,
    );

    const selected = setRoaxRpcPreference("https://custom.rpc/path", defaultUrl, storage);
    expect(selected).toEqual({
      rpcUrl: "https://custom.rpc/path",
      defaultRpcUrl: "https://default.rpc/",
      isCustom: true,
    });
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBe("https://custom.rpc/path");

    expect(resetRoaxRpcPreference(defaultUrl, storage)).toEqual({
      rpcUrl: "https://default.rpc/",
      defaultRpcUrl: "https://default.rpc/",
      isCustom: false,
    });
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBeNull();
  });

  it("preserves path and query case, which carry API routes and tokens", async () => {
    const storage = new MemoryStorage();
    const defaultUrl = "https://default.rpc";
    const cased = "https://Rpc.Example/V1/RoutE?Token=AbCdEf";

    // Only the host is case-insensitive; a lowercased path or query would address a different
    // route, or present a different token, than the operator typed.
    expect(normalizeRpcUrl(cased)).toBe("https://rpc.example/V1/RoutE?Token=AbCdEf");

    const selected = setRoaxRpcPreference(cased, defaultUrl, storage);
    expect(selected.rpcUrl).toBe("https://rpc.example/V1/RoutE?Token=AbCdEf");
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBe(
      "https://rpc.example/V1/RoutE?Token=AbCdEf",
    );

    // The contacted peer, not merely the stored string, keeps that case.
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      calls.push({ url: String(input), method: body.method });
      return rpcResponse(body.id, body.method === "eth_chainId" ? "0x87" : "0x01");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: selected.rpcUrl,
      defaultUrl,
      fetchFn,
    });

    await expect(
      request({
        method: "eth_call",
        params: [{ to: "0x0000000000000000000000000000000000000001", data: "0x" }],
      }),
    ).resolves.toBe("0x01");
    expect(calls).toEqual([
      { url: "https://rpc.example/V1/RoutE?Token=AbCdEf", method: "eth_chainId" },
      { url: "https://rpc.example/V1/RoutE?Token=AbCdEf", method: "eth_call" },
    ]);

    resetRoaxRpcPreference(defaultUrl, storage);
  });

  it("ignores a stale malformed stored value instead of dispatching to it", () => {
    const storage = new MemoryStorage();
    storage.setItem(ROAX_RPC_STORAGE_KEY, "javascript:alert(1)");
    expect(getRoaxRpcPreference("https://default.rpc", storage)).toEqual({
      rpcUrl: "https://default.rpc/",
      defaultRpcUrl: "https://default.rpc/",
      isCustom: false,
    });
  });
});

describe("ROAX RPC chain guard", () => {
  it("sends no address-bound request to a different-chain preference and uses the guarded default", async () => {
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      const url = String(input);
      calls.push({ url, method: body.method });
      if (body.method === "eth_chainId") {
        return rpcResponse(body.id, url.includes("wrong") ? "0x1" : "0x87");
      }
      return rpcResponse(body.id, "0x01");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: "https://wrong.rpc",
      defaultUrl: "https://default.rpc",
      fetchFn,
    });

    await expect(
      request({
        method: "eth_call",
        params: [{ to: "0x0000000000000000000000000000000000000001", data: "0x" }],
      }),
    ).resolves.toBe("0x01");

    expect(calls).toEqual([
      { url: "https://wrong.rpc/", method: "eth_chainId" },
      { url: "https://default.rpc/", method: "eth_chainId" },
      { url: "https://default.rpc/", method: "eth_call" },
    ]);
    expect(calls).not.toContainEqual({ url: "https://wrong.rpc/", method: "eth_call" });
  });

  it("guards the bundled default too and sends no read when neither endpoint establishes chain 135", async () => {
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      calls.push({ url: String(input), method: body.method });
      return rpcResponse(body.id, "0x1");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: "https://wrong.rpc",
      defaultUrl: "https://also-wrong.rpc",
      fetchFn,
    });

    await expect(
      request({
        method: "eth_getLogs",
        params: [{ address: "0x0000000000000000000000000000000000000001" }],
      }),
    ).rejects.toThrow(/no blockchain read was sent/i);
    expect(calls).toEqual([
      { url: "https://wrong.rpc/", method: "eth_chainId" },
      { url: "https://also-wrong.rpc/", method: "eth_chainId" },
    ]);
  });

  it("falls back before a read when the preferred endpoint is unreachable", async () => {
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      const url = String(input);
      calls.push({ url, method: body.method });
      if (url.includes("offline")) throw new TypeError("Failed to fetch");
      if (body.method === "eth_chainId") return rpcResponse(body.id, "0x87");
      return rpcResponse(body.id, "0x10");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: "https://offline.rpc",
      defaultUrl: "https://default.rpc",
      fetchFn,
    });

    await expect(request({ method: "eth_blockNumber" })).resolves.toBe("0x10");
    expect(calls).toEqual([
      { url: "https://offline.rpc/", method: "eth_chainId" },
      { url: "https://default.rpc/", method: "eth_chainId" },
      { url: "https://default.rpc/", method: "eth_blockNumber" },
    ]);
  });

  it("re-guards and retries the default after a custom transport failure", async () => {
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      const url = String(input);
      calls.push({ url, method: body.method });
      if (body.method === "eth_chainId") return rpcResponse(body.id, "0x87");
      if (url.includes("flaky")) throw new TypeError("connection closed");
      return rpcResponse(body.id, "0x20");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: "https://flaky.rpc",
      defaultUrl: "https://default.rpc",
      fetchFn,
    });

    await expect(request({ method: "eth_blockNumber" })).resolves.toBe("0x20");
    expect(calls).toEqual([
      { url: "https://flaky.rpc/", method: "eth_chainId" },
      { url: "https://flaky.rpc/", method: "eth_blockNumber" },
      { url: "https://default.rpc/", method: "eth_chainId" },
      { url: "https://default.rpc/", method: "eth_blockNumber" },
    ]);
  });

  it("does not hide a contract JSON-RPC error by retrying it on another peer", async () => {
    const calls: Array<{ url: string; method: string }> = [];
    const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      const url = String(input);
      calls.push({ url, method: body.method });
      if (body.method === "eth_chainId") return rpcResponse(body.id, "0x87");
      return rpcError(body.id, -32_000, "execution reverted");
    }) as RpcFetch;
    const request = createGuardedRoaxRpcRequest({
      preferredUrl: "https://custom.rpc",
      defaultUrl: "https://default.rpc",
      fetchFn,
    });

    await expect(request({ method: "eth_call", params: [] })).rejects.toThrow(
      /execution reverted/i,
    );
    expect(calls).toEqual([
      { url: "https://custom.rpc/", method: "eth_chainId" },
      { url: "https://custom.rpc/", method: "eth_call" },
    ]);
  });

  it("clears a prior custom preference when a replacement is rejected", async () => {
    const storage = new MemoryStorage();
    setRoaxRpcPreference("https://old-custom.rpc", "https://default.rpc", storage);
    const fetchFn = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      return rpcResponse(body.id, "0x1");
    }) as RpcFetch;

    await expect(
      validateAndSaveRoaxRpcPreference("https://wrong-chain.rpc", {
        defaultUrl: "https://default.rpc",
        storage,
        fetchFn,
      }),
    ).rejects.toThrow(/custom endpoint was removed/i);
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBeNull();
    expect(getRoaxRpcPreference("https://default.rpc", storage).isCustom).toBe(false);
  });

  it("does not let a slow validation overwrite a newer preference change", async () => {
    const storage = new MemoryStorage();
    const defaultUrl = "https://default.rpc";
    let releaseProbe!: () => void;
    const probeGate = new Promise<void>((resolve) => {
      releaseProbe = resolve;
    });
    const fetchFn = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      await probeGate;
      return rpcResponse(body.id, "0x87");
    }) as RpcFetch;
    const expectedRevision = getRoaxRpcPreferenceRevision();
    const slowSave = validateAndSaveRoaxRpcPreference("https://slow.rpc", {
      defaultUrl,
      storage,
      fetchFn,
      expectedRevision,
    });

    setRoaxRpcPreference("https://newer.rpc", defaultUrl, storage);
    releaseProbe();

    await expect(slowSave).rejects.toBeInstanceOf(
      RpcPreferenceOperationSupersededError,
    );
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBe("https://newer.rpc/");
  });

  it("does not persist after its component operation is cancelled or unmounted", async () => {
    const storage = new MemoryStorage();
    let current = true;
    let releaseProbe!: () => void;
    const probeGate = new Promise<void>((resolve) => {
      releaseProbe = resolve;
    });
    const fetchFn = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      const body = requestBody(init);
      await probeGate;
      return rpcResponse(body.id, "0x87");
    }) as RpcFetch;
    const slowSave = validateAndSaveRoaxRpcPreference("https://slow.rpc", {
      defaultUrl: "https://default.rpc",
      storage,
      fetchFn,
      expectedRevision: getRoaxRpcPreferenceRevision(),
      shouldApply: () => current,
    });

    current = false;
    releaseProbe();

    await expect(slowSave).rejects.toBeInstanceOf(
      RpcPreferenceOperationSupersededError,
    );
    expect(storage.getItem(ROAX_RPC_STORAGE_KEY)).toBeNull();
  });
});
