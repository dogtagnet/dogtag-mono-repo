import {
  getRoaxRpcPreference,
  isWhitelistedFor as readIsWhitelistedFor,
  recordTypeKey,
} from "@dogtag/ui";
import { env } from "./env";

export { recordTypeKey };

export function whitelistConfigured(): boolean {
  return Boolean(env.issuerRegistryAddr);
}

/** Reads on-chain IssuerRegistry.isWhitelistedFor(keccak256(recordType), address). */
export async function isWhitelistedFor(recordType: string, address: string): Promise<boolean> {
  if (!whitelistConfigured()) {
    throw new Error("VITE_ISSUER_REGISTRY_ADDR not configured");
  }
  const rpc = getRoaxRpcPreference(env.roaxRpc);
  return readIsWhitelistedFor({
    registryAddr: env.issuerRegistryAddr,
    address,
    recordType,
    rpcUrl: rpc.rpcUrl,
    defaultRpcUrl: rpc.defaultRpcUrl,
  });
}
