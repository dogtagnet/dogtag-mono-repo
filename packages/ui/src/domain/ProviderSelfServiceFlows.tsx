/**
 * The provider self-service flows, end to end (registry-plan S-15).
 *
 * Shared because vet and groomer are both PROVIDERS and must not drift into two implementations of
 * one surface - but they do not get the same flows, and the difference is real rather than
 * cosmetic. A groomer VERIFIES and does not ISSUE: `BUSINESS_TYPE=groomer` mounts no issuance routes
 * at all, so a groomer has no `DogTagIssuer` clone. Flows 1, 2 and 3 are all keyed BY a clone -
 * deploy one, select one, claim a domain FOR one - so for a groomer they are not merely hidden, they
 * are inapplicable. Flow 4 is keyed by `providerId`, so it applies to every provider. Hence
 * {@link ProviderFlowCapabilities}: the caller states what it is, and the component does not guess
 * from a URL or a build flag.
 *
 * Every assessment comes from `../provider`, which is pure, reader-injected and separately tested.
 * This component adds exactly one thing on top of the renderer: the SEND. Each action is gated on
 * the matching `can*` boolean from the plan it belongs to, so a transaction is never offered for a
 * flow the preflight refused or could not resolve - `indeterminate` disables the button just as
 * `refused` does, because "we could not check" is not permission.
 */

import { useCallback, useMemo, useState, type ReactNode } from "react";
import { keccak256, toHex } from "viem";
import { useAccount, useWriteContract } from "wagmi";
import { Button } from "../components/Button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/Card";
import { Input } from "../components/Input";
import { Label } from "../components/Label";
import {
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  assessCandidateClone,
  assessDomainClaim,
  canWithdraw,
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  createLiveProviderReader,
  planCloneDeployment,
  planDirectoryPublication,
  REPOINT_SCOPE_NOTICE,
  validateDomain,
  type CloneAssessment,
  type DeployPlan,
  type DirectoryPublicationPlan,
  type DomainClaimAssessment,
  type ProviderContracts,
} from "../provider";
import {
  CORE_ABI,
  DIRECTORY_ABI,
  DOMAIN_ABI,
  FACTORY_ABI,
} from "../provider/liveReader";
import {
  CloneLifecycleCard,
  DeployPlanCard,
  DirectoryPublicationCard,
  DomainClaimCard,
} from "./ProviderSelfServicePanel";

/** Which flows this operator is a provider FOR. Stated by the caller, never inferred. */
export interface ProviderFlowCapabilities {
  /**
   * Deploy a clone, select which is current, and claim a domain for it. False for an operator that
   * issues nothing - those three flows are keyed by a clone it does not have.
   */
  issuance: boolean;
  /** Publish contacts and (optionally) a location. Keyed by provider id, so true for any provider. */
  listing: boolean;
}

export interface ProviderSelfServiceFlowsProps {
  contracts: ProviderContracts;
  /** The env var names that are unset, if any. Non-empty renders the unconfigured state. */
  missingConfig: readonly string[];
  rpcUrl?: string;
  defaultProviderId?: string;
  capabilities: ProviderFlowCapabilities;
}

const BLANK_CONTACTS = { phone: "", whatsapp: "", telegram: "", email: "", website: "" };
const CONTACT_CHANNELS = ["phone", "whatsapp", "telegram", "email", "website"] as const;

