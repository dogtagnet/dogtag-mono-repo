/**
 * The provider self-service page (registry-plan S-15).
 *
 * Four flows against the generation-2 registry set, all driven from the PROVIDER'S OWN WALLET: the
 * provider deploys its own contract, selects which of its contracts is current, claims a domain,
 * and publishes its contacts and (optionally) its location.
 *
 * There is deliberately NO BACKEND on this path. Every write is a wallet transaction to a contract
 * the provider controls, and every read goes straight to the chain - the same posture as the owner
 * wallet, and for the same reason: nothing here is DogTag's to hold on the provider's behalf.
 *
 * ADDRESSES COME FROM CONFIG WITH NO FALLBACK. Unset means this page reports itself unavailable
 * rather than reading a baked constant. That is the `VITE_ISSUER_DOMAIN_REGISTRY_ADDR` precedent and
 * it matters more here: the generation-2 set is DEPLOYED BUT UNWIRED, and client repointing is
 * C-9/C-10, so a default baked into this file would be a repoint by accident - exactly what those
 * steps exist to do deliberately and reviewably.
 */

import {
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  assessCandidateClone,
  assessDomainClaim,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  CloneLifecycleCard,
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  createLiveProviderReader,
  DeployPlanCard,
  DirectoryPublicationCard,
  DomainClaimCard,
  Input,
  Label,
  planCloneDeployment,
  planDirectoryPublication,
  REPOINT_SCOPE_NOTICE,
  validateDomain,
  type CloneAssessment,
  type DeployPlan,
  type DirectoryPublicationPlan,
  type DomainClaimAssessment,
  type ProviderContracts,
} from "@dogtag/ui";
import { useCallback, useMemo, useState } from "react";
import { keccak256, toHex } from "viem";
import { useAccount } from "wagmi";
import { env } from "../lib/env";

/** Every generation-2 address this page needs. All required; none defaulted. */
function readContracts(): { contracts: ProviderContracts; missing: string[] } {
  const entries: [keyof ProviderContracts, string, string][] = [
    ["core", "VITE_PROVIDER_REGISTRY_ADDR", env.providerRegistryAddr],
    ["factory", "VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR", env.issuerFactoryV2Addr],
    ["domainResolver", "VITE_SERVICE_DOMAIN_RESOLVER_ADDR", env.serviceDomainResolverAddr],
    ["directory", "VITE_PROVIDER_DIRECTORY_ADDR", env.providerDirectoryAddr],
  ];
  const missing = entries.filter(([, , v]) => !v).map(([, name]) => name);
  // Built explicitly rather than folded, so adding a field to `ProviderContracts` is a compile error
  // here instead of an address that silently arrives as `undefined`.
  const contracts: ProviderContracts = {
    core: env.providerRegistryAddr as `0x${string}`,
    factory: env.issuerFactoryV2Addr as `0x${string}`,
    domainResolver: env.serviceDomainResolverAddr as `0x${string}`,
    directory: env.providerDirectoryAddr as `0x${string}`,
  };
  return { contracts, missing };
}

const BLANK_CONTACTS = { phone: "", whatsapp: "", telegram: "", email: "", website: "" };

