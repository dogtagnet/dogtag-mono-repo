/**
 * The vet portal's provider self-service page (registry-plan S-15).
 *
 * A vet ISSUES, so all four flows apply: deploy your own clone, choose which contract is current,
 * claim a domain for it, and publish your listing. The groomer portal mounts the SAME shared
 * component with `issuance: false` - see `ProviderSelfServiceFlows` for why that split is real
 * rather than cosmetic.
 *
 * ADDRESSES COME FROM CONFIG WITH NO FALLBACK. Unset makes the page report itself unconfigured
 * rather than reading a baked constant. That is the `VITE_ISSUER_DOMAIN_REGISTRY_ADDR` precedent and
 * it matters more here: the generation-2 set is deployed but UNWIRED, and client repointing is
 * C-9/C-10, so a default baked into this file would be a repoint by accident.
 */

import { ProviderSelfServiceFlows, type ProviderContracts } from "@dogtag/ui";
import { useMemo } from "react";
import { env } from "../lib/env";

function readContracts(): { contracts: ProviderContracts; missing: string[] } {
  const named: [string, string][] = [
    ["VITE_PROVIDER_REGISTRY_ADDR", env.providerRegistryAddr],
    ["VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR", env.issuerFactoryV2Addr],
    ["VITE_SERVICE_DOMAIN_RESOLVER_ADDR", env.serviceDomainResolverAddr],
    ["VITE_PROVIDER_DIRECTORY_ADDR", env.providerDirectoryAddr],
  ];
  // Built explicitly rather than folded, so adding a field to `ProviderContracts` is a compile error
  // here instead of an address that silently arrives as `undefined`.
  const contracts: ProviderContracts = {
    core: env.providerRegistryAddr as `0x${string}`,
    factory: env.issuerFactoryV2Addr as `0x${string}`,
    domainResolver: env.serviceDomainResolverAddr as `0x${string}`,
    directory: env.providerDirectoryAddr as `0x${string}`,
  };
  return { contracts, missing: named.filter(([, v]) => !v).map(([n]) => n) };
}

export default function ProviderSelfService() {
  const { contracts, missing } = useMemo(() => readContracts(), []);
  return (
    <ProviderSelfServiceFlows
      contracts={contracts}
      missingConfig={missing}
      rpcUrl={env.roaxRpc}
      defaultProviderId={env.providerId}
      capabilities={{ issuance: true, listing: true }}
    />
  );
}
