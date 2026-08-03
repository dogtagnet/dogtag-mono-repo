/**
 * The groomer portal's provider self-service page (registry-plan S-15).
 *
 * ONLY the listing flow, and that is a real split rather than a trimmed copy. A groomer VERIFIES and
 * does not ISSUE - `BUSINESS_TYPE=groomer` mounts no issuance routes at all - so a groomer has no
 * `DogTagIssuer` clone. Deploying a clone, choosing which clone is current, and claiming a domain
 * for a clone are all keyed BY one, so for a groomer they are inapplicable rather than merely
 * hidden. Publishing a listing is keyed by `providerId`, so it applies to every provider, and a
 * groomer appears in the provider directory exactly as a vet does.
 *
 * Without this page a groomer would have no way to publish its contacts or location at all, which
 * is why the flows were split instead of the page being mounted for vets alone.
 */

import { ProviderSelfServiceFlows, type ProviderContracts } from "@dogtag/ui";
import { useMemo } from "react";
import { env } from "../lib/env";

function readContracts(): { contracts: ProviderContracts; missing: string[] } {
  // The domain resolver and the factory are required by the shared component's config check even
  // though this portal offers neither flow: they are part of one deployed set, and letting a
  // groomer run with half of it configured would make a later capability change silently
  // half-broken rather than loudly unconfigured.
  const named: [string, string][] = [
    ["VITE_PROVIDER_REGISTRY_ADDR", env.providerRegistryAddr],
    ["VITE_DOGTAG_ISSUER_FACTORY_ADDR", env.issuerFactoryAddr],
    ["VITE_SERVICE_DOMAIN_RESOLVER_ADDR", env.serviceDomainResolverAddr],
    ["VITE_PROVIDER_DIRECTORY_ADDR", env.providerDirectoryAddr],
  ];
  const contracts: ProviderContracts = {
    core: env.providerRegistryAddr as `0x${string}`,
    factory: env.issuerFactoryAddr as `0x${string}`,
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
      mirrorBase={env.contentMirrorBase}
      mirrorToken={env.contentMirrorToken}
      demoMode={env.demoMode}
      capabilities={{ issuance: false, listing: true }}
    />
  );
}
