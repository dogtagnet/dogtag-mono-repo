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
 *
 * TWO RULES GOVERN THE SEND, and both close the same defect: a verdict stated before the fact is
 * established.
 *
 *  1. **A send acts on what was CHECKED, never on what the form holds now.** Every handler used to
 *     read current state while its gate came from a plan computed earlier, so editing an input after
 *     pressing Check sent an unchecked value. On flow 2 that was not merely confusing: pasting a
 *     DIFFERENT clone the caller also owns would SUCCEED and silently move the wrong record type's
 *     pointer. Two mechanisms now stand between that, deliberately, so neither is load-bearing
 *     alone - a plan is INVALIDATED when any input it depends on changes (the button disables and
 *     the panel says why), and every send addresses the plan's OWN captured values.
 *
 *     A PLAN IS ALSO RETIRED BY ITS OWN TRANSACTION, which is the same rule applied to the other
 *     set of inputs. A plan is keyed on the FORM, so an untouched form leaves the key matching -
 *     but the answers came from the CHAIN, and submitting moves that. Without it, Check -> Publish
 *     -> Publish re-sent `setProfileAnchor` with a digest whose `anchorUnchanged` guard had been
 *     computed against pre-transaction state, bumping the anchor revision for nothing and
 *     invalidating `coversCurrentAddressText` on any registrar address confirmation the provider
 *     holds - the exact harm `directoryPlan.ts` reads the current anchor to avoid.
 *  2. **A submitted transaction is not a completed one.** `writeContractAsync` resolves on a hash
 *     and does not throw on a revert, so the outcome is read from the receipt and reported as
 *     itself. See `../provider/sendOutcome` for the four states and why the fourth cannot collapse
 *     into either neighbour.
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
import { PROVIDER_CONTACT_CHANNELS } from "../directory/channels";
import { blankContactFields } from "../directory/registration";
import {
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  assessCandidateClone,
  assessDomainClaim,
  canWithdraw,
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  createLiveProviderReader,
  mayContinueAfter,
  outcomeFromReceiptStatus,
  planCloneDeployment,
  planDirectoryPublication,
  REPOINT_SCOPE_NOTICE,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  validateDomain,
  WITHDRAW_LOCATION_NOTICE,
  type CloneAssessment,
  type DeployPlan,
  type DirectoryPublicationPlan,
  type DomainClaimAssessment,
  type ProviderContracts,
  type SendRecord,
  type SendState,
} from "../provider";
import {
  CORE_ABI,
  DIRECTORY_ABI,
  DOMAIN_ABI,
  FACTORY_ABI,
} from "../provider/liveReader";
import { roaxPublicClient } from "../wallet/contracts";
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

/**
 * How long to follow a transaction before reporting its outcome as unestablished.
 *
 * A bound is required, because the alternative to `unknown` here is not "wait a little longer" - it
 * is a spinner that never resolves and a provider who cannot tell whether anything happened.
 */
const RECEIPT_TIMEOUT_MS = 90_000;

/**
 * A plan plus the inputs it was computed FROM, and whether it has already been acted on.
 *
 * The key is compared against the live inputs on every render, so a plan whose inputs moved stops
 * gating anything - see rule 1 in the module header. `spent` closes the other half of the same rule:
 * the inputs a plan is keyed on are the FORM's, and a transaction moves the CHAIN's, so a plan whose
 * key still matches can have been falsified by its own send.
 */
interface Checked<T> {
  key: string;
  plan: T;
  /** Set once a transaction has been submitted against this plan. Never unset. */
  spent: boolean;
}

/** A plan that may still authorize a transaction: keyed on the current inputs, and not yet acted on. */
function fresh<T>(held: Checked<T> | null, key: string): T | null {
  return held && !held.spent && held.key === key ? held.plan : null;
}

/**
 * Why a held plan is no longer gating its button. `null` when it still is, or when there is none.
 *
 * Two reasons, told apart rather than merged, because they have different remedies and a provider
 * who cannot tell them apart cannot act: `edited` means re-check what you typed, `spent` means the
 * chain has moved under the answer you were shown.
 */
function retiredBecause<T>(held: Checked<T> | null, key: string): "edited" | "spent" | null {
  if (!held) return null;
  if (held.spent) return "spent";
  return held.key === key ? null : "edited";
}

/**
 * Retire a plan because a transaction has been submitted against it.
 *
 * Applied at SUBMISSION rather than at any one terminal outcome, which covers all of them: the
 * moment our own transaction is in flight, the chain state the plan was computed against is no
 * longer necessarily current. A revert is no exception - a write can revert precisely BECAUSE
 * something moved - and an unfetchable receipt least of all, since an outcome nobody could establish
 * must not authorize a resend.
 */
