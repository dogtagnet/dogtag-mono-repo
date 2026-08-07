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

import { Sparkles } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
import { cn } from "../lib/cn";
import { PROVIDER_CONTACT_CHANNELS } from "../directory/channels";
import {
  checkLogoPublication,
  keccakBytes,
  mirrorContentReader,
  publicationDigest,
  putMirrorContent,
  resolveProviderProfile,
  SERVABLE_IMAGE_MEDIA_TYPES,
  type LogoPublication,
  type ProfileResolution,
} from "../mirror";
import { blankContactFields } from "../directory/registration";
import { DEMO_PROVIDER_LISTING } from "../schema/demoData";
import {
  ATTACHMENT_IS_A_DOGTAG_STEP,
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  assessCandidateClone,
  checkBlock,
  describePlanRetirement,
  renderReason,
  sequenceReasons,
  planGateState,
  sendBlock,
  type ActionBlock,
  type RenderedReason,
  assessDomainClaim,
  assessResolverSelection,
  canStopUsing,
  canWithdraw,
  kindForScope,
  ZERO_ADDR,
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  createdAddressMeaning,
  createLiveProviderReader,
  DEPLOYED_CONTRACT_NEXT_STEP,
  DIRECTORY_NEEDS_TURNING_ON,
  DOMAIN_REGISTER_NEEDS_TURNING_ON,
  mayContinueAfter,
  mirrorPublicationRefusal,
  nextContractNumber,
  outcomeFromReceiptStatus,
  parseContractNumber,
  planCloneDeployment,
  planDirectoryPublication,
  readDeploymentHistory,
  recallProviderId,
  rememberProviderId,
  REPOINT_SCOPE_NOTICE,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  validateDomain,
  WITHDRAW_LOCATION_NOTICE,
  type CloneAssessment,
  type DeploymentHistory,
  type DeployPlan,
  type DirectoryPublicationPlan,
  type DomainClaimAssessment,
  type ProviderContracts,
  type ResolverListing,
  type ResolverSelectionPlan,
  type ResolverSelectionScope,
  type SendRecord,
  type SendState,
} from "../provider";
import {
  CORE_ABI,
  DIRECTORY_ABI,
  DOMAIN_ABI,
  FACTORY_ABI,
} from "../provider/liveReader";
import { ROAX_CHAIN_ID } from "../wallet/chain";
import { classifySurfaceFault, type SurfaceFault } from "../wallet/walletError";
import { roaxPublicClient } from "../wallet/contracts";
import {
  CloneLifecycleCard,
  DeployedContractsCard,
  DeployPlanCard,
  DirectoryPublicationCard,
  NextContractNumberNotice,
  PublishedListingCard,
  DomainClaimCard,
  ResolverSelectionCard,
  WalletFaultNotice,
  WhyThisExists,
  type PlanRetirement,
} from "./ProviderSelfServicePanel";

/**
 * What the register dropdown has to offer, as three states rather than an array.
 *
 * `pending` is not `read` with an empty list, and neither is `unavailable`. A dropdown that rendered
 * "no registers approved" while the read was still in flight would be a definite claim about DogTag
 * made before anybody asked, and one that rendered it after a FAILED read would be the same claim
 * made on the strength of a question that never got an answer. Both are the defect this page exists
 * not to have, in the one place a reader would take at face value: an empty list of options.
 */
type ResolverOptions =
  | { state: "pending" }
  | { state: "read"; listings: readonly ResolverListing[] }
  | { state: "unavailable"; reason: string };

/** Native `<select>` styling matching `Input`. Native because it must be driveable and keyboard-first. */
const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-surface px-3 py-2 text-sm text-onSurface "
  + "ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 "
  + "focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50";

/**
 * The provider's own half of the typed-resolver pair, rendered once and used by both flows.
 *
 * ONE COMPONENT, TWO SCOPES, deliberately. The two selections differ only in what they are keyed by
 * and which setter they send, so two near-identical inline blocks would be two places for a rule to
 * drift - and this whole surface exists because one half of a two-party arrangement had no
 * implementation at all.
 *
 * `<select>` is NATIVE rather than the radix component: it has to be driveable by a test harness and
 * by a keyboard, and the repo already records that `selectOption` cannot drive a radix Select.
 */
