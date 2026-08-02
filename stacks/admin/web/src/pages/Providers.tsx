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
 *    reverted status, so `outcome: "executed"` genuinely means mined-and-succeeded — and the two
 *    "nothing was broadcast" outcomes stay apart, because a designed proposal and a wrong hosted key
 *    have different remedies.
 *  - Three states, never two. An approval log that could not be READ is rendered as its own state
 *    with its reason, never as "approved for nothing".
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
  type GovernanceDisposition,
  type ProviderApprovalsRead,
  type ProviderRegistrarView,
  type ProviderStanding,
  type ProvidersResp,
  type WhitelistOutcome,
} from "@dogtag/ui";
import {
  AlertTriangle,
  BadgeCheck,
  Dices,
  HelpCircle,
  Plus,
  RefreshCw,
  ShieldCheck,
  ShieldOff,
  UserPlus,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
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

/** The record types a provider can be pre-authorized to create a service for. */
const RECORD_TYPES = ["DOG_PROFILE", "VACCINATION", "GROOMING", "BOARDING"] as const;

const STANDING_TONE: Record<ProviderStanding, "neutral" | "warning" | "success" | "danger" | "outline"> =
  {
    none: "outline",
    // PENDING is deliberately WARNING rather than neutral: it is what registration produces, and a
    // provider left there can do nothing at all. Painting it neutral is what makes it easy to miss.
    pending: "warning",
    active: "success",
    suspended: "danger",
    retired: "danger",
  };

/**
 * What a standing MEANS for the provider's ability to act, in the registrar's terms.
 *
 * `pending` is the one worth spelling out: it is what registration produces, it looks benign, and it
 * is indistinguishable from `active` on a badge alone — while `canWriteProvider` refuses everything
 * but `active`, so a provider left here can do nothing and the self-service page will say so.
 */
const STANDING_MEANING: Record<ProviderStanding, string> = {
  none: "Not registered.",
  pending: "Registered but INERT — every self-service action refuses until this is Active.",
  active: "Cleared to act.",
  suspended: "Frozen. Reversible.",
  retired: "Terminal — no further standing change is possible.",
};

/** The outcome of a send, reported from the receipt rather than from the submission. */
interface SendResult {
  outcome: WhitelistOutcome;
  warning?: string | null;
  actions: GovernanceDisposition[];
  summary: string;
}

function OutcomeNote({ result }: { result: SendResult }) {
  const tone =
    result.outcome === "executed"
      ? "border-emerald-500/40 bg-emerald-500/10"
      : result.outcome === "proposed_by_design"
        ? "border-amber-500/40 bg-amber-500/10"
        : "border-destructive/40 bg-destructive/10";
  return (
    <div className={`mt-3 rounded-md border p-3 text-sm ${tone}`}>
      <div className="font-medium">
        {result.outcome === "executed"
          ? `Mined: ${result.summary}`
          : "Nothing was broadcast — on-chain state is unchanged."}
      </div>
      {result.warning ? <p className="mt-1 text-muted-foreground">{result.warning}</p> : null}
      <div className="mt-2 space-y-1">
        {result.actions.map((a, i) => (
          <div key={i} className="text-xs">
            {a.disposition === "executed" ? (
              <TxRef event={{ txHash: a.txHash }} href={txExplorerHref({ txHash: a.txHash })} />
            ) : (
              <span className="font-mono break-all">
                → {shortAddr(a.holder ?? "")} : {a.calldata?.slice(0, 26)}…
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * The approvals cell — three states, and the third is NOT a neighbour of the other two.
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

export function Providers() {
  const { central } = useApp();
  const { toast } = useToast();
  const [data, setData] = useState<ProvidersResp | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, SendResult>>({});

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
        ? "An input changed after this was reviewed — review again before sending."
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

  useEffect(() => {
    void load();
  }, [load]);

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
      setResults((r) => ({
        ...r,
        [checked.providerId]: {
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          summary: `registered ${shortAddr(checked.providerId)}`,
        },
      }));
      if (resp.outcome === "executed") {
        toast({
          title: "Provider registered",
          description: "It is PENDING and can do nothing yet — set its standing to Active next.",
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
      setResults((r) => ({
        ...r,
        [pid]: {
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          summary: `standing → ${standing}`,
        },
      }));
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
      setResults((r) => ({
        ...r,
        [pid]: {
          outcome: resp.outcome ?? (resp.executed ? "executed" : "proposed_unauthorized"),
          warning: resp.warning,
          actions: resp.actions,
          summary: `${allowed ? "approved" : "withdrew"} ${recordType}`,
        },
      }));
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
            <>The hosted admin key holds this registry — registrar actions execute directly.</>
          ) : authority.heldByHosted === false ? (
            <>
              The hosted admin key does NOT own this registry, so actions come back as unsigned
              calldata for {shortAddr(authority.owner ?? "the owner")} to execute out of band.
            </>
          ) : (
            <>Could not establish who owns this registry — an action may execute or may be proposed.</>
          )}
        </div>
      ) : null}

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
                  const result = results[pid];
                  const approved = (rt: string) =>
                    p.approvals.state === "resolved" &&
                    p.approvals.entries.some((e) => e.recordType === rt && e.allowed);
                  return (
                    <TableRow key={pid}>
                      <TableCell className="align-top">
                        <div className="flex items-center gap-1">
                          <span className="font-mono text-xs">{shortAddr(pid)}</span>
                          {/* Permanent, opaque, and truncated on screen — so it must stay
                              recoverable without hovering. */}
                          <CopyButton value={pid} label="provider id" />
                        </div>
                        {result ? <OutcomeNote result={result} /> : null}
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
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

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
                Arbitrary and permanent. It is generated at random and deliberately means nothing —
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
                The wallet that will act AS this provider on the self-service portal — the
                provider&apos;s own key, not yours. If this is wrong, the provider cannot do anything
                and correcting it needs a separate controller-transfer.
              </p>
            </div>

            <fieldset className="rounded-md border p-3">
              <legend className="px-1 text-sm font-medium">Identity statement</legend>
              <p className="mb-3 text-xs text-muted-foreground">
                What you are asserting. Only its keccak256 digest is written on chain — the text below
                is never sent to the backend and is not stored anywhere, so keep your own copy.
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
                  <span className={fresh ? "" : "line-through opacity-70"}>Reviewed — ready to send</span>
                  {!fresh ? (
                    <span className="text-amber-700 dark:text-amber-500">Superseded — {staleReason}</span>
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