export default function ProviderSelfService() {
  const { address, isConnected } = useAccount();
  const { contracts, missing } = useMemo(() => readContracts(), []);

  const [providerId, setProviderId] = useState(env.providerId);
  const [recordType, setRecordType] = useState("VACCINATION");
  const [cloneNonce, setCloneNonce] = useState("0");
  const [candidate, setCandidate] = useState("");
  const [domain, setDomain] = useState("");
  const [latInput, setLatInput] = useState("");
  const [lngInput, setLngInput] = useState("");
  const [contacts, setContacts] = useState(BLANK_CONTACTS);

  const [deploy, setDeploy] = useState<DeployPlan | null>(null);
  const [clone, setClone] = useState<CloneAssessment | null>(null);
  const [domainState, setDomainState] = useState<DomainClaimAssessment | null>(null);
  const [publication, setPublication] = useState<DirectoryPublicationPlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reader = useMemo(
    () => (missing.length ? null : createLiveProviderReader({ contracts, rpcUrl: env.roaxRpc })),
    [contracts, missing.length],
  );

  const recordTypeKey = useMemo(
    () => keccak256(toHex(recordType)) as `0x${string}`,
    [recordType],
  );

  const run = useCallback(
    async (fn: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await fn();
      } catch (e) {
        // A thrown error here is a fault in THIS page, not a verdict about the provider: every
        // chain read the engine makes is already caught and reported as its own could-not-run.
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  if (missing.length) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Provider self-service is not configured</CardTitle>
          <CardDescription>
            This page reads the generation-2 registry set, and the addresses are not set on this
            deployment. Nothing about your provider record has been checked.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm">Set: {missing.join(", ")}</p>
          <p className="mt-2 text-xs text-muted-foreground">
            There is deliberately no built-in default. The generation-2 contracts are deployed but no
            client reads them yet, so a baked address here would repoint this deployment by accident.
          </p>
        </CardContent>
      </Card>
    );
  }

  const providerIdOk = /^0x[0-9a-fA-F]{40}$/.test(providerId);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <Card>
        <CardHeader>
          <CardTitle>Your provider record</CardTitle>
          <CardDescription>
            Your provider id is assigned by DogTag when you are approved. It is an opaque identifier -
            it is not derived from your name, your domain, your address or any of your keys.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Label htmlFor="providerId">Provider id</Label>
            <Input
              id="providerId"
              value={providerId}
              onChange={(e) => setProviderId(e.target.value)}
              placeholder="0x…"
              className="font-mono"
            />
            {providerId && !providerIdOk ? (
              <p className="mt-1 text-xs text-red-700 dark:text-red-400">
                A provider id is 20 bytes: 0x followed by 40 hex characters.
              </p>
            ) : null}
          </div>
          <div>
            <Label htmlFor="recordType">Record type</Label>
            <Input
              id="recordType"
              value={recordType}
              onChange={(e) => setRecordType(e.target.value)}
            />
          </div>
          {!isConnected ? (
            <p className="text-sm text-amber-700 dark:text-amber-400">
              Connect the wallet that owns (or will own) your contracts. Every action on this page is
              signed by you.
            </p>
          ) : (
            <p className="break-all font-mono text-xs text-muted-foreground">{address}</p>
          )}
          {error ? (
            <p className="text-sm text-red-700 dark:text-red-400" data-testid="page-error">
              {error}
            </p>
          ) : null}
        </CardContent>
      </Card>

      {/* ---- Flow 1: deploy your own contract ---------------------------------------------- */}
      <Card>
        <CardHeader>
          <CardTitle>1. Deploy your own contract</CardTitle>
          <CardDescription>
            You deploy it and you own it. DogTag does not deploy it for you.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Label htmlFor="cloneNonce">Contract number</Label>
            <Input
              id="cloneNonce"
              value={cloneNonce}
              onChange={(e) => setCloneNonce(e.target.value)}
              inputMode="numeric"
            />
            <p className="mt-1 text-xs text-muted-foreground">
              You can have more than one contract per record type. The number is what gives you
              somewhere to move to later.
            </p>
          </div>
          <Button
            disabled={busy || !reader || !isConnected || !providerIdOk}
            onClick={() =>
              run(async () => {
                setDeploy(
                  await planCloneDeployment(
                    {
                      providerId: providerId as `0x${string}`,
                      recordType: recordTypeKey,
                      caller: address as `0x${string}`,
                      cloneNonce: BigInt(cloneNonce || "0"),
                    },
                    reader!,
                  ),
                );
              })
            }
          >
            Check what this would deploy
          </Button>
          {deploy ? (
            <DeployPlanCard plan={deploy} attachmentNotice={ATTACHMENT_IS_NOT_SELF_SERVICE} />
          ) : null}
        </CardContent>
      </Card>

      {/* ---- Flow 2: select which contract is current -------------------------------------- */}
      <Card>
        <CardHeader>
          <CardTitle>2. Choose which contract is current</CardTitle>
          <CardDescription>{REPOINT_SCOPE_NOTICE}</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Label htmlFor="candidate">Contract address</Label>
            <Input
              id="candidate"
              value={candidate}
              onChange={(e) => setCandidate(e.target.value)}
              placeholder="0x…"
              className="font-mono"
            />
            <p className="mt-1 text-xs text-muted-foreground">
              Only a contract deployed by the DogTag issuer factory can be entered here. Anything else
              is refused, whoever asks.
            </p>
          </div>
          <Button
            disabled={busy || !reader || !isConnected || !providerIdOk || !candidate}
            onClick={() =>
              run(async () => {
                setClone(
                  await assessCandidateClone(
                    {
                      candidate: candidate as `0x${string}`,
                      caller: address as `0x${string}`,
                      providerId: providerId as `0x${string}`,
                    },
                    reader!,
                  ),
                );
              })
            }
          >
            Check this contract
          </Button>
          {clone ? <CloneLifecycleCard assessment={clone} /> : null}
        </CardContent>
      </Card>

      {/* ---- Flow 3: claim a domain -------------------------------------------------------- */}
      <Card>
        <CardHeader>
          <CardTitle>3. Your domain</CardTitle>
          <CardDescription>
            Publishing no domain, and never having said, are different things here - and both are
            recorded as themselves.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Label htmlFor="domain">Domain</Label>
            <Input
              id="domain"
              value={domain}
              onChange={(e) => setDomain(e.target.value)}
              placeholder="clinic.example.sg"
            />
            {domain && !validateDomain(domain).ok ? (
              <p className="mt-1 text-xs text-red-700 dark:text-red-400">
                {(validateDomain(domain) as { ok: false; reason: string }).reason}
              </p>
            ) : null}
          </div>
          <Button
            disabled={busy || !reader || !isConnected || !candidate}
            onClick={() =>
              run(async () => {
                setDomainState(
                  await assessDomainClaim(
                    candidate as `0x${string}`,
                    address as `0x${string}`,
                    reader!,
                  ),
                );
              })
            }
          >
            Check the domain record for the contract above
          </Button>
          {domainState ? <DomainClaimCard assessment={domainState} /> : null}
        </CardContent>
      </Card>

      {/* ---- Flow 4: publish your listing -------------------------------------------------- */}
      <Card>
        <CardHeader>
          <CardTitle>4. Your listing</CardTitle>
          <CardDescription>
            A location is optional. Leave both fields blank if you do not publish one.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="grid gap-3 sm:grid-cols-2">
            {(["phone", "whatsapp", "telegram", "email", "website"] as const).map((channel) => (
              // There are five channels in a two-column grid, so the last one would sit half-width
              // alone on its own row. A URL is also the longest value here, so spanning is the right
              // shape rather than only the tidier one.
              <div key={channel} className={channel === "website" ? "sm:col-span-2" : undefined}>
                <Label htmlFor={channel} className="capitalize">
                  {channel}
                </Label>
                <Input
                  id={channel}
                  value={contacts[channel]}
                  onChange={(e) => setContacts({ ...contacts, [channel]: e.target.value })}
                />
              </div>
            ))}
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <Label htmlFor="lat">Latitude (optional)</Label>
              <Input id="lat" value={latInput} onChange={(e) => setLatInput(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="lng">Longitude (optional)</Label>
              <Input id="lng" value={lngInput} onChange={(e) => setLngInput(e.target.value)} />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">{CONTACT_ONLY_NOTICE}</p>
          <Button
            disabled={busy || !reader || !isConnected || !providerIdOk}
            onClick={() =>
              run(async () => {
                setPublication(
                  await planDirectoryPublication(
                    {
                      providerId: providerId as `0x${string}`,
                      caller: address as `0x${string}`,
                      latInput,
                      lngInput,
                      contacts,
                      locationKind: 1,
                      locationActive: true,
                    },
                    reader!,
                    (utf8) => keccak256(toHex(utf8)),
                  ),
                );
              })
            }
          >
            Check what this would publish
          </Button>
          {publication ? (
            <DirectoryPublicationCard
              plan={publication}
              contactOnlyNotice={CONTACT_ONLY_NOTICE}
              anchoredNotice={CONTACTS_ARE_ANCHORED_NOT_SERVED}
            />
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