function spend<T>(held: Checked<T> | null): Checked<T> | null {
  return held && !held.spent ? { ...held, spent: true } : held;
}

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
  // Folded from the SHARED channel list rather than a local literal, so a sixth channel cannot be
  // added to the list and silently missed here.
  const [contacts, setContacts] = useState(() => blankContactFields());

  const [deployHeld, setDeployHeld] = useState<Checked<DeployPlan> | null>(null);
  const [cloneHeld, setCloneHeld] = useState<Checked<CloneAssessment> | null>(null);
  const [domainHeld, setDomainHeld] = useState<Checked<DomainClaimAssessment> | null>(null);
  const [publicationHeld, setPublicationHeld] = useState<Checked<DirectoryPublicationPlan> | null>(
    null,
  );

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState<SendRecord[]>([]);

  const reader = useMemo(
    () => (missingConfig.length ? null : createLiveProviderReader({ contracts, rpcUrl })),
    [contracts, missingConfig.length, rpcUrl],
  );
  const recordTypeKey = useMemo(() => keccak256(toHex(recordType)), [recordType]);
  const providerIdOk = /^0x[0-9a-fA-F]{40}$/.test(providerId);
  const caller = address as `0x${string}` | undefined;

  // The input fingerprints. Anything a plan's answer depends on belongs in its key; a value left out
  // is a value that can be edited after Check without disabling the button, which is the whole bug.
  const identity = `${caller ?? ""}|${providerId}`;
  const deployKey = `${identity}|${recordTypeKey}|${cloneNonce}`;
  const cloneKey = `${identity}|${candidate}`;
  const domainKey = `${identity}|${candidate}`;
  const publicationKey = `${identity}|${latInput}|${lngInput}|${JSON.stringify(contacts)}`;

  const deploy = fresh(deployHeld, deployKey);
  const clone = fresh(cloneHeld, cloneKey);
  const domainState = fresh(domainHeld, domainKey);
  const publication = fresh(publicationHeld, publicationKey);

  const run = useCallback(async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      // A throw here is a fault in THIS surface or a rejected signature - never a verdict about the
      // provider. Every chain READ the engine makes is already caught and reported as its own
      // could-not-run, and every transaction OUTCOME is reported as its own record, so nothing that
      // reaches here can be mistaken for either.
      setError(e instanceof Error ? e.message.split("\n")[0]! : String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  /**
   * Send one transaction and report what actually happened to it.
   *
   * The hash is recorded as `submitted` BEFORE the receipt is awaited, so a provider who closes the
   * tab mid-wait still has the hash; it is then replaced in place by the settled record. Returns the
   * final state so a multi-step sequence can stop rather than sending the next step after a revert.
   */
  const sendAndFollow = useCallback(
    async (
      what: string,
      retire: () => void,
      send: () => Promise<`0x${string}`>,
    ): Promise<SendState> => {
      const hash = await send();
      // The plan that authorized this is retired HERE, before the outcome is known, so every
      // terminal state inherits it and no handler can forget one. It is a required parameter rather
      // than an optional one so a new flow cannot silently omit it. A rejected signature never
      // reaches this line, which is correct: nothing was submitted, so nothing was falsified.
      retire();
      setSent((s) => [sendRecord(hash, what, "submitted"), ...s]);
      const settle = (record: SendRecord) =>
        setSent((s) => s.map((r) => (r.hash === record.hash ? record : r)));
      try {
        const receipt = await roaxPublicClient(rpcUrl).waitForTransactionReceipt({
          hash,
          timeout: RECEIPT_TIMEOUT_MS,
        });
        const state = outcomeFromReceiptStatus(receipt.status);
        settle(sendRecord(hash, what, state));
        return state;
      } catch (e) {
        // NOT a failure and NOT a success. A receipt we could not fetch says nothing about whether
        // the transaction mined - it is routed here rather than into the page-level error, which
        // would collapse it into the same bucket as a rejected signature.
        const reason = e instanceof Error ? e.message.split("\n")[0]! : String(e);
        settle(sendRecord(hash, what, "unknown", reason));
        return "unknown";
      }
    },
    [rpcUrl],
  );

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
          <SendLog records={sent} />
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
                      const plan = await planCloneDeployment(
                        {
                          providerId: providerId as `0x${string}`,
                          recordType: recordTypeKey,
                          caller: caller!,
                          cloneNonce: BigInt(cloneNonce || "0"),
                        },
                        reader!,
                      );
                      setDeployHeld({ key: deployKey, plan, spent: false });
                    })
                  }
                >
                  Check what this would deploy
                </Button>
                <Button
                  // Gated on the FRESH plan, so a refused, unresolved OR stale preflight offers no
                  // transaction - and the arguments come from that plan, not from the fields.
                  disabled={!ready || !deploy?.canDeploy}
                  data-testid="deploy-send"
                  onClick={() =>
                    run(async () => {
                      const { request } = deploy!;
                      await sendAndFollow("Deploy contract", () => setDeployHeld(spend), () =>
                        writeContractAsync({
                          address: contracts.factory,
                          abi: FACTORY_ABI,
                          functionName: "createIssuer",
                          args: [request.providerId, request.recordType, request.cloneNonce],
                        }),
                      );
                    })
                  }
                >
                  Deploy
                </Button>
              </div>
              <PlanNotice reason={retiredBecause(deployHeld, deployKey)} testId="deploy-stale" />
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
                      const plan = await assessCandidateClone(
                        {
                          candidate: candidate as `0x${string}`,
                          caller: caller!,
                          providerId: providerId as `0x${string}`,
                        },
                        reader!,
                      );
                      setCloneHeld({ key: cloneKey, plan, spent: false });
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
                      // The address the assessment JUDGED. Reading the field here is how a different
                      // but also-owned clone gets selected silently.
                      await sendAndFollow("Select contract", () => setCloneHeld(spend), () =>
                        writeContractAsync({
                          address: contracts.core,
                          abi: CORE_ABI,
                          functionName: "repointService",
                          args: [clone!.candidate],
                        }),
                      );
                    })
                  }
                >
                  Make this my current contract
                </Button>
              </div>
              <PlanNotice reason={retiredBecause(cloneHeld, cloneKey)} testId="repoint-stale" />
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
                      const plan = await assessDomainClaim(
                        candidate as `0x${string}`,
                        caller!,
                        reader!,
                      );
                      setDomainHeld({ key: domainKey, plan, spent: false });
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
                      // The domain STRING is legitimately current - the assessment checks the
                      // resolver and this key's authority, neither of which depends on it. The
                      // SERVICE is the assessment's, for the same reason flow 2's is.
                      await sendAndFollow("Publish domain", () => setDomainHeld(spend), () =>
                        writeContractAsync({
                          address: contracts.domainResolver,
                          abi: DOMAIN_ABI,
                          functionName: "claimDomain",
                          args: [domainState!.service, v.domain],
                        }),
                      );
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
                      await sendAndFollow("Declare no domain", () => setDomainHeld(spend), () =>
                        writeContractAsync({
                          address: contracts.domainResolver,
                          abi: DOMAIN_ABI,
                          functionName: "declareNoDomain",
                          args: [domainState!.service],
                        }),
                      );
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
                        await sendAndFollow("Withdraw domain claim", () => setDomainHeld(spend), () =>
                          writeContractAsync({
                            address: contracts.domainResolver,
                            abi: DOMAIN_ABI,
                            functionName: "clearDomain",
                            args: [domainState!.service],
                          }),
                        );
                      })
                    }
                  >
                    Withdraw my domain claim
                  </Button>
                ) : null}
              </div>
              <PlanNotice reason={retiredBecause(domainHeld, domainKey)} testId="domain-stale" />
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
              {PROVIDER_CONTACT_CHANNELS.map((channel) => (
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
                    const plan = await planDirectoryPublication(
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
                    );
                    setPublicationHeld({ key: publicationKey, plan, spent: false });
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
                    // Sent in PLAN ORDER, from the plan's own captured digest, coordinates and
                    // provider id. The anchor lists the provider, so it goes first - and for a
                    // contact-only provider there is simply no second step, which is the whole of
                    // the no-placeholder rule: the absent pin is absent from the loop, not skipped
                    // inside it.
                    const plan = publication!;
                    for (const step of plan.steps) {
                      const state =
                        step.kind === "profileAnchor"
                          ? await sendAndFollow("Publish contact details", () => setPublicationHeld(spend), () =>
                              writeContractAsync({
                                address: contracts.directory,
                                abi: DIRECTORY_ABI,
                                functionName: "setProfileAnchor",
                                args: [
                                  plan.providerId,
                                  step.digest,
                                  step.schema,
                                  step.codec,
                                  step.hashAlgorithm,
                                  step.contenthash,
                                ],
                              }),
                            )
                          : step.op === "publish"
                            ? await sendAndFollow("Publish location", () => setPublicationHeld(spend), () =>
                                writeContractAsync({
                                  address: contracts.directory,
                                  abi: DIRECTORY_ABI,
                                  functionName: "publishPin",
                                  args: [
                                    plan.providerId,
                                    step.lat,
                                    step.lng,
                                    step.locationKind,
                                    step.active,
                                  ],
                                }),
                              )
                            : // The step that stops one provider appearing in two places: a second
                              // Publish REWRITES the pin it is correcting rather than appending.
                              await sendAndFollow("Replace location", () => setPublicationHeld(spend), () =>
                                writeContractAsync({
                                  address: contracts.directory,
                                  abi: DIRECTORY_ABI,
                                  functionName: "updatePin",
                                  args: [
                                    plan.providerId,
                                    step.locationNo,
                                    step.lat,
                                    step.lng,
                                    step.locationKind,
                                    step.active,
                                  ],
                                }),
                              );
                      // A reverted or unfollowable step stops the sequence. `writeContractAsync`
                      // does not throw on a revert, so without this the pin would be sent after the
                      // anchor had already failed.
                      if (!mayContinueAfter(state)) return;
                    }
                  })
                }
              >
                Publish
              </Button>
              {publication?.canWithdrawPin ? (
                <Button
                  variant="outline"
                  data-testid="withdraw-pin-send"
                  disabled={!ready}
                  onClick={() =>
                    run(async () => {
                      const plan = publication!;
                      await sendAndFollow("Withdraw location", () => setPublicationHeld(spend), () =>
                        writeContractAsync({
                          address: contracts.directory,
                          abi: DIRECTORY_ABI,
                          functionName: "removePin",
                          args: [plan.providerId, plan.listing!.onlyPin!.locationNo],
                        }),
                      );
                    })
                  }
                >
                  Take my published location down
                </Button>
              ) : null}
            </div>
            {publication?.canWithdrawPin ? (
              <p className="text-xs text-muted-foreground" data-testid="withdraw-pin-notice">
                {WITHDRAW_LOCATION_NOTICE}
              </p>
            ) : null}
            <PlanNotice reason={retiredBecause(publicationHeld, publicationKey)} testId="publish-stale" />
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

