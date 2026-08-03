/**
 * The REGISTRAR surface for the generation-2 `ProviderRegistry` (registry plan C-2).
 *
 * This is the admin half of the provider journey. Until it existed, `registerProvider` and
 * `setServiceCreationApproval` had no caller outside contracts and tests, `providerCount()` was 0,
 * and every action on the provider self-service page refused.
 *
 * Three things here are load-bearing rather than decorative, all of them lessons the S-15 provider
 * surface paid for:
 *
 *  - A CHECK authorises a send of the values it checked, never of whatever the form holds when the
 *    button is pressed. Editing any input retires the plan. Here that matters more than it did on
 *    the provider side: `providerId` cannot be reassigned and the identity anchor's revision only
 *    moves forward, so a send of unreviewed values is not correctable.
 *  - A submitted transaction is not a completed one. The backend awaits the receipt and errors on a
 *    reverted status, so `outcome: "executed"` genuinely means mined-and-succeeded - and the two
 *    "nothing was broadcast" outcomes stay apart, because a designed proposal and a wrong hosted key
 *    have different remedies.
 *  - Three states, never two. An approval log that could not be READ is rendered as its own state
 *    with its reason, never as "approved for nothing".
 *  - A dispatch is a RECORD, not a transient. Nothing broadcast means the operator holds unsigned
 *    calldata they must sign out of band, so it outlives the send, the provider row, and a failed
 *    reload - see `DispatchLog`.
 */

import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  CopyButton,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TxRef,
  txExplorerHref,
  useToast,
  type AttachPreflightResp,
  type CapabilitiesRead,
  type DispatchOutcome,
  type GovernanceDisposition,
  type ProviderApprovalsRead,
  type ProviderRegistrarView,
  type ProviderServicesResp,
  type ProviderServiceView,
  type ProviderStanding,
  type ProvidersResp,
  type ResolverKind,
  type ResolversResp,
  type VerifierCapabilitiesResp,
  DEMO_PROVIDER_REGISTRATION,
} from "@dogtag/ui";
import {
  AlertTriangle,
  BadgeCheck,
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Dices,
  HelpCircle,
  Link2,
  Plus,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
  UserPlus,
  Sparkles,
} from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";
import { AddressRef } from "../components/ChainRef";
import { shortAddr } from "../lib/format";
import {
  canonicalIdentityStatement,
  checkRegistration,
  EMPTY_IDENTITY_STATEMENT,
  generateProviderId,
  registrationKey,
  type CheckedRegistration,
  type IdentityStatement,
} from "../lib/providerIdentity";
import {
  attachKey,
  attachSendable,
  effectiveTerms,
  issuanceBlocker,
  resolvedGenerationId,
  resolvedOwner,
  STANDING_TONE,
  termTone,
} from "../lib/providerServices";

/** The record types a provider can be pre-authorized to create a service for. */
const RECORD_TYPES = ["DOG_PROFILE", "VACCINATION", "GROOMING", "BOARDING"] as const;

/**
 * What a standing MEANS for the provider's ability to act, in the registrar's terms.
 *
 * `pending` is the one worth spelling out: it is what registration produces, it looks benign, and it
 * is indistinguishable from `active` on a badge alone - while `canWriteProvider` refuses everything
 * but `active`, so a provider left here can do nothing and the self-service page will say so.
 */
const STANDING_MEANING: Record<ProviderStanding, string> = {
  none: "Not registered.",
  pending: "Registered but INERT - every self-service action refuses until this is Active.",
  active: "Cleared to act.",
  suspended: "Frozen. Reversible.",
  retired: "Terminal - no further standing change is possible.",
};

/**
 * One dispatched registrar action, kept as a RECORD rather than as the latest state of a slot.
 *
 * A `proposed` disposition broadcasts nothing, so the unsigned calldata IS the deliverable: the
 * operator must carry it to the holder before the action exists at all. Three things follow, and the
 * previous single-slot-per-provider shape held none of them - which made a propose-only deployment,
 * the posture a cautious operator would choose, the one that could not complete a registration.
 *
 *  - It is keyed by the ACTION it came from, so approving VACCINATION and then GROOMING keeps both.
 *  - It is never destroyed by a later, unrelated action.
 *  - It renders outside the provider table, so it survives a failed reload AND the case where no row
 *    exists at all - which is exactly the proposed REGISTRATION, since nothing was broadcast and the
 *    id is therefore absent from `_providerIds`.
 */
interface DispatchRecord {
  key: string;
  providerId: string;
  summary: string;
  outcome: DispatchOutcome;
  warning?: string | null;
  actions: GovernanceDisposition[];
  /**
   * The registrar's identity statement as REVIEWED, carried here on a registration.
   *
   * The text is never sent to the backend and is stored nowhere, so once the dialog closes this is
   * the only copy that exists - and the screen asked the operator to keep their own on the strength
   * of it still being here to copy.
   */
  reviewed?: { canonical: string; digest: string } | null;
}

/**
 * A dispatched action, with everything the operator needs to finish it.
 *
 * The calldata is shown truncated but offered WHOLE through `CopyButton`: middle-truncation removes
 * the elided characters from the DOM entirely, so for a value that must be signed elsewhere a
 * truncated rendering with no copy affordance is not a smaller version of the deliverable - it is
 * the absence of one.
 */