function RegisterPicker({
  scope,
  options,
  choice,
  onChoice,
  allowStop,
  checkLabel,
  onCheck,
  checkDisabled,
  checkReason,
  sendLabel,
  onSend,
  sendDisabled,
  sendReason,
  plan,
  retired,
  planCheckBlock,
}: {
  scope: ResolverSelectionScope;
  options: ResolverOptions;
  choice: string;
  onChoice: (value: string) => void;
  /** Offer "stop using one" only when something is selected - the chain refuses the no-op otherwise. */
  allowStop: boolean;
  checkLabel: string;
  onCheck: () => void;
  checkDisabled: boolean;
  checkReason: RenderedReason | null;
  sendLabel: string;
  onSend: () => void;
  sendDisabled: boolean;
  sendReason: RenderedReason | null;
  plan: ResolverSelectionPlan | null;
  retired: PlanRetirement | null;
  planCheckBlock: ActionBlock | null;
}): ReactNode {
  const noun = scope === "directory" ? "directory register" : "domain register";
  const approved = options.state === "read" ? options.listings.filter((l) => l.approved) : [];
  return (
    <div className="flex flex-col gap-3 rounded-lg border border-border p-3">
      <div>
        <Label htmlFor={`${scope}-register`}>Which {noun} do you use?</Label>
        {/* THREE STATES, NEVER COLLAPSED INTO AN EMPTY DROPDOWN. A `<select>` with no options is the
            one thing a reader takes at face value, so "still reading", "the read failed" and "DogTag
            has approved none" each say themselves rather than sharing an appearance. */}
        {options.state === "pending" ? (
          <p
            className="mt-1 text-sm text-muted-foreground"
            data-testid={`${scope}-register-options-pending`}
          >
            Reading which registers DogTag approves…
          </p>
        ) : options.state === "unavailable" ? (
          <p
            className="mt-1 text-sm text-amber-700 dark:text-amber-400"
            data-testid={`${scope}-register-options-unavailable`}
          >
            The registers you may choose from could not be read, so this page cannot offer a choice.
            That is its connection to the chain, not anything you have set up. Reason: {options.reason}
          </p>
        ) : approved.length === 0 && !allowStop ? (
          <p
            className="mt-1 text-sm text-muted-foreground"
            data-testid={`${scope}-register-options-none`}
          >
            DogTag has approved no {noun} yet, so there is nothing to choose. That half is theirs -
            ask them. Nothing you have set up is wrong.
          </p>
        ) : (
          <select
            id={`${scope}-register`}
            className={cn(SELECT_CLASS, "mt-1")}
            data-testid={`${scope}-register-select`}
            value={choice}
            onChange={(e) => onChoice(e.target.value)}
          >
            {choice === "" ? <option value="">Pick a register…</option> : null}
            {approved.map((l) => (
              <option key={l.resolver} value={l.resolver}>
                {l.resolver}
              </option>
            ))}
            {/* Offered only when something is selected, because the chain refuses a write that would
                change nothing - and because without it the selection would be permanent from this
                page, which is the shape of dead end this whole surface exists to remove. */}
            {allowStop ? (
              <option value={ZERO_ADDR}>Stop using a {noun} — publish through none</option>
            ) : null}
          </select>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          disabled={checkDisabled}
          data-testid={`${scope}-register-check`}
          onClick={onCheck}
        >
          {checkLabel}
        </Button>
        <Button disabled={sendDisabled} data-testid={`${scope}-register-send`} onClick={onSend}>
          {sendLabel}
        </Button>
      </div>
      <ActionReason reason={checkReason} testId={`${scope}-register-check-reason`} />
      <ActionReason reason={sendReason} testId={`${scope}-register-send-reason`} />
      <PlanNotice
        reason={retired}
        testId={`${scope}-register-stale`}
        checkBlocked={planCheckBlock}
      />
      {plan ? <ResolverSelectionCard plan={plan} retired={retired} /> : null}
    </div>
  );
}

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
  /**
   * The content mirror the profile blob and logo are published to (the indexer's base URL).
   *
   * A deployment setting, never provider-supplied. That is not a trust claim about the host - a
   * content address makes the host irrelevant, which is the point of the whole slice - but a
   * provider-named upload target is a request the provider controls, and there is no reason to
   * offer one.
   */
  mirrorBase?: string;
  /** Bearer for the mirror's write route. Reads are public; writes are not a free content host. */
  mirrorToken?: string;
  /**
   * Show the demo-fill affordance. Defaults OFF: this component is shared by two production
   * portals, so a prefill must be opted into by a host that knows it is running in demo mode.
   */
  demoMode?: boolean;
}

/**
 * How long to follow a transaction before reporting its outcome as unestablished.
 *
 * A bound is required, because the alternative to `unknown` here is not "wait a little longer" - it
 * is a spinner that never resolves and a provider who cannot tell whether anything happened.
 */
const RECEIPT_TIMEOUT_MS = 90_000;

/**
 * How long to wait for the WALLET to answer before saying we cannot tell.
 *
 * Generous, because a person may be reading the transaction, fetching a hardware key, or unlocking
 * an extension - and a premature could-not-tell beside a request they are about to approve is its
 * own kind of wrong. But bounded, because the alternative is what happened: a promise that never
 * resolves and a page that shows nothing at all while the transaction mines.
 */
const WALLET_TIMEOUT_MS = 180_000;

/**
 * Why the control above cannot be used, or nothing when it can.
 *
 * Rendered under the button row rather than as a tooltip, for the reason this repo applies to every
 * finding: a reason reachable only by hovering is not reported. Muted rather than amber - most of
 * these are ordinary first-run states ("run the check first"), not warnings, and painting them all
 * as alarms would train the reader past the ones that are.
 */
function ActionReason({
  reason,
  testId,
}: {
  reason: RenderedReason | null;
  testId: string;
}): ReactNode {
  if (!reason) return null;
  return (
    <p
      className="text-xs text-muted-foreground"
      data-testid={testId}
      data-block={reason.block.kind}
      data-style={reason.style}
    >
      {renderReason(reason)}
    </p>
  );
}

/**
 * That a flow waits on a step DogTag takes, said before the provider tries it.
 *
 * Deliberately not styled as a failure. It states a DEPENDENCY, which is permanent and true whether
 * or not the step has happened yet - so it must not read as "this is broken", and it must not go
 * stale the day the step is taken. What it does buy is the thing a wall never tells you: that
 * hitting it is not something you did.
 */
function DependencyNotice({
  children,
  testId,
}: {
  children: ReactNode;
  testId: string;
}): ReactNode {
  return (
    <p
      className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground"
      data-testid={testId}
    >
      {children}
    </p>
  );
}

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

/**
 * A plan that may still AUTHORIZE a transaction: keyed on the current inputs, and not yet acted on.
 *
 * Deliberately not the same question as whether the answers are VISIBLE - see {@link shown}. One
 * value answering both is why a retired plan used to vanish underneath a notice that referred to it.
 */
function fresh<T>(held: Checked<T> | null, key: string): T | null {
  return held && !held.spent && held.key === key ? held.plan : null;
}

/**
 * The plan to DISPLAY, retired or not.
 *
 * A retired plan is still worth reading: right after a transaction goes out, "what did I check
 * before I sent this" is the question a provider actually asks, and dropping the card answers it
 * with nothing. What makes showing it safe is the label - {@link retiredBecause} feeds a superseded
 * marker onto the verdict itself, so the answers cannot be read as current.
 */
function shown<T>(held: Checked<T> | null): T | null {
  return held ? held.plan : null;
}

/**
 * Why a held plan no longer authorizes its button. `null` when it still does, or when there is none.
 *
 * Two reasons, told apart rather than merged, because they have different remedies and a provider
 * who cannot tell them apart cannot act: `edited` means re-check what you typed, `spent` means the
 * chain has moved under the answer you were shown.
 */
function retiredBecause<T>(held: Checked<T> | null, key: string): PlanRetirement | null {
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
  mirrorBase,
  mirrorToken,
  demoMode = false,
}: ProviderSelfServiceFlowsProps): ReactNode {
  // `chainId` is read for the SEND gate only, and `undefined` means the connector did not report
  // one - which is not the wrong chain. See `ChainCheck`.
  const { address, isConnected, chainId } = useAccount();
  const { writeContractAsync } = useWriteContract();

  const [providerId, setProviderId] = useState(defaultProviderId);
  // Whether the operator has typed into the provider-id field THIS session. Their typing wins over
  // anything remembered, same rule as `nonceEdited`: a restore is a convenience, and a page that
  // overwrote a value being typed would be deciding whose record this is on their behalf.
  const [providerIdEdited, setProviderIdEdited] = useState(false);
  const [recordType, setRecordType] = useState("VACCINATION");
  const [cloneNonce, setCloneNonce] = useState("0");
  const [candidate, setCandidate] = useState("");
  const [domain, setDomain] = useState("");
  const [latInput, setLatInput] = useState("");
  const [lngInput, setLngInput] = useState("");
  // Folded from the SHARED channel list rather than a local literal, so a sixth channel cannot be
  // added to the list and silently missed here.
  const [contacts, setContacts] = useState(() => blankContactFields());
  // `null` is the ordinary "no logo" case and is never a failure - the rendering side reports it as
  // `notPublished`, quietly. `logoError` is what a REFUSED file leaves behind: a picker that
  // silently ignored a selection would leave the provider believing they had published one.
  const [logo, setLogo] = useState<LogoPublication | null>(null);
  const [logoName, setLogoName] = useState<string>("");
  const [logoError, setLogoError] = useState<string | null>(null);

  // The register each selection would switch TO. `""` is "nothing picked yet" and is not a valid
  // choice; `ZERO_ADDR` IS one, and means "stop using a register" - which the chain accepts, since
  // both setters check approval only for a non-zero value.
  const [domainRegisterChoice, setDomainRegisterChoice] = useState("");
  const [directoryRegisterChoice, setDirectoryRegisterChoice] = useState("");

  const [deployHeld, setDeployHeld] = useState<Checked<DeployPlan> | null>(null);
  const [cloneHeld, setCloneHeld] = useState<Checked<CloneAssessment> | null>(null);
  const [domainHeld, setDomainHeld] = useState<Checked<DomainClaimAssessment> | null>(null);
  const [domainRegisterHeld, setDomainRegisterHeld] =
    useState<Checked<ResolverSelectionPlan> | null>(null);
  const [directoryRegisterHeld, setDirectoryRegisterHeld] =
    useState<Checked<ResolverSelectionPlan> | null>(null);
  const [publicationHeld, setPublicationHeld] = useState<Checked<DirectoryPublicationPlan> | null>(
    null,
  );

  // What this wallet has already deployed, read back from the chain. `undefined` is NOT a fourth
  // rendered outcome and NOT an empty list - it is "not read yet", which is the state every page
  // load passes through while wagmi reconnects, and which must render nothing at all.
  const [deployHistory, setDeployHistory] = useState<DeploymentHistory | undefined>(undefined);
  const [historyLoading, setHistoryLoading] = useState(false);
  // Bumped when a transaction SETTLES, never when one is submitted. Keying the re-read on the send
  // log's length would fire at `awaitingWallet` - before the deploy is mined, so the read comes back
  // without it - and then never again, leaving the list stale at precisely the moment the operator
  // has just deployed and is looking for the result.
  const [settled, setSettled] = useState(0);
  // Whether the operator has taken the contract-number field over. Their number wins from then on:
  // the pre-fill is a convenience, and a page that kept overwriting a typed value would be deciding
  // which contract to deploy on their behalf.
  const [nonceEdited, setNonceEdited] = useState(false);

  const [busy, setBusy] = useState(false);
  // A CLASSIFIED fault, not a raw string. The kind is what lets the renderer say whose fault it is
  // and what nothing was established about - which a message alone cannot carry.
  const [fault, setFault] = useState<SurfaceFault | null>(null);
  const [sent, setSent] = useState<SendRecord[]>([]);
  // Row identity, minted before the wallet is asked. A hash cannot serve, because at that moment
  // there is not one - which is what kept the old code from recording anything at all until after.
  const sendSeq = useRef(0);

  const reader = useMemo(
    () => (missingConfig.length ? null : createLiveProviderReader({ contracts, rpcUrl })),
    [contracts, missingConfig.length, rpcUrl],
  );
  const recordTypeKey = useMemo(() => keccak256(toHex(recordType)), [recordType]);
  const providerIdOk = /^0x[0-9a-fA-F]{40}$/.test(providerId);

  // WHAT MAY BE CHOSEN, READ FROM THE CHAIN'S OWN ALLOWLIST rather than from a bundled address, so a
  // register DogTag approves tomorrow becomes choosable with no rebuild and no redeploy. Read once
  // per reader, because it is a fleet-wide fact and depends on neither the provider id nor the
  // contract in the form.
  //
  // The dropdown is only the INPUT WIDGET. The plan re-reads the list itself and validates the chosen
  // address against it, so a stale option can at worst be refused by the check with a precise
  // sentence - never sent. That is the safe direction for the two sources to disagree in.
  const [registerOptions, setRegisterOptions] = useState<
    Readonly<Record<ResolverSelectionScope, ResolverOptions>>
  >({ directory: { state: "pending" }, domain: { state: "pending" } });
  useEffect(() => {
    if (!reader) {
      setRegisterOptions({ directory: { state: "pending" }, domain: { state: "pending" } });
      return;
    }
    let current = true;
    const load = async (scope: ResolverSelectionScope) => {
      try {
        const listings = await reader.approvedResolvers(kindForScope(scope));
        if (current) {
          setRegisterOptions((prev) => ({ ...prev, [scope]: { state: "read", listings } }));
        }
      } catch (error) {
        if (current) {
          setRegisterOptions((prev) => ({
            ...prev,
            [scope]: {
              state: "unavailable",
              reason:
                error instanceof Error && error.message ? error.message : "the read gave no reason",
            },
          }));
        }
      }
    };
    void load("directory");
    void load("domain");
    return () => {
      current = false;
    };
    // `sent.length` so a registrar approving a register while this page is open is picked up by the
    // next action rather than needing a reload.
  }, [reader, sent.length]);

  // Pre-select the first approved register, ONCE, and never over a value the provider has set. Same
  // rule as the contract-number field: a pre-fill is a convenience, and a page that kept overwriting
  // a chosen value would be deciding on the provider's behalf. `""` is the only state this replaces,
  // and every real choice - including `ZERO_ADDR` for "stop using one" - is non-empty.
  const directoryFirstApproved =
    registerOptions.directory.state === "read"
      ? registerOptions.directory.listings.find((l) => l.approved)?.resolver
      : undefined;
  const domainFirstApproved =
    registerOptions.domain.state === "read"
      ? registerOptions.domain.listings.find((l) => l.approved)?.resolver
      : undefined;
  useEffect(() => {
    if (directoryFirstApproved) setDirectoryRegisterChoice((c) => c || directoryFirstApproved);
  }, [directoryFirstApproved]);
  useEffect(() => {
    if (domainFirstApproved) setDomainRegisterChoice((c) => c || domainFirstApproved);
  }, [domainFirstApproved]);

  // What a READER sees of this provider. `undefined` is the pending state and is NOT a fourth
  // rendered outcome - it renders nothing, because before the resolution lands there is no answer.
  const [listing, setListing] = useState<ProfileResolution | undefined>(undefined);
  useEffect(() => {
    if (!reader || !providerIdOk || !mirrorBase) {
      setListing(undefined);
      return;
    }
    let current = true;
    void (async () => {
      try {
        const anchor = await reader.providerProfileAnchor(providerId as `0x${string}`);
        const resolved = await resolveProviderProfile(
          { digest: anchor.digest, hashAlgorithm: anchor.hashAlgorithm, revision: anchor.revision },
          mirrorContentReader(mirrorBase),
          keccakBytes,
        );
        if (current) setListing(resolved);
      } catch (error) {
        // A failed ANCHOR read is not an absence either: the chain could not be asked, so nothing
        // about what this provider published has been established.
        if (current) {
          setListing({
            state: "unverified",
            reason: `the published record could not be read from the chain (${
              error instanceof Error && error.message ? error.message : "no reason given"
            })`,
            logo: { state: "unverified", reason: "the published record could not be read" },
          });
        }
      }
    })();
    return () => {
      current = false;
    };
  }, [reader, providerId, providerIdOk, mirrorBase, sent.length]);
  const caller = address as `0x${string}` | undefined;

  // A REFRESH MUST NOT EMPTY THE PROVIDER ID. It is a stable identity DogTag assigned, not a
  // transient input, and losing it deadens three flows at once with reasons pointing at a card the
  // operator has scrolled past - which is how a captain came to be told a field was empty on a card
  // whose only visible field he had just filled. Remembered per wallet (see `providerIdMemory.ts`),
  // restored when the wallet arrives, and switched with it: wallet B's remembered id - or the
  // deployment default when B has none - replaces wallet A's, so one provider's id is never shown
  // as another's. Flow 2's contract address is deliberately NOT given this treatment; the module
  // header owns that argument.
  const providerIdScope = useRef<string | null>(null);
  useEffect(() => {
    if (!caller || providerIdScope.current === caller) return;
    const firstScope = providerIdScope.current === null;
    providerIdScope.current = caller;
    if (firstScope && providerIdEdited) {
      // Typed before the wallet arrived: now there is a scope to remember it under.
      rememberProviderId(caller, providerId);
      return;
    }
    const stored = recallProviderId(caller);
    if (stored !== null) {
      setProviderId(stored);
      setProviderIdEdited(false);
    } else if (!firstScope) {
      // A wallet switch with nothing remembered must not carry the previous wallet's id across.
      setProviderId(defaultProviderId);
      setProviderIdEdited(false);
    }
  }, [caller, providerIdEdited, providerId, defaultProviderId]);

  // WHAT THIS WALLET HAS DEPLOYED. Read on mount and after every settled transaction, so a provider
  // returning to this page - on this browser or any other - sees their contracts without having
  // deployed anything again to find out. Keyed on `caller` alone: the record is the factory's own
  // creation log filtered by owner, and the provider id in the form is neither in the filter nor in
  // the address salt (see `deploymentHistory.ts`).
  useEffect(() => {
    if (!reader || !caller) {
      setDeployHistory(undefined);
      return;
    }
    let current = true;
    setHistoryLoading(true);
    void (async () => {
      const history = await readDeploymentHistory(caller, reader);
      if (current) {
        setDeployHistory(history);
        setHistoryLoading(false);
      }
    })();
    return () => {
      current = false;
      // Not cleared here: a superseded read must not blank an answer already on screen, and the
      // loading flag is what says a fresher one is on its way.
      setHistoryLoading(false);
    };
  }, [reader, caller, settled]);

  // The next free contract number, derived from what exists rather than from a counter. Scoped by
  // record type because that is what the factory's salt is scoped by.
  const suggestedNonce = useMemo(
    () => (deployHistory ? nextContractNumber(deployHistory, recordTypeKey) : null),
    [deployHistory, recordTypeKey],
  );
  // Pre-filled only while the operator has not taken the field over, and only from a KNOWN answer -
  // a number filled in from an incomplete read would collide on a deterministic address, and the
  // only sign of it would be this page's own check refusing a number this page put there.
  useEffect(() => {
    if (nonceEdited || suggestedNonce?.state !== "known") return;
    setCloneNonce(String(suggestedNonce.next));
  }, [nonceEdited, suggestedNonce]);
  const nonceInput = useMemo(() => parseContractNumber(cloneNonce), [cloneNonce]);

  // A FAULT MUST NOT OUTLIVE THE THING THAT CAUSED IT. It used to clear only at the start of the
  // next run(), so a wallet message stayed on screen while the user did exactly what it asked -
  // switched wallet, switched network, connected the right account - and went on reading as current
  // long after it had stopped being true. Which, for a message that was already being read as a
  // verdict about the provider, made it a verdict that could not be withdrawn.
  useEffect(() => {
    setFault(null);
  }, [isConnected, address, chainId]);

  // The input fingerprints. Anything a plan's answer depends on belongs in its key; a value left out
  // is a value that can be edited after Check without disabling the button, which is the whole bug.
  const identity = `${caller ?? ""}|${providerId}`;
  const deployKey = `${identity}|${recordTypeKey}|${cloneNonce}`;
  const cloneKey = `${identity}|${candidate}`;
  const domainKey = `${identity}|${candidate}`;
  // The logo is keyed by its CONTENT ADDRESS, not by the File object, whose identity changes on
  // every re-render and would retire a perfectly fresh plan on each keystroke. Leaving it out of
  // the key would be the stale-plan bug in its purest form: swap the logo after pressing Check and
  // the button stays enabled while the CHECKED plan still carries the old one.
  const logoAddress = useMemo(() => (logo ? publicationDigest(logo.bytes) : "none"), [logo]);
  const publicationKey = `${identity}|${latInput}|${lngInput}|${JSON.stringify(contacts)}|${logoAddress}`;
  // THE CHOSEN REGISTER IS IN THE KEY, and a dropdown is precisely the input that makes forgetting it
  // easy: check register A, switch the select to B, and without this the button stays enabled while
  // the CHECKED plan still carries A - so B would be sent unchecked. The directory key does not carry
  // `candidate`, because that selection is keyed by provider id (already in `identity`) and not by any
  // one contract.
  const domainRegisterKey = `${identity}|${candidate}|${domainRegisterChoice}`;
  const directoryRegisterKey = `${identity}|${directoryRegisterChoice}`;

  const deploy = fresh(deployHeld, deployKey);
  const clone = fresh(cloneHeld, cloneKey);
  const domainState = fresh(domainHeld, domainKey);
  const publication = fresh(publicationHeld, publicationKey);
  const domainRegister = fresh(domainRegisterHeld, domainRegisterKey);
  const directoryRegister = fresh(directoryRegisterHeld, directoryRegisterKey);

  const deployShown = shown(deployHeld);
  const cloneShown = shown(cloneHeld);
  const domainShown = shown(domainHeld);
  const publicationShown = shown(publicationHeld);
  const domainRegisterShown = shown(domainRegisterHeld);
  const directoryRegisterShown = shown(directoryRegisterHeld);
  const deployRetired = retiredBecause(deployHeld, deployKey);
  const cloneRetired = retiredBecause(cloneHeld, cloneKey);
  const domainRetired = retiredBecause(domainHeld, domainKey);
  const publicationRetired = retiredBecause(publicationHeld, publicationKey);
  const domainRegisterRetired = retiredBecause(domainRegisterHeld, domainRegisterKey);
  const directoryRegisterRetired = retiredBecause(directoryRegisterHeld, directoryRegisterKey);
  // Computed from the CHECKED plan when there is one, and from "a publication always uploads its
  // profile document" when there is not - so a deployment missing either setting says so before the
  // provider fills the form in, rather than after they press Publish.
  const publicationMirrorRefusal = mirrorPublicationRefusal(
    publication?.steps,
    mirrorBase,
    mirrorToken,
  );

  const run = useCallback(async (fn: () => Promise<void>) => {
    setBusy(true);
    setFault(null);
    try {
      await fn();
    } catch (e) {
      // A throw here is a fault in THIS surface or in the wallet - never a verdict about the
      // provider. Every chain READ the engine makes is caught and reported as its own could-not-run,
      // and every transaction OUTCOME is reported as its own record.
      //
      // THAT WAS ONCE ASSERTED HERE AS THOUGH IT SETTLED THE MATTER, AND IT DID NOT. A bare wallet
      // string was rendered unlabelled in red directly under the provider id, the record type and
      // the caller address - so a captain whose wallet answered EIP-1193 `4100 Unauthorized` read
      // it as his PROVIDER being unauthorized, while the admin portal showed that same provider
      // ACTIVE and approved for the record type. The comment said that reading was impossible; the
      // rendering made it the natural one. Being true about the origin is not the same as being
      // unmistakable on screen, and only the second is a property of the product.
      //
      // So the fault is now classified and rendered as a fault - what could not be done, that
      // nothing about the provider was established, and what to do - by `WalletFaultNotice`.
      setFault(classifySurfaceFault(e));
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
      /**
       * The contract address this send creates, for a send that creates one.
       *
       * Threaded onto EVERY state of the row rather than added at the end, so the address is on
       * screen from the moment the wallet is asked - which is the window the operator spends
       * wondering what is happening. `createdAddressMeaning` is what keeps each state's wording
       * honest about whether anything exists there yet.
       */
      opts: { createdAddress?: string } = {},
    ): Promise<SendState> => {
      const id = `send-${++sendSeq.current}`;
      const { createdAddress } = opts;
      const settle = (record: SendRecord) => {
        setSent((s) => s.map((r) => (r.id === record.id ? record : r)));
        // RE-READ THE CHAIN ON A TERMINAL STATE, never on submission. `submitted` means the hash
        // exists and nothing has mined, so a read triggered there comes back without the very thing
        // it was triggered to find - and, being the last trigger, would leave the page stale exactly
        // after the deploy the operator is watching for. The two could-not-tell states DO trigger
        // one, because that is the case where reading the chain is the only way to find out.
        if (record.state !== "submitted" && record.state !== "awaitingWallet") {
          setSettled((n) => n + 1);
        }
      };

      // RECORDED BEFORE THE WALLET IS ASKED. This is the whole fix: the row used to be created only
      // after `send()` resolved, so the entire wallet window had no on-screen state - and a wallet
      // that never answered left the page blank forever while a transaction mined on chain.
      setSent((s) => [sendRecord(id, what, "awaitingWallet", { createdAddress }), ...s]);

      // Follow a hash to its receipt. Extracted so the LATE path below - a wallet that answers after
      // we stopped waiting - gets exactly the same treatment rather than a second, thinner copy.
      const follow = async (hash: `0x${string}`): Promise<SendState> => {
        try {
          const receipt = await roaxPublicClient(rpcUrl).waitForTransactionReceipt({
            hash,
            timeout: RECEIPT_TIMEOUT_MS,
          });
          const state = outcomeFromReceiptStatus(receipt.status);
          settle(sendRecord(id, what, state, { hash, createdAddress }));
          return state;
        } catch (e) {
          // NOT a failure and NOT a success. A receipt we could not fetch says nothing about whether
          // the transaction mined - it is routed here rather than into the page-level fault notice,
          // which would collapse it into the same bucket as a wallet refusal.
          const reason = e instanceof Error ? e.message.split("\n")[0]! : String(e);
          settle(sendRecord(id, what, "unknown", { hash, unknownReason: reason, createdAddress }));
          return "unknown";
        }
      };

      const pending = send();
      let timedOut = false;
      let hash: `0x${string}`;
      try {
        hash = await Promise.race([
          pending,
          new Promise<never>((_, reject) =>
            setTimeout(() => {
              timedOut = true;
              reject(new Error("wallet-timeout"));
            }, WALLET_TIMEOUT_MS),
          ),
        ]);
      } catch (e) {
        if (!timedOut) {
          // A genuine rejection: nothing was submitted, so the attempt is withdrawn entirely rather
          // than left on screen as something that might have happened.
          setSent((s) => s.filter((r) => r.id !== id));
          throw e;
        }
        // The wallet has not answered. It MAY still have broadcast - so the plan is retired (it may
        // have been acted on) and the row says so, with the two things the user can actually do.
        retire();
        settle(
          sendRecord(id, what, "walletSilent", {
            createdAddress,
            unknownReason:
              "Your wallet did not respond in time. It may still have sent this - check your wallet's "
              + "activity, and run the check again: if the contract was created, the check reports that "
              + "contract number as already used and names the address.",
          }),
        );
        // If it answers late, upgrade the row and follow it properly rather than leaving a
        // could-not-tell standing next to a transaction we can now see.
        void pending.then((late) => void follow(late)).catch(() => {});
        return "walletSilent";
      }

      // The plan that authorized this is retired HERE, before the outcome is known, so every
      // terminal state inherits it and no handler can forget one. It is a required parameter rather
      // than an optional one so a new flow cannot silently omit it. A rejected signature never
      // reaches this line, which is correct: nothing was submitted, so nothing was falsified.
      retire();
      settle(sendRecord(id, what, "submitted", { hash, createdAddress }));
      return follow(hash);
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

  // ONE derivation of "why can this not be used", shared by all eleven controls. Written out here
  // rather than inline per button so that adding a control cannot quietly ship without a reason -
  // which is exactly how Deploy came to be dead and silent on first run.
  const pre = { busy, connected: isConnected && !!caller, hasReader: !!reader };
  const chain = { expected: ROAX_CHAIN_ID, actual: chainId };
  const gate = (missingInput?: string | null) => checkBlock({ ...pre, missingInput });

  // EACH FLOW'S CHECK BLOCK, DERIVED ONCE. Three things now read it - the sentence under the Check
  // button, the superseded banner (which must not instruct a Check that is unavailable), and the
  // send's own sentence (which must not repeat it) - and three call sites re-deriving it is three
  // chances for the banner to name an obstacle that is not the one in force.
  const deployCheckBlock = gate(
    !providerIdOk
      ? "Enter your provider id to check."
      : nonceInput.state === "invalid"
        ? "Enter a valid contract number to check."
        : null,
  );
  const repointCheckBlock = gate(
    !providerIdOk
      ? "Enter your provider id to check."
      : !candidate
        ? "Enter the address of the contract you deployed in step 1 to check it."
        : null,
  );
  const domainCheckBlock = gate(
    candidate
      ? null
      : "Enter your contract address in step 2 first. A domain is published for a "
        + "contract, so there is nothing to check until this page knows which one - "
        + "this button is gated on that field, not on the domain above.",
  );
  const publishCheckBlock = gate(providerIdOk ? null : "Enter your provider id to check.");


  const sendGate = (
    check: string,
    plan: Parameters<typeof sendBlock>[0]["plan"],
    otherwiseBlocked?: string | null,
  ) => sendBlock({ ...pre, chain, check, plan, otherwiseBlocked });

  const deployPlanState = planGateState({
    present: !!deployHeld,
    spent: !!deployHeld?.spent,
    keyMatches: deployHeld?.key === deployKey,
    canAct: !!deploy?.canDeploy,
    verdict: deploy?.verdict,
  });
  const clonePlanState = planGateState({
    present: !!cloneHeld,
    spent: !!cloneHeld?.spent,
    keyMatches: cloneHeld?.key === cloneKey,
    canAct: !!clone?.canRepoint,
    verdict: clone?.verdict,
  });
  const domainPlanState = planGateState({
    present: !!domainHeld,
    spent: !!domainHeld?.spent,
    keyMatches: domainHeld?.key === domainKey,
    canAct: !!domainState?.canWrite,
    verdict: domainState?.verdict,
  });
  const publicationPlanState = planGateState({
    present: !!publicationHeld,
    spent: !!publicationHeld?.spent,
    keyMatches: publicationHeld?.key === publicationKey,
    canAct: !!publication?.canPublish,
    verdict: publication?.verdict,
  });
  const domainRegisterPlanState = planGateState({
    present: !!domainRegisterHeld,
    spent: !!domainRegisterHeld?.spent,
    keyMatches: domainRegisterHeld?.key === domainRegisterKey,
    canAct: !!domainRegister?.canSelect,
    verdict: domainRegister?.verdict,
  });
  const directoryRegisterPlanState = planGateState({
    present: !!directoryRegisterHeld,
    spent: !!directoryRegisterHeld?.spent,
    keyMatches: directoryRegisterHeld?.key === directoryRegisterKey,
    canAct: !!directoryRegister?.canSelect,
    verdict: directoryRegister?.verdict,
  });

  /**
   * Why a register CHECK cannot run: the fields it needs, in the order a provider would fix them.
   *
   * The domain register's blocker is the contract address in step 2, which is in ANOTHER step - the
   * one shape a "<field> is required" template cannot carry, and the reason `missingInput` takes a
   * whole sentence.
   */
  const registerCheckMissing = (scope: ResolverSelectionScope): string | null => {
    const options = registerOptions[scope];
    if (scope === "domain" && !candidate) {
      return (
        "Enter your contract address in step 2 first. A domain register is chosen for one contract, "
        + "so there is nothing to check until this page knows which one."
      );
    }
    if (scope === "directory" && !providerIdOk) {
      return "Enter your provider id to check which directory register is selected.";
    }
    if (options.state === "pending") {
      return "Still reading which registers DogTag approves. This needs your wallet connected, because there is no backend here to read the chain for you.";
    }
    if (options.state === "unavailable") {
      return `The registers you may choose from could not be read (${options.reason}), so there is nothing to check against. That is this page's connection to the chain, not anything you have set up.`;
    }
    const choice = scope === "domain" ? domainRegisterChoice : directoryRegisterChoice;
    if (!choice) {
      return options.listings.some((l) => l.approved)
        ? "Pick a register above first."
        : "DogTag has approved no register of this kind yet, so there is nothing to pick. That half is theirs; ask them.";
    }
    return null;
  };
  const domainRegisterCheckBlock = gate(registerCheckMissing("domain"));
  const directoryRegisterCheckBlock = gate(registerCheckMissing("directory"));

  /** What the Send button will do, so its label states the action rather than describing the widget. */
  const registerSendLabel = (choice: string, scope: ResolverSelectionScope): string =>
    choice.toLowerCase() === ZERO_ADDR
      ? scope === "domain"
        ? "Stop using a domain register"
        : "Stop using the directory"
      : scope === "domain"
        ? "Use this domain register"
        : "Use this directory register";

  // ONE ORDERED LIST OF EVERY RENDERED REASON ON THE PAGE, deduped in source order.
  //
  // PAGE-WIDE rather than per-flow, because the obstacles that repeat are page-wide: a disconnected
  // wallet, a wrong chain, an action in flight and an unreachable node block every control there is.
  // Per-flow deduping fixed the adjacent repetition the captain reported and still left the same
  // four-line sentence four times down the page, once per flow. What does NOT collapse is anything
  // flow-specific - each flow's "run <its own check> first" and each flow's missing-field sentence
  // are different text, so they are each said in full where they belong.
  //
  // Built here rather than inline so the order the dedupe uses is the order of one array rather than
  // an assumption about when JSX children are evaluated, and so a control added later has to be
  // added to this list to get a reason at all - which is louder than silently inheriting none.
  const [
    deployCheckReason,
    deploySendReason,
    repointCheckReason,
    repointSendReason,
    domainRegisterCheckReason,
    domainRegisterSendReason,
    domainCheckReason,
    domainClaimReason,
    domainNoneReason,
    domainWithdrawReason,
    directoryRegisterCheckReason,
    directoryRegisterSendReason,
    publishCheckReason,
    publishSendReason,
    withdrawPinReason,
    // A CONTROL THAT WILL NOT RENDER CONTRIBUTES `null`, so a sentence is never spent on it.
    //
    // The groomer mounts this component with `issuance: false`, so flows 1-3 are not rendered at
    // all - and they are first in this order. Passing their blocks unconditionally assigned the full
    // sentence to a control that does not exist, and the only reason a groomer could see degraded to
    // the brief form. NOT redundant now that a brief reason keeps its identity: the brief form still
    // drops the remedy half only the full sentence carries - "using the button in the top right" for
    // a disconnected wallet is the pinned example - so the full sentence must still land on a
    // control that exists.
    //
    // Same rule as the withdraw-pin entry being last, applied one level up: there the gate is the
    // button's own condition, here it is the capability block around three whole flows.
  ] = sequenceReasons([
    ...(capabilities.issuance
      ? ([
          deployCheckBlock,
          sendGate("Check what this would deploy", deployPlanState),
          repointCheckBlock,
          sendGate("Check this contract", clonePlanState),
          // The register selection is FIRST within flow 3, matching the source order below, because
          // it is the step the rest of the flow is gated on - so it is the control that should carry
          // the full sentence when the whole flow shares an obstacle.
          domainRegisterCheckBlock,
          sendGate("Check the domain register", domainRegisterPlanState),
          domainCheckBlock,
          sendGate(
            "Check the domain record",
            domainPlanState,
            domainPlanState === "ready" && !validateDomain(domain).ok
              ? "Enter a valid domain to publish one. You can still declare that you deliberately have none."
              : null,
          ),
          sendGate("Check the domain record", domainPlanState),
          sendGate("Check the domain record", domainPlanState),
        ] as (ActionBlock | null)[])
      : [null, null, null, null, null, null, null, null, null, null]),
    ...(capabilities.listing
      ? ([
          directoryRegisterCheckBlock,
          sendGate("Check the directory register", directoryRegisterPlanState),
          publishCheckBlock,
          sendGate("Check what this would publish", publicationPlanState, publicationMirrorRefusal),
          // Flow 4's withdraw-pin send is conditionally RENDERED, so it is last within its flow: the
          // full sentence must land on a control that always exists.
          sendGate("Check what this would publish", publicationPlanState),
        ] as (ActionBlock | null)[])
      : [null, null, null, null, null]),
  ]);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle>Your provider record</CardTitle>
          <CardDescription>
            Your provider id is assigned by DogTag when you are approved. It is an opaque identifier -
            it is not derived from your name, your domain, your address or any of your keys.
            </CardDescription>
          </div>
          {demoMode && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                // Everything EXCEPT providerId: that one is registrar-assigned and opaque, so a
                // demo value would be a provider id that does not exist. The captain pastes the one
                // the admin registrar screen minted.
                setRecordType(DEMO_PROVIDER_LISTING.recordType);
                setDomain(DEMO_PROVIDER_LISTING.domain);
                setLatInput(DEMO_PROVIDER_LISTING.lat);
                setLngInput(DEMO_PROVIDER_LISTING.lng);
                setContacts((prev) => ({ ...prev, ...DEMO_PROVIDER_LISTING.contacts }));
              }}
            >
              <Sparkles className="h-4 w-4" /> Fill demo data
            </Button>
          )}
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Label htmlFor="providerId">Provider id</Label>
            <Input
              id="providerId"
              value={providerId}
              onChange={(e) => {
                setProviderIdEdited(true);
                setProviderId(e.target.value);
                // Written through as typed, so a refresh restores it. A no-op while no wallet is
                // connected - there is no scope to remember it under yet; the restore effect
                // catches up when one arrives. Clearing the field forgets it.
                rememberProviderId(caller, e.target.value);
              }}
              placeholder="0x…"
              className="font-mono"
            />
            {providerId && !providerIdOk ? (
              <p className="mt-1 text-xs text-red-700 dark:text-red-400">
                A provider id is 20 bytes: 0x followed by 40 hex characters.
              </p>
            ) : null}
            {/* Why this page is shorter here than a vet's. Without it a groomer sees a page with
                one card and no way to tell whether the rest is missing, hidden, or broken. */}
            {!capabilities.issuance ? (
              <p className="mt-1 text-xs text-muted-foreground" data-testid="listing-only-note">
                Your listing is the whole of this page for you. Issuing contracts, and the domain
                that belongs to one, apply to businesses that issue credentials - you verify them,
                so there is nothing here for you to deploy.
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
          <SendLog records={sent} />
        </CardContent>
      </Card>

      <WalletFaultNotice fault={fault} />

      {capabilities.issuance ? (
        <>
          {/* ---- Flow 1: deploy your own contract ------------------------------------------ */}
          <Card>
            <CardHeader>
              <CardTitle>1. Deploy your own contract</CardTitle>
              <CardDescription>
                You deploy it and you own it - DogTag does not deploy it for you. Check first: it
                shows you the exact address you are about to create, before anything is sent.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              {/* What already exists, BEFORE the form that would add to it. A provider who has just
                  deployed comes back to this page looking for one thing, and it is here. */}
              <DeployedContractsCard
                history={deployHistory}
                nextStep={DEPLOYED_CONTRACT_NEXT_STEP}
                refreshing={historyLoading}
              />
              <div>
                <Label htmlFor="cloneNonce">Contract number</Label>
                <Input
                  id="cloneNonce"
                  value={cloneNonce}
                  onChange={(e) => {
                    // Their number wins from here on. The pre-fill is a suggestion, and a field that
                    // kept re-filling itself would be choosing which contract to deploy for them.
                    setNonceEdited(true);
                    setCloneNonce(e.target.value);
                  }}
                  inputMode="numeric"
                />
                <p className="mt-1 text-xs text-muted-foreground">
                  This number is the only part of your contract&apos;s address that you choose. It is
                  filled in with the next free one; you can change it.
                </p>
                {/* PARSED, not trusted. The field is pre-filled and the operator is invited to
                    override it, so a non-numeric value is ordinary now - and unguarded it throws out
                    of `BigInt` into the wallet-fault notice, putting a surface fault where a verdict
                    about a typed field belongs. */}
                {nonceInput.state === "invalid" ? (
                  <p className="mt-1 text-xs text-red-700 dark:text-red-400" data-testid="nonce-invalid">
                    {nonceInput.reason}
                  </p>
                ) : null}
                {suggestedNonce ? <NextContractNumberNotice suggestion={suggestedNonce} /> : null}
                {suggestedNonce?.state === "known"
                && nonceInput.state === "ok"
                && nonceInput.value !== suggestedNonce.next ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="mt-1"
                    data-testid="use-next-number"
                    onClick={() => setCloneNonce(String(suggestedNonce.next))}
                  >
                    Use {String(suggestedNonce.next)}
                  </Button>
                ) : null}
                {/* The question the captain actually asked, answered where he asked it. The old
                    hint said what the number lets you DO and never why it exists, which is the
                    half that makes it make sense. */}
                <WhyThisExists question="Why is there a number at all?" testId="why-contract-number">
                  <p>
                    Your contract&apos;s address is not assigned to you - it is worked out in
                    advance from exactly three things: the record type, the wallet you deploy from,
                    and this number.
                  </p>
                  <p>
                    The first two are already fixed, so this number is the only one you can vary.
                    Without it every wallet would have exactly one possible address per record type,
                    for good - and if that contract ever had to be replaced, there would be nowhere
                    to move to.
                  </p>
                  <p>
                    That is what step 2 is for: deploy a contract under a new number, then select it
                    there. Use a new number if a key is compromised or you need to start fresh;
                    otherwise 0 is the answer.
                  </p>
                </WhyThisExists>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  disabled={!ready || !providerIdOk || nonceInput.state !== "ok"}
                  onClick={() =>
                    run(async () => {
                      // The PARSED value. `BigInt(cloneNonce || "0")` threw a SyntaxError out of
                      // here on any non-numeric entry, and the catch renders wallet faults.
                      if (nonceInput.state !== "ok") return;
                      const plan = await planCloneDeployment(
                        {
                          providerId: providerId as `0x${string}`,
                          recordType: recordTypeKey,
                          caller: caller!,
                          cloneNonce: nonceInput.value,
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
                      const { request, predictedAddress } = deploy!;
                      await sendAndFollow(
                        "Deploy contract",
                        () => setDeployHeld(spend),
                        () =>
                          writeContractAsync({
                            address: contracts.factory,
                            abi: FACTORY_ABI,
                            functionName: "createIssuer",
                            args: [request.providerId, request.recordType, request.cloneNonce],
                          }),
                        // The address the CHECKED plan predicted, so the row carries it from the
                        // moment the wallet is asked. Exact rather than a guess: the factory works
                        // the address out from the same three inputs whether it predicts or deploys.
                        // A plan whose prediction read failed passes none, and the row says nothing
                        // about an address rather than inventing one.
                        predictedAddress ? { createdAddress: predictedAddress } : {},
                      );
                    })
                  }
                >
                  Deploy
                </Button>
              </div>
              <ActionReason
                reason={deployCheckReason ?? null}
                testId="deploy-check-reason"
              />
              <ActionReason
                reason={deploySendReason ?? null}
                testId="deploy-send-reason"
              />
              <PlanNotice
                reason={deployRetired}
                testId="deploy-stale"
                checkBlocked={deployCheckBlock}
              />
              {deployShown ? (
                <DeployPlanCard
                  plan={deployShown}
                  attachmentNotice={ATTACHMENT_IS_NOT_SELF_SERVICE}
                  retired={deployRetired}
                />
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
              <DependencyNotice testId="repoint-dependency">
                {ATTACHMENT_IS_A_DOGTAG_STEP}
              </DependencyNotice>
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
                  Paste the address step 1 deployed. Only a contract deployed by the DogTag issuer
                  factory can be entered here - anything else is refused, whoever asks.
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
              <ActionReason
                reason={repointCheckReason ?? null}
                testId="repoint-check-reason"
              />
              <ActionReason
                reason={repointSendReason ?? null}
                testId="repoint-send-reason"
              />
              <PlanNotice
                reason={cloneRetired}
                testId="repoint-stale"
                checkBlocked={repointCheckBlock}
              />
              {cloneShown ? <CloneLifecycleCard assessment={cloneShown} retired={cloneRetired} /> : null}
            </CardContent>
          </Card>

          {/* ---- Flow 3: claim a domain ----------------------------------------------------- */}
          <Card>
            <CardHeader>
              <CardTitle>3. Your domain</CardTitle>
              <CardDescription>
                A domain belongs to one of your contracts, so this acts on the contract address you
                entered in step 2. Publishing no domain, and never having said, are different things
                here - and both are recorded as themselves.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              <DependencyNotice testId="domain-dependency">
                {DOMAIN_REGISTER_NEEDS_TURNING_ON}
              </DependencyNotice>
              {/* THE SELECTION COMES FIRST, because the rest of this flow is gated on it: without it
                  `claimStanding` answers `coreSelectsThisResolver: false` and every domain write is
                  refused. Before this shipped the provider had no way to make it at all. */}
              <RegisterPicker
                scope="domain"
                options={registerOptions.domain}
                choice={domainRegisterChoice}
                onChoice={setDomainRegisterChoice}
                allowStop={canStopUsing(domainRegister?.selection ?? domainRegisterShown?.selection)}
                checkLabel="Check the domain register"
                onCheck={() =>
                  run(async () => {
                    const plan = await assessResolverSelection({
                      scope: "domain",
                      subject: candidate as `0x${string}`,
                      chosen: domainRegisterChoice as `0x${string}`,
                      caller: caller!,
                      reader: reader!,
                    });
                    setDomainRegisterHeld({ key: domainRegisterKey, plan, spent: false });
                  })
                }
                checkDisabled={!!domainRegisterCheckBlock}
                checkReason={domainRegisterCheckReason ?? null}
                sendLabel={registerSendLabel(domainRegisterChoice, "domain")}
                onSend={() =>
                  run(async () => {
                    // Addresses the PLAN's own captured values, never the form's - see rule 1 in this
                    // file's header. Switching the dropdown after Check retires the plan, and this
                    // line is the second, independent guard against sending the unchecked value.
                    await sendAndFollow(
                      registerSendLabel(domainRegister!.chosen, "domain"),
                      () => setDomainRegisterHeld(spend),
                      () =>
                        writeContractAsync({
                          address: contracts.core,
                          abi: CORE_ABI,
                          functionName: "setDomainResolver",
                          args: [domainRegister!.subject, domainRegister!.chosen],
                        }),
                    );
                  })
                }
                sendDisabled={!!sendGate("Check the domain register", domainRegisterPlanState)}
                sendReason={domainRegisterSendReason ?? null}
                plan={domainRegisterShown}
                retired={domainRegisterRetired}
                planCheckBlock={domainRegisterCheckBlock}
              />
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
              <ActionReason
                reason={domainCheckReason ?? null}
                testId="domain-check-reason"
              />
              <ActionReason
                reason={domainClaimReason ?? null}
                testId="domain-claim-send-reason"
              />
              {/* Its own reason, because its own gate: declaring you have no domain does not need a
                  valid domain in the field, so sharing the claim button's sentence would tell a
                  provider to fix something this button never asked for. */}
              <ActionReason
                reason={domainNoneReason ?? null}
                testId="domain-none-send-reason"
              />
              {canWithdraw(domainState?.standing) ? (
                <ActionReason
                  reason={domainWithdrawReason ?? null}
                  testId="domain-withdraw-send-reason"
                />
              ) : null}
              <PlanNotice
                reason={domainRetired}
                testId="domain-stale"
                checkBlocked={domainCheckBlock}
              />
              {domainShown ? <DomainClaimCard assessment={domainShown} retired={domainRetired} /> : null}
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
            <DependencyNotice testId="directory-dependency">
              {DIRECTORY_NEEDS_TURNING_ON}
            </DependencyNotice>
            {/* THE SELECTION COMES FIRST, for the same reason as flow 3's: without it
                `ProviderDirectory.isLiveFor` is false and every publication is refused, so a provider
                who filled in five contact fields and a coordinate would have paid for the surprise.
                Measured on the live set: every provider's `directoryResolver` was the zero address,
                so this was the state EVERY provider was in. */}
            <RegisterPicker
              scope="directory"
              options={registerOptions.directory}
              choice={directoryRegisterChoice}
              onChoice={setDirectoryRegisterChoice}
              allowStop={canStopUsing(
                directoryRegister?.selection ?? directoryRegisterShown?.selection,
              )}
              checkLabel="Check the directory register"
              onCheck={() =>
                run(async () => {
                  const plan = await assessResolverSelection({
                    scope: "directory",
                    subject: providerId as `0x${string}`,
                    chosen: directoryRegisterChoice as `0x${string}`,
                    caller: caller!,
                    reader: reader!,
                  });
                  setDirectoryRegisterHeld({ key: directoryRegisterKey, plan, spent: false });
                })
              }
              checkDisabled={!!directoryRegisterCheckBlock}
              checkReason={directoryRegisterCheckReason ?? null}
              sendLabel={registerSendLabel(directoryRegisterChoice, "directory")}
              onSend={() =>
                run(async () => {
                  await sendAndFollow(
                    registerSendLabel(directoryRegister!.chosen, "directory"),
                    () => setDirectoryRegisterHeld(spend),
                    () =>
                      writeContractAsync({
                        address: contracts.core,
                        abi: CORE_ABI,
                        functionName: "setDirectoryResolver",
                        args: [directoryRegister!.subject, directoryRegister!.chosen],
                      }),
                  );
                })
              }
              sendDisabled={!!sendGate("Check the directory register", directoryRegisterPlanState)}
              sendReason={directoryRegisterSendReason ?? null}
              plan={directoryRegisterShown}
              retired={directoryRegisterRetired}
              planCheckBlock={directoryRegisterCheckBlock}
            />
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
            <div>
              <Label htmlFor="provider-logo">Logo (optional)</Label>
              <input
                id="provider-logo"
                type="file"
                data-testid="logo-input"
                accept={SERVABLE_IMAGE_MEDIA_TYPES.join(",")}
                className="mt-1 block w-full text-sm text-muted-foreground file:mr-3 file:rounded-md file:border file:border-input file:bg-background file:px-3 file:py-1.5 file:text-sm"
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  // Clearing the field means PUBLISH NO LOGO, not "leave the old one". The key
                  // moves with it, so a plan checked with a logo cannot be sent without one.
                  if (!file) {
                    setLogo(null);
                    setLogoName("");
                    setLogoError(null);
                    return;
                  }
                  void (async () => {
                    const bytes = new Uint8Array(await file.arrayBuffer());
                    const verdict = checkLogoPublication(bytes, file.type);
                    if (!verdict.ok) {
                      // Refused files leave NO logo selected and say which rule they broke. A
                      // silent drop would leave the provider believing they had published one.
                      setLogo(null);
                      setLogoName("");
                      setLogoError(verdict.reason);
                      return;
                    }
                    setLogo({ bytes, mediaType: file.type as LogoPublication["mediaType"] });
                    setLogoName(file.name);
                    setLogoError(null);
                  })();
                }}
              />
              {logoError ? (
                <p className="mt-1 text-xs text-amber-700 dark:text-amber-400" data-testid="logo-error">
                  {logoError}
                </p>
              ) : logo ? (
                <p className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
                  <span data-testid="logo-selected">{logoName} will be published.</span>
                  <button
                    type="button"
                    data-testid="logo-clear"
                    className="underline"
                    onClick={() => {
                      setLogo(null);
                      setLogoName("");
                      setLogoError(null);
                    }}
                  >
                    Publish no logo
                  </button>
                </p>
              ) : (
                <p className="mt-1 text-xs text-muted-foreground">
                  Publishing no logo is a normal listing. A logo is published to the content mirror
                  under its own fingerprint, and a reader that cannot match it shows no image at all
                  rather than a stand-in.
                </p>
              )}
            </div>
            <p className="text-xs text-muted-foreground">{CONTACT_ONLY_NOTICE}</p>
            {publicationMirrorRefusal ? (
              <p className="text-xs text-amber-700 dark:text-amber-400" data-testid="publish-mirror-refusal">
                {publicationMirrorRefusal}
              </p>
            ) : null}
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
                        logo,
                      },
                      reader!,
                      publicationDigest,
                    );
                    setPublicationHeld({ key: publicationKey, plan, spent: false });
                  })
                }
              >
                Check what this would publish
              </Button>
              <Button
                disabled={!ready || !publication?.canPublish || !!publicationMirrorRefusal}
                data-testid="publish-send"
                onClick={() =>
                  run(async () => {
                    // Sent in PLAN ORDER, from the plan's own captured digest, coordinates and
                    // provider id. The anchor lists the provider, so it goes first - and for a
                    // contact-only provider there is simply no second step, which is the whole of
                    // the no-placeholder rule: the absent pin is absent from the loop, not skipped
                    // inside it.
                    const plan = publication!;
                    // Refused BEFORE the first step, never during. Discovering this from inside the
                    // loop aborted the publication mid-sequence, and the missing-token case
                    // surfaced as a bare "missing bearer token" from the HTTP layer.
                    const refusal = mirrorPublicationRefusal(plan.steps, mirrorBase, mirrorToken);
                    if (refusal) throw new Error(refusal);
                    for (const step of plan.steps) {
                      // A mirror upload is an HTTP publication, not a transaction, and every one of
                      // them precedes the anchor. It has no receipt to follow, so it is handled here
                      // rather than through `sendAndFollow`; a failure THROWS, which `run` surfaces
                      // and which stops the loop before the anchor is sent. That ordering is the
                      // point: the irreversible write must never name content the mirror lacks.
                      if (step.kind === "mirrorUpload") {
                        await putMirrorContent(
                          mirrorBase!,
                          step.address,
                          step.bytes,
                          step.mediaType,
                          mirrorToken,
                        );
                        continue;
                      }
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
            <ActionReason
              reason={publishCheckReason ?? null}
              testId="publish-check-reason"
            />
            <ActionReason
              reason={publishSendReason ?? null}
              testId="publish-send-reason"
            />
            {publication?.canWithdrawPin ? (
              <ActionReason
                reason={withdrawPinReason ?? null}
                testId="withdraw-pin-send-reason"
              />
            ) : null}
            <PlanNotice
              reason={publicationRetired}
              testId="publish-stale"
              checkBlocked={publishCheckBlock}
            />
            {publicationShown ? (
              <DirectoryPublicationCard
                plan={publicationShown}
                contactOnlyNotice={CONTACT_ONLY_NOTICE}
                anchoredNotice={CONTACTS_ARE_ANCHORED_NOT_SERVED}
                retired={publicationRetired}
              />
            ) : null}
            {/* What a READER sees, resolved through the mirror rather than restated from the form.
                Rendered only once a provider id is well formed, because before that there is no
                record to resolve and "nothing published" would be a claim about nobody. */}
            {providerIdOk ? (
              <PublishedListingCard
                resolution={listing}
                providerName={providerId}
                unconfigured={!mirrorBase}
              />
            ) : null}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

/**
 * Says why the button is off, above answers that are still on screen.
 *
 * The panel below it IS rendered - see {@link shown}. Dropping it was the earlier defect: the copy
 * referred to "what is shown below" while nothing was, and it destroyed the one thing a provider
 * most wants right after signing, which is what they had checked. So the card stays and is LABELLED,
 * here and again on the verdict itself.
 *
 * Deliberately not `text-xs`: a label a reader can miss is not a state. Warning tone, in the flow of
 * the card, legible without hovering or interacting - the same treatment this repo gives every other
 * value it cannot stand behind.
 */
function PlanNotice({
  reason,
  testId,
  checkBlocked = null,
}: {
  reason: PlanRetirement | null;
  testId: string;
  /**
   * Why the Check button that would clear this is itself unavailable, or `null` when it is not.
   *
   * REQUIRED IN PRACTICE AT EVERY CALL SITE, because this banner's whole job is to name a remedy and
   * the remedy is that button. Told nothing, it used to instruct "Check again before sending" over a
   * greyed-out Check - a screen instructing an action it had disabled.
   */
  checkBlocked?: ActionBlock | null;
}): ReactNode {
  if (!reason) return null;
  return (
    <div
      className="rounded-md border border-amber-500/50 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-300"
      data-testid={reason === "spent" ? `${testId}-spent` : testId}
    >
      <p className="font-semibold">
        {reason === "edited"
          ? "Superseded: these answers are about earlier values"
          : "Superseded: these answers were read before your transaction"}
      </p>
      <p className="mt-1">{describePlanRetirement(reason, checkBlocked)}</p>
    </div>
  );
}

const SEND_STATE_STYLE: Readonly<Record<SendState, string>> = {
  // Neutral while we are simply waiting for a person to act - nothing is wrong yet, and painting it
  // as a warning would report an ordinary pause as a problem.
  awaitingWallet: "text-muted-foreground",
  // Amber, not neutral: an outcome nobody has established yet is a gap in what is known.
  submitted: "text-amber-700 dark:text-amber-400",
  succeeded: "text-emerald-700 dark:text-emerald-400",
  reverted: "text-red-700 dark:text-red-400",
  unknown: "text-amber-700 dark:text-amber-400",
  // Amber and never red: a wallet that did not answer has not failed the transaction.
  walletSilent: "text-amber-700 dark:text-amber-400",
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
          <li key={r.id} data-testid={`sent-${r.state}`}>
            <span className="text-muted-foreground">{r.what}</span>{" "}
            <span className={SEND_STATE_STYLE[r.state]}>{sendStateLabel(r.state)}</span>
            <br />
            {/* No hash yet is its own case, not an empty line: a blank where a transaction id
                belongs reads as one that failed to produce one. */}
            {r.hash ? (
              href ? (
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
              )
            ) : (
              <span className="text-muted-foreground">
                No transaction id yet - your wallet has not returned one.
              </span>
            )}
            {r.unknownReason ? (
              // Printed, not hovered - the same rule the check rows follow. This sentence is what
              // distinguishes "we could not follow it" from "it failed".
              <span className="block text-amber-700 dark:text-amber-400">
                Why the outcome is not known: {r.unknownReason}
              </span>
            ) : null}
            {/* THE ADDRESS, on the row, from the moment the wallet is asked. A hash alone leaves the
                operator to open an explorer and decode a log to find out what they just created,
                which is the value they actually need in hand. The caption is derived from the state
                rather than written here, so a pending or reverted row cannot present an address as a
                contract that exists. */}
            {r.createdAddress ? (
              <span className="block" data-testid="sent-created-address">
                <span className="text-muted-foreground">{createdAddressMeaning(r)}</span>{" "}
                <span className="break-all font-mono">{r.createdAddress}</span>
              </span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
