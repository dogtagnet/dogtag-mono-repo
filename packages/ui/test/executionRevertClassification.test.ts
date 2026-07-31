// A NODE-LEVEL ERROR IS NOT A CONTRACT ANSWER.
//
// The generation probe treats "the contract executed this call and reverted" as evidence that the
// authority is generation 1 - a `IssuerRegistry` has no `isRecognizedIssuer` and no fallback, so its
// dispatcher reverts. That conclusion then leaves an EMPTY grant history standing as a definite
// refusal, which for a genuine credential is a forgery verdict. So anything that is NOT a revert must
// never reach it: a rate limit, an internal error, a method-not-found, a dropped connection.
//
// The first cut walked for viem's `ContractFunctionRevertedError`, which is NOT revert-specific -
// `getContractError` folds both `code === 3` and `InternalRpcError.code` (-32603) into it. These cases
// drive REAL viem error objects through a throwing transport (no network) so the classification is
// pinned against the pinned viem's actual behaviour rather than against a reading of its source.
import { createPublicClient, custom, encodeFunctionData } from "viem";
import { describe, expect, it } from "vitest";
import {
  answeredWithExecutionRevert,
  generationFromProbeData,
} from "../src/wallet/contracts";

const ADDR = "0x1111111111111111111111111111111111111111" as const;
const ABI = [
  {
    type: "function",
    name: "isRecognizedIssuer",
    stateMutability: "view",
    inputs: [
      { name: "service", type: "address" },
      { name: "signer", type: "address" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
] as const;

/** A JSON-RPC error shaped the way a node returns one, put through viem's real error pipeline. */
async function classify(thrown: unknown): Promise<boolean> {
  const client = createPublicClient({
    // No retries: this pins the CLASSIFICATION, and viem's default backoff would spend seconds
    // re-throwing the same error before the assertion could see it.
    transport: custom(
      {
        async request() {
          throw thrown;
        },
      },
      { retryCount: 0 },
    ),
  });
  try {
    await client.call({
      to: ADDR,
      data: encodeFunctionData({
        abi: ABI,
        functionName: "isRecognizedIssuer",
        args: [ADDR, ADDR],
      }),
    });
    return false;
  } catch (e) {
    return answeredWithExecutionRevert(e);
  }
}

const rpcError = (code: number, message: string) =>
  Object.assign(new Error(message), { code, message });

describe("only an execution revert is a contract answer", () => {
  it("classifies geth's execution-reverted code as a revert", async () => {
    // Confirmed against ROAX with the exact production case: `isRecognizedIssuer` put to the deployed
    // generation-1 IssuerRegistry answers {"code":3,"message":"execution reverted","data":"0x"}.
    expect(await classify(rpcError(3, "execution reverted"))).toBe(true);
  });

  it("classifies the canonical message as a revert under another code", async () => {
    // Several clients spell an execution revert -32000. Without this the pillar would stop refusing
    // every never-granted generation-1 signer against such a peer.
    expect(await classify(rpcError(-32000, "execution reverted"))).toBe(true);
  });

  it.each([
    [-32005, "limit exceeded"],
    [-32603, "internal error"],
    [-32601, "the method does not exist/is not available"],
    [-32002, "resource unavailable"],
  ])("does NOT classify node error %i as a revert", async (code, message) => {
    // Each of these is the node speaking about ITSELF. -32603 is the one that matters most here: it
    // is precisely the code `ContractFunctionRevertedError` folds in, so walking for that class read
    // an internal error as a contract refusal.
    expect(await classify(rpcError(code as number, message as string))).toBe(false);
  });

  it("does NOT classify a transport failure as a revert", async () => {
    expect(await classify(new TypeError("fetch failed"))).toBe(false);
  });

  it("is not vacuously false - a non-viem value is simply not a revert", () => {
    // `walk` returns null on no match, so a truthy-looking implementation would still be caught by
    // the revert case above; this pins the guard clause rather than leaving it to inference.
    expect(answeredWithExecutionRevert(new Error("plain"))).toBe(false);
    expect(answeredWithExecutionRevert(undefined)).toBe(false);
  });
});

describe("a probe that did not throw is only the successor if something answered", () => {
  it("treats a returned word as the successor", () => {
    expect(generationFromProbeData(`0x${"0".repeat(63)}1`)).toBe("successor");
  });

  it.each(["0x", undefined])(
    "treats EMPTY returndata (%s) as undetermined, never the successor",
    (data) => {
      // An address with no code answers empty WITHOUT reverting. Switching the probe from
      // `readContract` to `call` is what made that reachable: `readContract` raised
      // `ContractFunctionZeroDataError` for it, while `call` simply resolves. Reading it as
      // "successor" would suppress the definite refusal for every never-granted signer whose
      // authority address points at nothing.
      expect(generationFromProbeData(data)).toBe("undetermined");
      expect(generationFromProbeData(data)).not.toBe("successor");
      expect(generationFromProbeData(data)).not.toBe("legacy");
    },
  );
});