/**
 * Says why the button is off, in the provider's own terms.
 *
 * Rendered rather than silently dropping the panel: a panel that vanished on the first keystroke -
 * or the moment a transaction was signed - would look like a fault. The button is disabled either
 * way; this is the half that explains WHICH of the two reasons it is, because "re-read what you
 * typed" and "the chain has moved" send a provider to different places.
 */
function PlanNotice({
  reason,
  testId,
}: {
  reason: "edited" | "spent" | null;
  testId: string;
}): ReactNode {
  if (!reason) return null;
  return (
    <p
      className="text-xs text-amber-700 dark:text-amber-400"
      data-testid={reason === "spent" ? `${testId}-spent` : testId}
    >
      {reason === "edited"
        ? "You have changed something since this was checked, so what is shown below is about the earlier values. Check again before sending."
        : "This has already been acted on, and the answers below were read before that transaction. Check again before sending another."}
    </p>
  );
}

const SEND_STATE_STYLE: Readonly<Record<SendState, string>> = {
  // Amber, not neutral: an outcome nobody has established yet is a gap in what is known.
  submitted: "text-amber-700 dark:text-amber-400",
  succeeded: "text-emerald-700 dark:text-emerald-400",
  reverted: "text-red-700 dark:text-red-400",
  unknown: "text-amber-700 dark:text-amber-400",
};

/**
 * What was sent, and what became of it.
 *
 * Each row states its own outcome rather than implying one from the fact that a hash exists, and
 * links the transaction only when the hash can actually address one.
 */
function SendLog({ records }: { records: readonly SendRecord[] }): ReactNode {
  if (records.length === 0) return null;
  return (
    <ul className="flex flex-col gap-2 text-xs" data-testid="sent-transactions">
      {records.map((r) => {
        const href = sendExplorerHref(r);
        return (
          <li key={r.hash} data-testid={`sent-${r.state}`}>
            <span className="text-muted-foreground">{r.what}</span>{" "}
            <span className={SEND_STATE_STYLE[r.state]}>{sendStateLabel(r.state)}</span>
            <br />
            {href ? (
              <a
                className="break-all font-mono underline"
                href={href}
                target="_blank"
                rel="noreferrer"
              >
                {r.hash}
              </a>
            ) : (
              <span className="break-all font-mono text-muted-foreground">{r.hash}</span>
            )}
            {r.unknownReason ? (
              // Printed, not hovered - the same rule the check rows follow. This sentence is what
              // distinguishes "we could not follow it" from "it failed".
              <span className="block text-amber-700 dark:text-amber-400">
                Why the outcome is not known: {r.unknownReason}
              </span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