export function ProviderSelfServiceFlows({
  contracts,
  missingConfig,
  rpcUrl,
  defaultProviderId = "",
  capabilities,
}: ProviderSelfServiceFlowsProps): ReactNode {
  const { address, isConnected } = useAccount();
  const { writeContractAsync } = useWriteContract();

  const [providerId, setProviderId] = useState(defaultProviderId);
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
  const [sent, setSent] = useState<string[]>([]);

  const reader = useMemo(
    () => (missingConfig.length ? null : createLiveProviderReader({ contracts, rpcUrl })),
    [contracts, missingConfig.length, rpcUrl],
  );
  const recordTypeKey = useMemo(() => keccak256(toHex(recordType)), [recordType]);
  const providerIdOk = /^0x[0-9a-fA-F]{40}$/.test(providerId);
  const caller = address as `0x${string}` | undefined;

  const run = useCallback(async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      // A throw here is a fault in THIS surface or a rejected signature - never a verdict about the
      // provider. Every chain READ the engine makes is already caught and reported as its own
      // could-not-run, so nothing that reaches here can be mistaken for one.
      setError(e instanceof Error ? e.message.split("\n")[0]! : String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const note = (hash: string, what: string) => setSent((s) => [`${what}: ${hash}`, ...s]);

  if (missingConfig.length) {
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
          <p className="text-sm">Set: {missingConfig.join(", ")}</p>
          <p className="mt-2 text-xs text-muted-foreground">
            There is deliberately no built-in default. The generation-2 contracts are deployed but no
            client reads them yet, so a baked address here would repoint this deployment by accident.
          </p>
        </CardContent>
      </Card>
    );
  }

  const ready = !busy && !!reader && isConnected && !!caller;

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
          {capabilities.issuance ? (
            <div>
              <Label htmlFor="recordType">Record type</Label>
              <Input id="recordType" value={recordType} onChange={(e) => setRecordType(e.target.value)} />
            </div>
          ) : null}
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
          {sent.length ? (
            <ul className="text-xs text-muted-foreground" data-testid="sent-transactions">
              {sent.map((s) => (
                <li key={s} className="break-all font-mono">
                  {s}
                </li>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>

      {capabilities.issuance ? (
        <>
          {/* ---- Flow 1: deploy your own contract ------------------------------------------ */}
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
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  disabled={!ready || !providerIdOk}
                  onClick={() =>
                    run(async () => {
                      setDeploy(
                        await planCloneDeployment(
                          {
                            providerId: providerId as `0x${string}`,
                            recordType: recordTypeKey,
                            caller: caller!,
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
                <Button
                  // Gated on the plan, so a refused OR unresolved preflight offers no transaction.
                  disabled={!ready || !deploy?.canDeploy}
                  data-testid="deploy-send"
                  onClick={() =>
                    run(async () => {
                      const hash = await writeContractAsync({
                        address: contracts.factory,
                        abi: FACTORY_ABI,
                        functionName: "createIssuer",
                        args: [
                          providerId as `0x${string}`,
                          recordTypeKey,
                          BigInt(cloneNonce || "0"),
                        ],
                      });
                      note(hash, "Deployed contract");
                    })
                  }
                >
                  Deploy
                </Button>
              </div>
              {deploy ? (
                <DeployPlanCard plan={deploy} attachmentNotice={ATTACHMENT_IS_NOT_SELF_SERVICE} />
              ) : null}
            </CardContent>
          </Card>

          {/* ---- Flow 2: select which contract is current ----------------------------------- */}
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
                  Only a contract deployed by the DogTag issuer factory can be entered here. Anything
                  else is refused, whoever asks.
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  disabled={!ready || !providerIdOk || !candidate}
                  onClick={() =>
                    run(async () => {
                      setClone(
                        await assessCandidateClone(
                          {
                            candidate: candidate as `0x${string}`,
                            caller: caller!,
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
                <Button
                  disabled={!ready || !clone?.canRepoint}
                  data-testid="repoint-send"
                  onClick={() =>
                    run(async () => {
                      const hash = await writeContractAsync({
                        address: contracts.core,
                        abi: CORE_ABI,
                        functionName: "repointService",
                        args: [candidate as `0x${string}`],
                      });
                      note(hash, "Selected contract");
                    })
                  }
                >
                  Make this my current contract
                </Button>
              </div>
              {clone ? <CloneLifecycleCard assessment={clone} /> : null}
            </CardContent>
          </Card>

          {/* ---- Flow 3: claim a domain ----------------------------------------------------- */}
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
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  disabled={!ready || !candidate}
                  onClick={() =>
                    run(async () => {
                      setDomainState(await assessDomainClaim(candidate as `0x${string}`, caller!, reader!));
                    })
                  }
                >
                  Check the domain record
                </Button>
                <Button
                  disabled={!ready || !domainState?.canWrite || !validateDomain(domain).ok}
                  data-testid="domain-claim-send"
                  onClick={() =>
                    run(async () => {
                      const v = validateDomain(domain);
                      if (!v.ok) throw new Error(v.reason);
                      const hash = await writeContractAsync({
                        address: contracts.domainResolver,
                        abi: DOMAIN_ABI,
                        functionName: "claimDomain",
                        args: [candidate as `0x${string}`, v.domain],
                      });
                      note(hash, "Published domain");
                    })
                  }
                >
                  Publish this domain
                </Button>
                <Button
                  variant="outline"
                  disabled={!ready || !domainState?.canWrite}
                  data-testid="domain-none-send"
                  onClick={() =>
                    run(async () => {
                      const hash = await writeContractAsync({
                        address: contracts.domainResolver,
                        abi: DOMAIN_ABI,
                        functionName: "declareNoDomain",
                        args: [candidate as `0x${string}`],
                      });
                      note(hash, "Declared no domain");
                    })
                  }
                >
                  I deliberately have no domain
                </Button>
                {/* Offered ONLY when a claim exists to withdraw: the contract refuses otherwise,
                    because CLEARED asserts that one WAS withdrawn, and a button that always
                    reverted would be a fault the provider cannot interpret. */}
                {canWithdraw(domainState?.standing) ? (
                  <Button
                    variant="outline"
                    disabled={!ready || !domainState?.canWrite}
                    data-testid="domain-withdraw-send"
                    onClick={() =>
                      run(async () => {
                        const hash = await writeContractAsync({
                          address: contracts.domainResolver,
                          abi: DOMAIN_ABI,
                          functionName: "clearDomain",
                          args: [candidate as `0x${string}`],
                        });
                        note(hash, "Withdrew domain claim");
                      })
                    }
                  >
                    Withdraw my domain claim
                  </Button>
                ) : null}
              </div>
              {domainState ? <DomainClaimCard assessment={domainState} /> : null}
            </CardContent>
          </Card>
        </>
      ) : null}

      {/* ---- Flow 4: publish your listing ------------------------------------------------- */}
      {capabilities.listing ? (
        <Card>
          <CardHeader>
            <CardTitle>{capabilities.issuance ? "4. Your listing" : "Your listing"}</CardTitle>
            <CardDescription>
              A location is optional. Leave both fields blank if you do not publish one.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <div className="grid gap-3 sm:grid-cols-2">
              {CONTACT_CHANNELS.map((channel) => (
                // Five channels in a two-column grid, so the last would sit half-width alone. A URL
                // is also the longest value here, so spanning is the right shape and not only the
                // tidier one.
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
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                disabled={!ready || !providerIdOk}
                onClick={() =>
                  run(async () => {
                    setPublication(
                      await planDirectoryPublication(
                        {
                          providerId: providerId as `0x${string}`,
                          caller: caller!,
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
              <Button
                disabled={!ready || !publication?.canPublish}
                data-testid="publish-send"
                onClick={() =>
                  run(async () => {
                    // Sent in PLAN ORDER. The anchor lists the provider, so it goes first - and for
                    // a contact-only provider there is simply no second step, which is the whole of
                    // the no-placeholder rule: the absent pin is absent from the loop, not skipped
                    // inside it.
                    for (const step of publication!.steps) {
                      if (step.kind === "profileAnchor") {
                        const hash = await writeContractAsync({
                          address: contracts.directory,
                          abi: DIRECTORY_ABI,
                          functionName: "setProfileAnchor",
                          args: [
                            providerId as `0x${string}`,
                            step.digest,
                            step.schema,
                            step.codec,
                            step.hashAlgorithm,
                            step.contenthash,
                          ],
                        });
                        note(hash, "Published contact details");
                      } else {
                        const hash = await writeContractAsync({
                          address: contracts.directory,
                          abi: DIRECTORY_ABI,
                          functionName: "publishPin",
                          args: [
                            providerId as `0x${string}`,
                            step.lat,
                            step.lng,
                            step.locationKind,
                            step.active,
                          ],
                        });
                        note(hash, "Published location");
                      }
                    }
                  })
                }
              >
                Publish
              </Button>
            </div>
            {publication ? (
              <DirectoryPublicationCard
                plan={publication}
                contactOnlyNotice={CONTACT_ONLY_NOTICE}
                anchoredNotice={CONTACTS_ARE_ANCHORED_NOT_SERVED}
              />
            ) : null}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}