function DispatchEntry({ record }: { record: DispatchRecord }) {
  const tone =
    record.outcome === "executed"
      ? "border-emerald-500/40 bg-emerald-500/10"
      : record.outcome === "proposed_by_design"
        ? "border-amber-500/40 bg-amber-500/10"
        : "border-destructive/40 bg-destructive/10";
  return (
    <div className={`rounded-md border p-3 text-sm ${tone}`} data-testid="dispatch-record">
      <div className="flex flex-wrap items-center gap-2 font-medium">
        <span>
          {record.outcome === "executed"
            ? `Mined: ${record.summary}`
            : `Nothing was broadcast - ${record.summary}`}
        </span>
        <span className="font-mono text-xs font-normal">{shortAddr(record.providerId)}</span>
        <CopyButton value={record.providerId} label="provider id" />
      </div>
      {record.warning ? <p className="mt-1 text-muted-foreground">{record.warning}</p> : null}
      <div className="mt-2 space-y-2">
        {/* Defensive on a REQUIRED field, because this component renders the operator's only copy of
            unsigned calldata: a throw here unmounts the whole log and takes every payload recorded
            before it, which is precisely what `DispatchLog` exists to prevent. A response carrying no
            actions says so rather than rendering as an empty, successful-looking record. */}
        {(record.actions ?? []).length === 0 ? (
          <p className="text-xs text-amber-600 dark:text-amber-500">
            The backend reported no dispatched actions for this request, so there is nothing here to
            sign or to look up.
          </p>
        ) : null}
        {(record.actions ?? []).map((a, i) => (
          <div key={i} className="text-xs">
            {a.disposition === "executed" ? (
              <TxRef event={{ txHash: a.txHash }} href={txExplorerHref({ txHash: a.txHash })} />
            ) : (
              <div className="space-y-1">
                <div className="flex items-center gap-1">
                  <span className="text-muted-foreground">sign as</span>
                  <span className="font-mono">{shortAddr(a.holder ?? "")}</span>
                  {a.holder ? <CopyButton value={a.holder} label="holder" /> : null}
                  <span className="text-muted-foreground">to</span>
                  <span className="font-mono">{shortAddr(a.target)}</span>
                  <CopyButton value={a.target} label="target" />
                </div>
                <div className="flex items-start gap-1">
                  <span className="shrink-0 text-muted-foreground">calldata</span>
                  <span className="font-mono break-all">{a.calldata.slice(0, 26)}…</span>
                  <CopyButton value={a.calldata} label="calldata" />
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
      {record.reviewed ? (
        <div className="mt-2 border-t pt-2">
          <div className="flex items-center gap-1 text-xs">
            <span className="text-muted-foreground">identity digest</span>
            <span className="font-mono break-all">{record.reviewed.digest}</span>
            <CopyButton value={record.reviewed.digest} label="identity digest" />
          </div>
          <div className="mt-1 flex items-center gap-1 text-xs">
            <span className="text-muted-foreground">statement as reviewed</span>
            <CopyButton value={record.reviewed.canonical} label="statement" />
          </div>
          <pre className="mt-1 whitespace-pre-wrap rounded bg-background/60 p-2 text-xs">
            {record.reviewed.canonical}
          </pre>
        </div>
      ) : null}
    </div>
  );
}

/**
 * The durable log, rendered independently of the provider table.
 *
 * Deliberately OUTSIDE the card that branches on loading / read-error / empty: every one of those
 * branches replaces the table, and a payload that lived inside it went with them.
 */
function DispatchLog({ records }: { records: DispatchRecord[] }) {
  if (records.length === 0) return null;
  const outstanding = records.filter((r) => r.outcome !== "executed").length;
  return (
    <Card data-testid="dispatch-log">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <ClipboardList className="h-4 w-4" />
          Registrar actions this session
        </CardTitle>
        <CardDescription>
          {outstanding > 0 ? (
            <>
              <span className="font-medium">
                {outstanding} action{outstanding === 1 ? "" : "s"} broadcast nothing
              </span>{" "}
              - the unsigned calldata below is the whole deliverable, and it is not stored anywhere
              once this page is left.
            </>
          ) : (
            <>What was sent from this page, kept so a payload is never lost to a later action.</>
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {[...records].reverse().map((r) => (
          <DispatchEntry key={r.key} record={r} />
        ))}
      </CardContent>
    </Card>
  );
}

/**
 * The registrar's own identity assertion for a provider, as it stands ON CHAIN.
 *
 * Three states, the same discipline as the approvals cell: an anchor that could not be read is its
 * own state and never a missing one. This is what makes the statement-is-stored-nowhere limitation
 * survivable - an admin who kept their copy can compare its digest against what the chain holds.
 */
function IdentityAnchor({ anchor }: { anchor: ProviderRegistrarView["identityAnchor"] }) {
  if (anchor === null) {
    return <p className="mt-1 text-xs text-muted-foreground">No identity anchor.</p>;
  }
  if ("unavailable" in anchor) {
    return (
      <p className="mt-1 text-xs text-amber-600 dark:text-amber-500">
        Identity anchor could not be read.{" "}
        <span className="text-muted-foreground">{anchor.unavailable}</span>
      </p>
    );
  }
  return (
    <div className="mt-1 flex items-center gap-1 text-xs" data-testid="identity-anchor">
      <span className="text-muted-foreground">identity</span>
      <span className="font-mono">{shortAddr(anchor.digest)}</span>
      <CopyButton value={anchor.digest} label="identity digest" />
      <span className="text-muted-foreground">rev {anchor.revision}</span>
    </div>
  );
}

/**
 * The approvals cell - three states, and the third is NOT a neighbour of the other two.
 *
 * "The registrar has approved nothing" is a fact about the provider. "The approval log could not be
 * read" is a fact about us, and rendering it as the first would tell the admin something about a
 * provider on the strength of a read that never happened.
 */
function Approvals({ approvals }: { approvals: ProviderApprovalsRead }) {
  if (approvals.state === "unavailable") {
    return (
      <div className="flex items-start gap-2 text-sm text-amber-600 dark:text-amber-500">
        <HelpCircle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>
          <span className="font-medium">Could not be read.</span>{" "}
          <span className="text-muted-foreground">{approvals.reason}</span>
        </span>
      </div>
    );
  }
  const granted = approvals.entries.filter((e) => e.allowed);
  const withdrawn = approvals.entries.filter((e) => !e.allowed);
  if (granted.length === 0 && withdrawn.length === 0) {
    return <span className="text-sm text-muted-foreground">Approved for nothing yet.</span>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {granted.map((e) => (
        <Badge key={e.recordTypeKey} variant="success">
          {e.recordType ?? shortAddr(e.recordTypeKey)}
        </Badge>
      ))}
      {/* A withdrawn approval is shown rather than dropped: "approved then withdrawn" is a different
          fact from "never approved", and only the log can tell them apart. */}
      {withdrawn.map((e) => (
        <Badge key={e.recordTypeKey} variant="outline" className="line-through opacity-70">
          {e.recordType ?? shortAddr(e.recordTypeKey)}
        </Badge>
      ))}
    </div>
  );
}

/**
 * The holders of one capability - three states, same discipline as {@link Approvals}.
 *
 * "Nobody holds this" is a fact about the service; "the log could not be read" is a fact about us,
 * and rendering the second as the first would say a service has no issuer on the strength of a read
 * that never happened.
 */
function CapabilityHolders({ read, empty }: { read: CapabilitiesRead; empty: string }) {
  if (read.state === "unavailable") {
    return (
      <div className="flex items-start gap-2 text-xs text-amber-600 dark:text-amber-500">
        <HelpCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <span>
          <span className="font-medium">Could not be read.</span>{" "}
          <span className="text-muted-foreground">{read.reason}</span>
        </span>
      </div>
    );
  }
  const granted = read.entries.filter((e) => e.allowed);
  const withdrawn = read.entries.filter((e) => !e.allowed);
  if (granted.length === 0 && withdrawn.length === 0) {
    return <span className="text-xs text-muted-foreground">{empty}</span>;
  }
  return (
    <div className="flex flex-wrap items-center gap-1">
      {granted.map((e) => (
        <span key={e.holder} className="flex items-center gap-1">
          <Badge variant="success" className="font-mono text-[11px]">
            {shortAddr(e.holder)}
          </Badge>
          <CopyButton value={e.holder} label="holder" />
        </span>
      ))}
      {/* A withdrawn grant is shown rather than dropped: "granted then withdrawn" is a different
          fact from "never granted", and only the log can tell them apart. */}
      {withdrawn.map((e) => (
        <Badge
          key={e.holder}
          variant="outline"
          className="font-mono text-[11px] line-through opacity-70"
        >
          {shortAddr(e.holder)}
        </Badge>
      ))}
    </div>
  );
}

/**
 * One attached service, with the five lifecycle terms reported APART.
 *
 * Never folded into one "can issue" badge: each term has its own remedy, so a single bool would tell
 * an admin something is wrong while withholding the only thing that says what to do about it.
 */
function ServiceRow({
  view,
  busy,
  onStanding,
  onGrant,
}: {
  view: ProviderServiceView;
  busy: boolean;
  onStanding: (service: string, standing: "active" | "suspended") => void;
  onGrant: (service: string) => void;
}) {
  const s = view.service;
  const terms = effectiveTerms(view.effective, view.currentPointer);
  const blocker = issuanceBlocker(terms);
  return (
    <div className="rounded-md border p-3" data-testid="service-row">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <AddressRef address={s.serviceAddress} />
          <Badge variant="outline">{s.recordType ?? shortAddr(s.recordTypeKey)}</Badge>
          <Badge variant={STANDING_TONE[s.standing]}>{s.standing}</Badge>
        </div>
        <div className="flex gap-1">
          {s.standing !== "active" && s.standing !== "retired" ? (
            <Button size="sm" disabled={busy} onClick={() => onStanding(s.serviceAddress, "active")}>
              <BadgeCheck className="mr-1 h-3.5 w-3.5" />
              Activate
            </Button>
          ) : null}
          {s.standing === "active" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => onStanding(s.serviceAddress, "suspended")}
            >
              Suspend
            </Button>
          ) : null}
          <Button size="sm" variant="outline" disabled={busy} onClick={() => onGrant(s.serviceAddress)}>
            <ShieldCheck className="mr-1 h-3.5 w-3.5" />
            Issuance capability
          </Button>
        </div>
      </div>

      <div className="mt-2 flex flex-wrap gap-1" data-testid="service-terms">
        {terms.map((t) => (
          <Badge key={t.key} variant={termTone(t.held)} title={t.held === true ? undefined : t.remedy}>
            {t.label}
            {t.held === null ? " ?" : t.held ? " ✓" : " ✗"}
          </Badge>
        ))}
      </div>
      {blocker ? (
        <p className="mt-1 text-xs text-amber-600 dark:text-amber-500" data-testid="service-blocker">
          {blocker}
        </p>
      ) : null}

      {/*
        REGISTRY-WIDE, and labelled as such. The issue right is keyed on the ADDRESS, so this is the
        same list on every service row in the system - a heading reading "may issue on this contract"
        would imply a scope the data no longer has, which is how an operator comes to believe a
        holder is confined to one provider.
      */}
      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
        <span className="text-muted-foreground">holds the issue right (registry-wide):</span>
        <CapabilityHolders read={view.issuance} empty="Nobody yet - grant the issue right." />
      </div>
      <p className="mt-0.5 text-[11px] text-muted-foreground" data-testid="issuance-scope-note">
        The issue right is granted on an address and names no service, so every holder above can
        anchor on any service in effective standing - not only this one.
      </p>

      <div className="mt-1 text-xs text-muted-foreground" data-testid="service-pointer">
        {view.currentPointer.state === "unavailable" ? (
          <>The provider&apos;s current pointer could not be read. {view.currentPointer.reason}</>
        ) : view.currentPointer.isCurrent ? (
          <>The provider has published this as its current {s.recordType ?? "record type"} service.</>
        ) : (
          // The provider's own `repointService` decision; no registrar route writes it, so this is
          // reported and never offered as an action here.
          <>
            Not published as current — the provider selects this itself on their portal; the registrar
            cannot do it for them.
          </>
        )}
      </div>
    </div>
  );
}

/**
 * The verify axis, keyed by PURPOSE.
 *
 * Deliberately its own card rather than a column in a service row: `setVerifierCapability` takes a
 * purpose and a relayer and no service at all, so rendering it inside a service would present it as
 * a property of that service. An issuer is not implicitly a verifier.
 */
function VerifierCapabilities({
  data,
  loadError,
  busy,
  onSet,
  onRefresh,
}: {
  data: VerifierCapabilitiesResp | null;
  loadError: string | null;
  busy: boolean;
  onSet: (purpose: string) => void;
  onRefresh: () => void;
}) {
  return (
    <Card data-testid="verifier-capabilities">
      <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
        <div>
          <CardTitle className="text-base">Verify capability</CardTitle>
          <CardDescription>
            Who may submit verifications, per purpose. This axis is{" "}
            <span className="font-medium">separate from issuance</span>: an issuer is not implicitly a
            verifier, and a relayer granted here can verify without being able to issue anything.
          </CardDescription>
        </div>
        <Button variant="outline" size="sm" onClick={onRefresh} disabled={busy}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </CardHeader>
      <CardContent className="space-y-2">
        {loadError ? (
          <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              <span className="font-medium">Verify capability could not be read.</span>{" "}
              <span className="text-muted-foreground">{loadError}</span>
            </span>
          </div>
        ) : !data ? (
          <Spinner />
        ) : (
          data.purposes.map((p) => (
            <div
              key={p.purpose}
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2"
            >
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{p.purpose}</Badge>
                <CapabilityHolders read={p.relayers} empty="No relayer may verify for this purpose." />
              </div>
              <Button size="sm" variant="outline" disabled={busy} onClick={() => onSet(p.purpose)}>
                Grant / withdraw
              </Button>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The typed resolver allowlist - the fleet-wide lever a provider's domain claim and directory
 * listing both need before either can work.
 *
 * Approval and SELECTION are two different halves and are never merged into one "working" badge: the
 * core keeps a stored selection when a resolver is deapproved, which is precisely why approval is a
 * separate lever, and only the provider can select.
 */
function Resolvers({
  data,
  loadError,
  busy,
  onSet,
  onRefresh,
}: {
  data: ResolversResp | null;
  loadError: string | null;
  busy: boolean;
  onSet: (kind: ResolverKind) => void;
  onRefresh: () => void;
}) {
  return (
    <Card data-testid="resolvers">
      <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
        <div>
          <CardTitle className="text-base">Typed resolvers</CardTitle>
          <CardDescription>
            A resolver answers nothing until you approve it here{" "}
            <span className="font-medium">and</span> the provider selects it. Approving is the whole
            of the registrar&apos;s part - without it a provider&apos;s domain claim and directory
            listing both refuse with <code>ResolverNotApproved</code>.
          </CardDescription>
        </div>
        <Button variant="outline" size="sm" onClick={onRefresh} disabled={busy}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </CardHeader>
      <CardContent className="space-y-2">
        {loadError ? (
          <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              <span className="font-medium">The resolver allowlist could not be read.</span>{" "}
              <span className="text-muted-foreground">{loadError}</span>
            </span>
          </div>
        ) : !data ? (
          <Spinner />
        ) : (
          data.kinds.map((k) => (
            <div
              key={k.kind}
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2"
            >
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{k.kind}</Badge>
                {k.listing.state === "unavailable" ? (
                  <span className="flex items-start gap-1 text-xs text-amber-600 dark:text-amber-500">
                    <HelpCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    <span>
                      <span className="font-medium">Could not be read.</span>{" "}
                      <span className="text-muted-foreground">{k.listing.reason}</span>
                    </span>
                  </span>
                ) : k.listing.resolvers.length === 0 ? (
                  <span className="text-xs text-muted-foreground">
                    None approved - every {k.kind} claim refuses until one is.
                  </span>
                ) : (
                  k.listing.resolvers.map((r) => (
                    <span key={r.resolver} className="flex items-center gap-1">
                      <Badge
                        variant={r.approved ? "success" : "outline"}
                        className={`font-mono text-[11px] ${r.approved ? "" : "line-through opacity-70"}`}
                      >
                        {shortAddr(r.resolver)}
                      </Badge>
                      <CopyButton value={r.resolver} label="resolver" />
                    </span>
                  ))
                )}
              </div>
              <Button size="sm" variant="outline" disabled={busy} onClick={() => onSet(k.kind)}>
                Approve / pull
              </Button>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}

/**
 * What a capability dialog is being opened FOR. One shape, three uses.
 *
 * These three writes each name an address that gains a power, so they get the same reviewed dialog
 * rather than three ad-hoc prompts: `setIssuanceCapability` in particular names the key that may
 * SIGN credentials, which is the last write on this page that should be one keystroke from sent.
 */
type CapabilityTarget =
  | { kind: "issuance"; providerId: string; service: string }
  | { kind: "verify"; purpose: string }
  | { kind: "resolver"; resolverKind: ResolverKind };

/**
 * The shared grant/withdraw dialog.
 *
 * Carrying the DIRECTION is not a nicety: every one of these three is a two-way lever on the
 * contract - `setIssuanceCapability(false)`, `setVerifierCapability(false)` and
 * `setResolverApproved(..., false)` are all real, and the panels above render a withdrawn entry
 * struck through. A control labelled "Grant / withdraw" that can only grant implies a state it gives
 * no way to reach.
 */
function CapabilityDialog({
  target,
  busy,
  onClose,
  onSubmit,
}: {
  target: CapabilityTarget | null;
  busy: boolean;
  onClose: () => void;
  onSubmit: (target: CapabilityTarget, address: string, allowed: boolean) => void;
}) {
  const [address, setAddress] = useState("");
  const [allowed, setAllowed] = useState(true);
  // Reset whenever the dialog is opened for a different target, so a value typed for one service
  // can never be carried into another.
  const key = target ? JSON.stringify(target) : "";
  const [openedFor, setOpenedFor] = useState("");
  if (key !== openedFor) {
    setOpenedFor(key);
    setAddress("");
    setAllowed(true);
  }
  if (!target) return null;

  const copy =
    target.kind === "issuance"
      ? {
          title: "Issue right",
          // NOT "on <clone>": the write names no service, and a dialog that said otherwise would
          // describe a narrower grant than the one about to be sent.
          what: "on this address, registry-wide",
          label: "Address to grant",
          hint:
            "The key that will SIGN issuances. This is the registrar's grant to make and nobody " +
            "else's: a service delegate carries content-write permissions and does not satisfy " +
            "canIssue, so a provider cannot grant their own signing key. The grant is on the " +
            "ADDRESS and names no service, so it reaches EVERY service in effective standing - " +
            "including other providers'.",
        }
      : target.kind === "verify"
        ? {
            title: "Verify capability",
            what: `for "${target.purpose}"`,
            label: "Relayer address",
            hint:
              "The relayer that may submit verifications for this purpose. This grants no issuance: " +
              "the verify axis is orthogonal, and an issuer is not implicitly a verifier.",
          }
        : {
            title: `${target.resolverKind === "directory" ? "Directory" : "Domain"} resolver`,
            what: "",
            label: "Resolver address",
            hint:
              target.resolverKind === "directory"
                ? "The deployed ProviderDirectory. Approving is the whole of the registrar's part - the provider must then SELECT it on their own portal."
                : "The deployed ServiceDomainResolver. Approving is the whole of the registrar's part - the provider must then SELECT it on their own portal.",
          };
  const verb = target.kind === "resolver" ? (allowed ? "Approve" : "Pull") : allowed ? "Grant" : "Withdraw";
  const ok = /^0x[0-9a-fA-F]{40}$/.test(address.trim());

  return (
    <Dialog open onOpenChange={(v) => (v ? null : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {copy.title} {copy.what}
          </DialogTitle>
          <DialogDescription>{copy.hint}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <Label htmlFor="cap-addr">{copy.label}</Label>
            <Input
              id="cap-addr"
              className="mt-1 font-mono text-xs"
              placeholder="0x…"
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
            {address.trim() !== "" && !ok ? (
              <p className="mt-1 text-xs text-destructive">
                Not a 0x-prefixed 20-byte address.
              </p>
            ) : null}
          </div>
          {/* Labelled, because the segmented choice and the footer button otherwise both read
              "Grant" with nothing saying which one is the action. */}
          <div>
            <Label>Direction</Label>
          <div className="mt-1 flex gap-2" data-testid="capability-direction">
            <Button
              type="button"
              size="sm"
              variant={allowed ? "secondary" : "outline"}
              onClick={() => setAllowed(true)}
            >
              {target.kind === "resolver" ? "Approve" : "Grant"}
            </Button>
            <Button
              type="button"
              size="sm"
              variant={!allowed ? "secondary" : "outline"}
              onClick={() => setAllowed(false)}
            >
              {target.kind === "resolver" ? "Pull" : "Withdraw"}
            </Button>
          </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            onClick={() => onSubmit(target, address.trim(), allowed)}
            disabled={!ok || busy}
            title={ok ? undefined : "Enter a 0x-prefixed 20-byte address first."}
            data-testid="capability-submit"
          >
            {busy ? <Spinner className="mr-2 h-4 w-4" /> : null}
            {verb}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function Providers() {
  const { central } = useApp();
  const { toast } = useToast();
  const [data, setData] = useState<ProvidersResp | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [dispatches, setDispatches] = useState<DispatchRecord[]>([]);
  // A monotonic key, so two dispatches for the same provider AND the same record type are still two
  // records. Deriving the key from the action alone would silently collapse a retry onto its
  // predecessor, which is the overwrite this log exists to stop.
  const dispatchSeq = useRef(0);
  const recordDispatch = useCallback((r: Omit<DispatchRecord, "key">) => {
    dispatchSeq.current += 1;
    const key = `${dispatchSeq.current}`;
    setDispatches((d) => [...d, { ...r, key }]);
  }, []);

  // ---- register dialog state -------------------------------------------------------------------
  const [open, setOpen] = useState(false);
  const [providerId, setProviderId] = useState("");
  const [controller, setController] = useState("");
  const [statement, setStatement] = useState<IdentityStatement>(EMPTY_IDENTITY_STATEMENT);
  const [checked, setChecked] = useState<CheckedRegistration | null>(null);
  const [checkedKey, setCheckedKey] = useState("");
  const [spent, setSpent] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const currentKey = useMemo(
    () => registrationKey(providerId, controller, statement),
    [providerId, controller, statement],
  );
  // `shown` and `fresh` answer DIFFERENT questions and must never be one value: a retired plan stays
  // VISIBLE (it is what the admin reviewed, and destroying it right after a send would take away the
  // one record of what they checked) while losing its authority to send.
  const shown = checked !== null;
  const fresh = shown && !spent && checkedKey === currentKey;
  const staleReason = !shown
    ? null
    : spent
      ? "This registration was submitted. Review again to send another."
      : checkedKey !== currentKey
        ? "An input changed after this was reviewed - review again before sending."
        : null;

  // ---- the rest of the journey ---------------------------------------------------------------
  const [expanded, setExpanded] = useState<string | null>(null);
  const [services, setServices] = useState<Record<string, ProviderServicesResp>>({});
  const [serviceError, setServiceError] = useState<Record<string, string>>({});
  const [verifiers, setVerifiers] = useState<VerifierCapabilitiesResp | null>(null);
  const [verifierError, setVerifierError] = useState<string | null>(null);
  const [resolvers, setResolvers] = useState<ResolversResp | null>(null);
  const [resolverError, setResolverError] = useState<string | null>(null);

  // ---- attach dialog: same check-then-send discipline as registration --------------------------
  const [attachFor, setAttachFor] = useState<string | null>(null);
  const [candidate, setCandidate] = useState("");
  const [preflight, setPreflight] = useState<AttachPreflightResp | null>(null);
  const [preflightKey, setPreflightKey] = useState("");
  const [attachSpent, setAttachSpent] = useState(false);
  const [capability, setCapability] = useState<CapabilityTarget | null>(null);

  const attachCurrentKey = useMemo(
    () => attachKey(attachFor ?? "", candidate),
    [attachFor, candidate],
  );
  // Same split as the registration dialog, and for the same reason: a retired plan stays VISIBLE
  // (it is the record of what was checked) while losing its authority to send.
  const attachShown = preflight !== null;
  const attachFresh = attachShown && !attachSpent && preflightKey === attachCurrentKey;
  const attachStaleReason = !attachShown
    ? null
    : attachSpent
      ? "This attach was submitted. Check again to send another."
      : preflightKey !== attachCurrentKey
        ? "The address changed after this was checked - check again before sending."
        : null;

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      setData(await central.listProviders());
    } catch (e) {
      // A failed read leaves `data` null and renders the reason. It must never fall through to an
      // empty list, which would read as "no providers exist".
      setData(null);
      setLoadError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [central]);

  const loadServices = useCallback(
    async (pid: string) => {
      setServiceError((m) => {
        const next = { ...m };
        delete next[pid];
        return next;
      });
      try {
        const resp = await central.listProviderServices(pid);
        setServices((m) => ({ ...m, [pid]: resp }));
      } catch (e) {
        // Could-not-read renders as itself. Falling through to an empty list would tell the admin
        // this provider has attached nothing - which is exactly what would make them attach a
        // duplicate.
        setServices((m) => {
          const next = { ...m };
          delete next[pid];
          return next;
        });
        setServiceError((m) => ({ ...m, [pid]: e instanceof Error ? e.message : String(e) }));
      }
    },
    [central],
  );

  const loadVerifiers = useCallback(async () => {
    setVerifierError(null);
    try {
      setVerifiers(await central.listVerifierCapabilities());
    } catch (e) {
      setVerifiers(null);
      setVerifierError(e instanceof Error ? e.message : String(e));
    }
  }, [central]);

  const loadResolvers = useCallback(async () => {
    setResolverError(null);
    try {
      setResolvers(await central.listResolvers());
    } catch (e) {
      setResolvers(null);
      setResolverError(e instanceof Error ? e.message : String(e));
    }
  }, [central]);

  useEffect(() => {
    void load();
    void loadVerifiers();
    void loadResolvers();
  }, [load, loadVerifiers, loadResolvers]);

  function resetDialog() {
    setProviderId("");
    setController("");
    setStatement(EMPTY_IDENTITY_STATEMENT);
    setChecked(null);
    setCheckedKey("");
    setSpent(false);
    setProblem(null);
  }

  function review() {
    const r = checkRegistration(providerId, controller, statement);
    if (!r.ok) {
      setProblem(r.problem);
      setChecked(null);
      return;
    }
    setProblem(null);
    setChecked(r.checked);
    setCheckedKey(currentKey);
    setSpent(false);
  }

  async function submitRegistration() {
    if (!checked || !fresh) return;
    setBusy("register");
    // Retire at SUBMISSION, before the outcome is known, so every terminal path inherits it and no
    // branch can forget to. A rejected send never reaches here, which is correct: nothing went out.
    setSpent(true);
    try {
      // Addresses the CHECKED values, never the live form.
      const resp = await central.registerProvider({
        providerId: checked.providerId,
        controller: checked.controller,
        identityDigest: checked.digest,
      });
      recordDispatch({
        providerId: checked.providerId,
        outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
        warning: resp.warning,
        actions: resp.actions,
        summary: "registration",
        // The dialog is about to close and the statement is stored nowhere else, so it travels into
        // the durable record. This is the success path honouring the same rule the review panel
        // states: what was reviewed stays readable after the send, because it IS the record of it.
        reviewed: { canonical: checked.canonical, digest: checked.digest },
      });
      if (resp.outcome === "executed") {
        toast({
          title: "Provider registered",
          description: "It is PENDING and can do nothing yet - set its standing to Active next.",
        });
      } else {
        toast({ title: "Nothing was broadcast", description: resp.warning ?? "", variant: "danger" });
      }
      setOpen(false);
      resetDialog();
      await load();
    } catch (e) {
      toast({
        title: "Registration failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  async function setStanding(pid: string, standing: "active" | "suspended" | "retired") {
    setBusy(`${pid}:standing`);
    try {
      const resp = await central.setProviderStanding(pid, { standing });
      recordDispatch({
        providerId: pid,
        outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
        warning: resp.warning,
        actions: resp.actions,
        summary: `standing → ${standing}`,
      });
      await load();
    } catch (e) {
      toast({
        title: "Standing change failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  async function setApproval(pid: string, recordType: string, allowed: boolean) {
    setBusy(`${pid}:${recordType}`);
    try {
      const resp = await central.setServiceCreationApproval(pid, { recordType, allowed });
      recordDispatch({
        providerId: pid,
        outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
        warning: resp.warning,
        actions: resp.actions,
        summary: `${allowed ? "approved" : "withdrew"} ${recordType}`,
      });
      await load();
    } catch (e) {
      toast({
        title: allowed ? "Approval failed" : "Withdrawal failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  function resetAttach() {
    setCandidate("");
    setPreflight(null);
    setPreflightKey("");
    setAttachSpent(false);
  }

  async function checkCandidate() {
    if (!attachFor) return;
    setBusy(`${attachFor}:preflight`);
    try {
      const resp = await central.preflightAttachService(attachFor, { serviceAddress: candidate });
      setPreflight(resp);
      setPreflightKey(attachCurrentKey);
      setAttachSpent(false);
    } catch (e) {
      setPreflight(null);
      toast({
        title: "Check failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  async function submitAttach() {
    if (!attachFor || !preflight || !attachFresh) return;
    const generationId = resolvedGenerationId(preflight);
    const expectedOwner = resolvedOwner(preflight);
    // Both are REQUIRED: the calldata cannot be built without them, and inventing either would send
    // a transaction addressing something nobody checked.
    if (!generationId || !expectedOwner) return;
    setBusy(`${attachFor}:attach`);
    // Retire at SUBMISSION, before the outcome is known, so every terminal path inherits it.
    setAttachSpent(true);
    try {
      // Addresses the CHECKED values, never the live form.
      const resp = await central.attachService(attachFor, {
        serviceAddress: preflight.serviceAddress,
        generationId,
        expectedOwner,
      });
      recordDispatch({
        providerId: attachFor,
        outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
        warning: resp.warning,
        actions: resp.actions,
        summary: `attached ${shortAddr(preflight.serviceAddress)}`,
      });
      if (resp.outcome === "executed") {
        toast({
          title: "Service attached",
          description:
            "It is PENDING and can issue nothing yet - set its standing to Active, then grant issuance capability.",
        });
      } else {
        toast({ title: "Nothing was broadcast", description: resp.warning ?? "", variant: "danger" });
      }
      setAttachFor(null);
      resetAttach();
      await loadServices(attachFor);
    } catch (e) {
      toast({
        title: "Attach failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  async function setServiceStanding(pid: string, service: string, standing: "active" | "suspended") {
    setBusy(`${service}:standing`);
    try {
      const resp = await central.setServiceStanding(service, { standing });
      recordDispatch({
        providerId: pid,
        outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
        warning: resp.warning,
        actions: resp.actions,
        summary: `service ${shortAddr(service)} standing → ${standing}`,
      });
      await loadServices(pid);
    } catch (e) {
      toast({
        title: "Service standing change failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  /**
   * The three capability writes, all through the reviewed dialog.
   *
   * They used to read their address from `window.prompt` and send it straight through - no review,
   * no direction, and unstubbable in a jsdom test, which is why they had no UI coverage at all. That
   * was worst on `setIssuanceCapability`, the one write that names a key allowed to sign credentials.
   */
  async function submitCapability(
    target: CapabilityTarget,
    address: string,
    allowed: boolean,
  ) {
    setBusy("capability");
    try {
      if (target.kind === "issuance") {
        const resp = await central.setIssueRight(address, { allowed });
        recordDispatch({
          providerId: target.providerId,
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          // Deliberately does NOT name a service. The grant carries none, so a summary reading
          // "granted on <clone>" would describe a narrower write than the one just sent.
          summary: `issue right ${allowed ? "granted to" : "withdrawn from"} ${shortAddr(address)} (every service in standing)`,
        });
        await loadServices(target.providerId);
      } else if (target.kind === "verify") {
        const resp = await central.setVerifierCapability({
          purpose: target.purpose,
          relayer: address,
          allowed,
        });
        recordDispatch({
          providerId: `verify:${target.purpose}`,
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          summary: `verify:${target.purpose} ${allowed ? "granted to" : "withdrawn from"} ${shortAddr(address)}`,
        });
        await loadVerifiers();
      } else {
        const resp = await central.setResolverApproved({
          kind: target.resolverKind,
          resolver: address,
          approved: allowed,
        });
        recordDispatch({
          providerId: `${target.resolverKind} resolver`,
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          summary: `${target.resolverKind} resolver ${allowed ? "approved" : "pulled"}: ${shortAddr(address)}`,
        });
        await loadResolvers();
      }
      setCapability(null);
    } catch (e) {
      toast({
        title: "Capability change failed",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(null);
    }
  }

  const authority = data?.authority;

  return (
    <div className="space-y-6">
      {/* Which path a write will take, stated BEFORE the admin fills a form in. `heldByHosted` is
          tri-state: null means we could not establish it, which is not the same as "no". */}
      {authority ? (
        <div
          className={`rounded-md border p-3 text-sm ${
            authority.heldByHosted === true
              ? "border-emerald-500/40 bg-emerald-500/10"
              : authority.heldByHosted === false
                ? "border-amber-500/40 bg-amber-500/10"
                : "border-muted bg-muted/40"
          }`}
          data-testid="registrar-authority"
        >
          {authority.heldByHosted === true ? (
            <>The hosted admin key holds this registry - registrar actions execute directly.</>
          ) : authority.heldByHosted === false ? (
            <>
              The hosted admin key does NOT own this registry, so actions come back as unsigned
              calldata for {shortAddr(authority.owner ?? "the owner")} to execute out of band.
            </>
          ) : (
            <>Could not establish who owns this registry - an action may execute or may be proposed.</>
          )}
        </div>
      ) : null}

      {/* ABOVE the table on purpose: an action that broadcast nothing is a call to action, and the
          table is exactly what a failed reload replaces. */}
      <DispatchLog records={dispatches} />

      <Card>
        <CardHeader className="flex-row items-start justify-between gap-4 space-y-0">
          <div>
            <CardTitle>Providers</CardTitle>
            <CardDescription>
              Registering a provider is a KYC assertion, not a formality: you are stating that you
              have verified this entity. A <span className="font-medium">provider id is permanent</span>{" "}
              and can never be reassigned.
            </CardDescription>
          </div>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
              <RefreshCw className="mr-2 h-4 w-4" />
              Refresh
            </Button>
            <Button
              size="sm"
              onClick={() => {
                resetDialog();
                setProviderId(generateProviderId());
                setOpen(true);
              }}
            >
              <UserPlus className="mr-2 h-4 w-4" />
              Register provider
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {loading ? (
            <Spinner />
          ) : loadError ? (
            // Could-not-read renders as itself, never as an empty registry.
            <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                <span className="font-medium">The provider registry could not be read.</span>{" "}
                <span className="text-muted-foreground">{loadError}</span>
              </span>
            </div>
          ) : data && data.providers.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No providers are registered on {shortAddr(data.registry)} yet.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Provider id</TableHead>
                  <TableHead>Standing</TableHead>
                  <TableHead>Controller</TableHead>
                  <TableHead>Approved to create</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data?.providers.map((p: ProviderRegistrarView) => {
                  const pid = p.provider.providerId;
                  const approved = (rt: string) =>
                    p.approvals.state === "resolved" &&
                    p.approvals.entries.some((e) => e.recordType === rt && e.allowed);
                  const open = expanded === pid;
                  return (
                    <Fragment key={pid}>
                    <TableRow>
                      <TableCell className="align-top">
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            className="text-muted-foreground hover:text-foreground"
                            aria-label={open ? "Hide services" : "Show services"}
                            data-testid="toggle-services"
                            onClick={() => {
                              const next = open ? null : pid;
                              setExpanded(next);
                              if (next) void loadServices(next);
                            }}
                          >
                            {open ? (
                              <ChevronDown className="h-4 w-4" />
                            ) : (
                              <ChevronRight className="h-4 w-4" />
                            )}
                          </button>
                          <span className="font-mono text-xs">{shortAddr(pid)}</span>
                          {/* Permanent, opaque, and truncated on screen - so it must stay
                              recoverable without hovering. */}
                          <CopyButton value={pid} label="provider id" />
                        </div>
                        <IdentityAnchor anchor={p.identityAnchor} />
                      </TableCell>
                      <TableCell className="align-top">
                        <Badge variant={STANDING_TONE[p.provider.standing]}>
                          {p.provider.standing}
                        </Badge>
                        <p className="mt-1 max-w-[16rem] text-xs text-muted-foreground">
                          {STANDING_MEANING[p.provider.standing]}
                        </p>
                      </TableCell>
                      <TableCell className="align-top">
                        <AddressRef address={p.provider.controller} />
                      </TableCell>
                      <TableCell className="align-top">
                        <Approvals approvals={p.approvals} />
                      </TableCell>
                      <TableCell className="align-top text-right">
                        <div className="flex flex-col items-end gap-2">
                          <div className="flex gap-1">
                            {p.provider.standing !== "active" && p.provider.standing !== "retired" ? (
                              <Button
                                size="sm"
                                disabled={busy !== null}
                                onClick={() => void setStanding(pid, "active")}
                              >
                                <BadgeCheck className="mr-1 h-3.5 w-3.5" />
                                Activate
                              </Button>
                            ) : null}
                            {p.provider.standing === "active" ? (
                              <Button
                                size="sm"
                                variant="outline"
                                disabled={busy !== null}
                                onClick={() => void setStanding(pid, "suspended")}
                              >
                                Suspend
                              </Button>
                            ) : null}
                          </div>
                          <div className="flex flex-wrap justify-end gap-1">
                            {RECORD_TYPES.map((rt) => {
                              const on = approved(rt);
                              // An UNAVAILABLE approvals read disables the toggle: we do not know
                              // the current bit, and the contract refuses a redundant write. That is
                              // could-not-check declining to guess, not a refusal of the action.
                              const unknown = p.approvals.state !== "resolved";
                              return (
                                <Button
                                  key={rt}
                                  size="sm"
                                  variant={on ? "secondary" : "outline"}
                                  disabled={busy !== null || unknown}
                                  title={
                                    unknown
                                      ? "The approval log could not be read, so the current state is unknown."
                                      : undefined
                                  }
                                  onClick={() => void setApproval(pid, rt, !on)}
                                >
                                  {on ? (
                                    <ShieldOff className="mr-1 h-3.5 w-3.5" />
                                  ) : (
                                    <ShieldCheck className="mr-1 h-3.5 w-3.5" />
                                  )}
                                  {on ? `Withdraw ${rt}` : rt}
                                </Button>
                              );
                            })}
                          </div>
                        </div>
                      </TableCell>
                    </TableRow>
                    {open ? (
                      <TableRow>
                        <TableCell colSpan={5} className="bg-muted/30">
                          <div className="space-y-2">
                            <div className="flex flex-wrap items-center justify-between gap-2">
                              <span className="text-sm font-medium">Attached services</span>
                              <Button
                                size="sm"
                                disabled={busy !== null}
                                onClick={() => {
                                  resetAttach();
                                  setAttachFor(pid);
                                }}
                              >
                                <Link2 className="mr-1 h-3.5 w-3.5" />
                                Attach a contract
                              </Button>
                            </div>
                            {serviceError[pid] ? (
                              // Could-not-read renders as itself. An empty list here would read as
                              // "this provider has attached nothing", which is what would make an
                              // admin attach a duplicate.
                              <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
                                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                                <span>
                                  <span className="font-medium">
                                    This provider&apos;s services could not be read.
                                  </span>{" "}
                                  <span className="text-muted-foreground">{serviceError[pid]}</span>
                                </span>
                              </div>
                            ) : !services[pid] ? (
                              <Spinner />
                            ) : services[pid].services.length === 0 ? (
                              <p className="text-sm text-muted-foreground">
                                Nothing attached yet. A provider deploys their own contract from their
                                portal, then sends you the address to attach here - until you do,{" "}
                                <span className="font-medium">
                                  they cannot select it or do anything else with it
                                </span>
                                .
                              </p>
                            ) : (
                              <div className="space-y-2">
                                {services[pid].services.map((v) => (
                                  <ServiceRow
                                    key={v.service.serviceAddress}
                                    view={v}
                                    busy={busy !== null}
                                    onStanding={(svc, standing) =>
                                      void setServiceStanding(pid, svc, standing)
                                    }
                                    onGrant={(svc) => setCapability({ kind: "issuance", providerId: pid, service: svc })}
                                  />
                                ))}
                              </div>
                            )}
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : null}
                    </Fragment>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* The two ORTHOGONAL levers, deliberately outside the provider table: neither is keyed by a
          provider. `setVerifierCapability` takes a purpose and a relayer; `setResolverApproved` is
          fleet-wide. Rendering either inside a provider row would misstate what it applies to. */}
      <VerifierCapabilities
        data={verifiers}
        loadError={verifierError}
        busy={busy !== null}
        onSet={(p) => setCapability({ kind: "verify", purpose: p })}
        onRefresh={() => void loadVerifiers()}
      />
      <Resolvers
        data={resolvers}
        loadError={resolverError}
        busy={busy !== null}
        onSet={(k) => setCapability({ kind: "resolver", resolverKind: k })}
        onRefresh={() => void loadResolvers()}
      />

      <CapabilityDialog
        target={capability}
        busy={busy !== null}
        onClose={() => setCapability(null)}
        onSubmit={(t, addr, allowed) => void submitCapability(t, addr, allowed)}
      />

      <Dialog
        open={attachFor !== null}
        onOpenChange={(v) => (v ? null : (setAttachFor(null), resetAttach()))}
      >
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Attach a contract</DialogTitle>
            <DialogDescription>
              Binding a provider-deployed contract to their record. Until this happens the provider
              cannot select it, claim a domain for it, or issue from it -{" "}
              <code>repointService</code> refuses an address that was never attached.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div>
              <Label htmlFor="svc">Contract address</Label>
              <Input
                id="svc"
                className="mt-1 font-mono text-xs"
                placeholder="0x…"
                value={candidate}
                onChange={(e) => setCandidate(e.target.value)}
              />
              <p className="mt-1 text-xs text-muted-foreground">
                The address the provider deployed and sent you. Nothing else is typed: the record type,
                the factory generation and the owner are all read off the contract itself.
              </p>
            </div>

            {/* A retired plan stays VISIBLE with its verdict struck through - it is the record of
                what was checked - while losing its authority to send. */}
            {attachShown && preflight ? (
              <div
                className={`rounded-md border p-3 text-sm ${
                  preflight.verdict === "ready" && attachFresh
                    ? "border-emerald-500/40 bg-emerald-500/10"
                    : preflight.verdict === "refused"
                      ? "border-destructive/40 bg-destructive/10"
                      : "border-amber-500/40 bg-amber-500/10"
                }`}
                data-testid="attach-preflight"
              >
                <div className="flex flex-wrap items-center gap-2 font-medium">
                  <span className={attachFresh ? "" : "line-through opacity-70"}>
                    {preflight.verdict === "ready"
                      ? "Checked - ready to attach"
                      : preflight.verdict === "refused"
                        ? "The chain would refuse this"
                        : "Could not be established"}
                  </span>
                  {!attachFresh ? (
                    <span className="text-amber-700 dark:text-amber-500">
                      Superseded - {attachStaleReason}
                    </span>
                  ) : null}
                </div>
                {preflight.reason ? (
                  <p className="mt-1 text-muted-foreground">{preflight.reason}</p>
                ) : null}

                <dl className="mt-2 space-y-1 text-xs">
                  <div className="flex gap-2">
                    <dt className="w-32 shrink-0 text-muted-foreground">factory generation</dt>
                    <dd>
                      {preflight.generation.state === "resolved" ? (
                        <span className="font-mono">
                          {shortAddr(preflight.generation.generationId)}
                        </span>
                      ) : (
                        <span className="text-amber-700 dark:text-amber-500">
                          {preflight.generation.reason}
                        </span>
                      )}
                    </dd>
                  </div>
                  <div className="flex gap-2">
                    <dt className="w-32 shrink-0 text-muted-foreground">record type</dt>
                    <dd>
                      {preflight.metadata.state === "resolved" ? (
                        <>
                          {preflight.metadata.recordType ??
                            shortAddr(preflight.metadata.recordTypeKey)}
                        </>
                      ) : (
                        <span className="text-amber-700 dark:text-amber-500">
                          {preflight.metadata.reason}
                        </span>
                      )}
                    </dd>
                  </div>
                  <div className="flex gap-2">
                    <dt className="w-32 shrink-0 text-muted-foreground">owner</dt>
                    <dd className="flex items-center gap-1">
                      {preflight.metadata.state === "resolved" ? (
                        <>
                          <span className="font-mono">{shortAddr(preflight.metadata.owner)}</span>
                          <CopyButton value={preflight.metadata.owner} label="owner" />
                          <span className="text-muted-foreground">
                            — sent as the expected owner; a mismatch at send time refuses the
                            transaction rather than attaching to the wrong key.
                          </span>
                        </>
                      ) : (
                        <span className="text-amber-700 dark:text-amber-500">not established</span>
                      )}
                    </dd>
                  </div>
                </dl>
              </div>
            ) : null}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => void checkCandidate()} disabled={busy !== null}>
              <Plus className="mr-2 h-4 w-4" />
              Check
            </Button>
            <Button
              onClick={() => void submitAttach()}
              disabled={!attachFresh || !attachSendable(preflight) || busy !== null}
              title={
                !attachShown
                  ? "Check the address first."
                  : !attachFresh
                    ? (attachStaleReason ?? undefined)
                    : !attachSendable(preflight)
                      ? "The factory generation and owner could not both be established, so there is nothing to address a transaction to. Check again."
                      : undefined
              }
            >
              {busy?.endsWith(":attach") ? <Spinner className="mr-2 h-4 w-4" /> : null}
              Attach
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={open} onOpenChange={(v) => (v ? setOpen(true) : (setOpen(false), resetDialog()))}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Register a provider</DialogTitle>
            <DialogDescription>
              You are asserting that you have completed KYC on this entity. Registration is permanent:
              the provider id can never be reassigned, and the identity anchor&apos;s revision only
              ever moves forward.
            </DialogDescription>
          </DialogHeader>

          {env.demoMode && (
            <div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  // A fresh id every time: a fixed one collides with AlreadyRegistered the second
                  // time this demo is walked.
                  setProviderId(generateProviderId());
                  setController(DEMO_PROVIDER_REGISTRATION.controller);
                  setStatement({
                    legalName: DEMO_PROVIDER_REGISTRATION.legalName,
                    jurisdiction: DEMO_PROVIDER_REGISTRATION.jurisdiction,
                    registrationNumber: DEMO_PROVIDER_REGISTRATION.registrationNumber,
                    verifiedOn: DEMO_PROVIDER_REGISTRATION.verifiedOn,
                    notes: DEMO_PROVIDER_REGISTRATION.notes,
                  });
                }}
              >
                <Sparkles className="h-4 w-4" /> Fill demo data
              </Button>
            </div>
          )}

          <div className="space-y-4">
            <div>
              <Label htmlFor="pid">Provider id</Label>
              <div className="mt-1 flex items-center gap-2">
                <Input
                  id="pid"
                  className="font-mono text-xs"
                  value={providerId}
                  onChange={(e) => setProviderId(e.target.value)}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setProviderId(generateProviderId())}
                >
                  <Dices className="mr-1 h-3.5 w-3.5" />
                  New
                </Button>
                <CopyButton value={providerId} label="provider id" />
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                Arbitrary and permanent. It is generated at random and deliberately means nothing -
                it must not be derived from the provider&apos;s name, domain, address or keys. Copy it
                now: the provider needs it to use the self-service portal, and it cannot be changed
                later.
              </p>
            </div>

            <div>
              <Label htmlFor="ctrl">Controller address</Label>
              <Input
                id="ctrl"
                className="mt-1 font-mono text-xs"
                placeholder="0x…"
                value={controller}
                onChange={(e) => setController(e.target.value)}
              />
              <p className="mt-1 text-xs text-muted-foreground">
                The wallet that will act AS this provider on the self-service portal - the
                provider&apos;s own key, not yours. If this is wrong, the provider cannot do anything
                and correcting it needs a separate controller-transfer.
              </p>
            </div>

            <fieldset className="rounded-md border p-3">
              <legend className="px-1 text-sm font-medium">Identity statement</legend>
              <p className="mb-3 text-xs text-muted-foreground">
                What you are asserting. Only its keccak256 digest is written on chain - the text below
                is never sent to the backend and is not stored anywhere, so keep your own copy. After
                you send, it stays readable under &ldquo;Registrar actions this session&rdquo; on this
                page, but only until you leave it.
              </p>
              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <Label htmlFor="legalName">Legal entity name</Label>
                  <Input
                    id="legalName"
                    className="mt-1"
                    value={statement.legalName}
                    onChange={(e) => setStatement({ ...statement, legalName: e.target.value })}
                  />
                </div>
                <div>
                  <Label htmlFor="jurisdiction">Jurisdiction</Label>
                  <Input
                    id="jurisdiction"
                    className="mt-1"
                    value={statement.jurisdiction}
                    onChange={(e) => setStatement({ ...statement, jurisdiction: e.target.value })}
                  />
                </div>
                <div>
                  <Label htmlFor="regNo">Registration number</Label>
                  <Input
                    id="regNo"
                    className="mt-1"
                    value={statement.registrationNumber}
                    onChange={(e) =>
                      setStatement({ ...statement, registrationNumber: e.target.value })
                    }
                  />
                </div>
                <div>
                  <Label htmlFor="verifiedOn">Verified on (YYYY-MM-DD)</Label>
                  <Input
                    id="verifiedOn"
                    className="mt-1"
                    placeholder="2026-08-02"
                    value={statement.verifiedOn}
                    onChange={(e) => setStatement({ ...statement, verifiedOn: e.target.value })}
                  />
                </div>
                <div className="sm:col-span-2">
                  <Label htmlFor="notes">What you checked</Label>
                  <Input
                    id="notes"
                    className="mt-1"
                    value={statement.notes}
                    onChange={(e) => setStatement({ ...statement, notes: e.target.value })}
                  />
                </div>
              </div>
            </fieldset>

            {problem ? (
              <div className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm">
                {problem}
              </div>
            ) : null}

            {/* A retired plan stays VISIBLE with its verdict struck through, rather than vanishing:
                it is the record of what was reviewed. The qualifier is attached to the verdict, not
                only to a banner, because a reader who scans to the digest and stops is exactly the
                reader a separate banner misses. */}
            {shown && checked ? (
              <div
                className={`rounded-md border p-3 ${
                  fresh ? "border-emerald-500/40 bg-emerald-500/10" : "border-amber-500/40 bg-amber-500/10"
                }`}
                data-testid="registration-review"
              >
                <div className="flex items-center gap-2 text-sm font-medium">
                  <span className={fresh ? "" : "line-through opacity-70"}>Reviewed - ready to send</span>
                  {!fresh ? (
                    <span className="text-amber-700 dark:text-amber-500">Superseded - {staleReason}</span>
                  ) : null}
                </div>
                <pre className="mt-2 whitespace-pre-wrap rounded bg-background/60 p-2 text-xs">
                  {checked.canonical}
                </pre>
                <div className="mt-2 flex items-center gap-1 text-xs">
                  <span className="text-muted-foreground">digest</span>
                  <span className="font-mono break-all">{checked.digest}</span>
                  <CopyButton value={checked.digest} label="identity digest" />
                </div>
                <div className="mt-1 flex items-center gap-1 text-xs">
                  <span className="text-muted-foreground">statement</span>
                  <CopyButton value={canonicalIdentityStatement(checked.statement)} label="statement" />
                </div>
              </div>
            ) : null}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={review} disabled={busy !== null}>
              <Plus className="mr-2 h-4 w-4" />
              Review
            </Button>
            <Button onClick={() => void submitRegistration()} disabled={!fresh || busy !== null}>
              {busy === "register" ? <Spinner className="mr-2 h-4 w-4" /> : null}
              Register
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
