import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerAddressUrl,
  useToast,
  type ApiError,
  type AuthoritySlot,
  type GovernanceAuthorityResp,
} from "@dogtag/ui";
import {
  ArrowRightLeft,
  CheckCircle2,
  Clock,
  Copy,
  ExternalLink,
  Gavel,
  History,
  Hourglass,
  Info,
  KeyRound,
  Landmark,
  Send,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useApp } from "../app/AppContext";
import { absoluteTime } from "../lib/activity";
import { shortAddr } from "../lib/format";
import {
  countdown,
  FORMER_DEPLOYER_EOA,
  GOVERNANCE_SIGNER_1,
  GOVERNANCE_SIGNER_1_LABEL,
  isFormerDeployer,
  isGovernanceSigner1,
  TIMELOCK_MODELS,
} from "../lib/governance";

/**
 * Governance console (plan §3.6, PR-F) — the read-only home for the signer-1 authority.
 *
 * Surfaces the LIVE on-chain governance state from `GET /v1/admin/governance/authority` (PR-A):
 * who holds the factory `Ownable2Step` ownership, the IssuerRegistry `WHITELIST_ADMIN`, and the
 * IssuerRegistry `DEFAULT_ADMIN` (ACDAR two-step + 3-day timelock), whether the hosted control-plane
 * key holds each (→ writes execute directly) or not (→ writes downgrade to `GovernanceAction`
 * proposals), and any pending two-step admin transfer with its live timelock countdown. The page is
 * READ-ONLY: it never assumes the former deployer EOA and it never broadcasts — actual transfers
 * are captain-initiated through the ACDAR two-step flow, documented here but not wired as live buttons.
 * Non-PII: chain + admin authority directory only.
 */
export function Governance() {
  const { central } = useApp();
  const { toast } = useToast();
  const [authority, setAuthority] = useState<GovernanceAuthorityResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const a = await central.governanceAuthority();
        if (!cancelled) setAuthority(a);
      } catch (err) {
        const e = err as ApiError;
        if (!cancelled) {
          setError(e.message);
          toast({ title: "Failed to read authority map", description: e.message, variant: "danger" });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [central, toast]);

  // Tick the clock so pending-transfer timelock countdowns stay live (days-scale → 30s is ample).
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, []);

  if (authority === null) {
    return (
      <Card>
        <CardContent className="flex flex-col items-center gap-3 py-16 text-sm text-muted">
          {error ? (
            <>
              <TriangleAlert className="h-6 w-6 text-danger" />
              <p>Could not read the on-chain authority map.</p>
              <p className="font-mono text-xs">{error}</p>
            </>
          ) : (
            <>
              <Spinner className="h-6 w-6" />
              Reading the on-chain authority map…
            </>
          )}
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      <AuthorityHeader authority={authority} />
      <RoleHoldersCard authority={authority} />
      <PendingTransfersCard authority={authority} now={now} />
      <GovernanceActionModelCard authority={authority} />
      <TimelockModelCard />
      <Phase2RunbookCard />
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------
// Authority header — signer-1 as the post-Phase-2 protocol authority + the control-plane signer status.
// ---------------------------------------------------------------------------------------------------

function AuthorityHeader({ authority }: { authority: GovernanceAuthorityResp }) {
  const hosted = authority.hostedSigner;
  const hostedIsSigner1 = isGovernanceSigner1(hosted);
  const hostedIsFormerDeployer = isFormerDeployer(hosted);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Landmark className="h-5 w-5 text-primary" /> Protocol governance
        </CardTitle>
        <CardDescription>
          Post-Phase-2, all governance/admin authority sits with the governance signer{" "}
          <span className="font-medium text-onSurface">{GOVERNANCE_SIGNER_1_LABEL}</span>. This
          console reads the live authority map directly from ROAX
          {authority.chainId ? ` (chain ${authority.chainId})` : ""}.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* the governance authority itself */}
        <div className="rounded-lg border border-primary/30 bg-primary/5 p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <div className="flex items-center gap-2 text-sm font-semibold text-onSurface">
                <ShieldCheck className="h-4 w-4 text-primary" />
                Governance authority — {GOVERNANCE_SIGNER_1_LABEL}
              </div>
              <AddrLink addr={GOVERNANCE_SIGNER_1} withCopy />
              <p className="text-xs text-muted">
                Holds factory ownership, <code>WHITELIST_ADMIN</code>, and the{" "}
                <code>DEFAULT_ADMIN</code> of the IssuerRegistry, VerificationRegistry and DogTagSBT
                (each behind its ACDAR two-step timelock).
              </p>
            </div>
            <Badge variant="success">authority</Badge>
          </div>
        </div>

        {/* the hosted control-plane signer, and what its identity implies for writes */}
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="rounded-md border border-border bg-surface-muted px-3 py-3">
            <div className="text-xs font-semibold uppercase tracking-wide text-muted">
              Control-plane signer
            </div>
            <div className="mt-1">
              {hosted ? <AddrLink addr={hosted} withCopy /> : <span className="text-sm text-muted">unavailable</span>}
            </div>
            <div className="mt-2">
              {hostedIsSigner1 ? (
                <Badge variant="success">
                  <CheckCircle2 className="h-3 w-3" /> is signer-1 → privileged writes execute directly
                </Badge>
              ) : hostedIsFormerDeployer ? (
                <Badge variant="danger">
                  <TriangleAlert className="h-3 w-3" /> former deployer EOA → writes downgrade to proposals
                </Badge>
              ) : hosted ? (
                <Badge variant="warning">not signer-1 → writes route as proposals</Badge>
              ) : (
                <Badge variant="neutral">unknown</Badge>
              )}
            </div>
          </div>

          <div className="rounded-md border border-border bg-surface-muted px-3 py-3">
            <div className="text-xs font-semibold uppercase tracking-wide text-muted">
              Former deployer EOA
            </div>
            <div className="mt-1">
              <AddrLink addr={FORMER_DEPLOYER_EOA} />
            </div>
            <div className="mt-2">
              <Badge variant="neutral">no governance authority - not role-free</Badge>
            </div>
            <p className="mt-2 text-xs text-muted">
              Phase-2 removed its factory ownership, WHITELIST_ADMIN and DEFAULT_ADMIN. It still holds
              the retired owner-revealing SBT's ISSUER_ROLE and historical record-type whitelists, so
              never reuse it as a neutral key.
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------------------------------
// Role holders — the live authority map across the governed contracts.
// ---------------------------------------------------------------------------------------------------

interface RoleRow {
  label: string;
  slot: AuthoritySlot;
  mechanism: string;
  /** the single-holder accessor value (owner/holder), or null for ordinary AccessControl roles. */
  holder: string | null;
}

function roleRows(authority: GovernanceAuthorityResp): RoleRow[] {
  return [
    {
      label: "Factory owner",
      slot: authority.factoryOwner,
      mechanism: "Ownable2Step",
      holder: authority.factoryOwner.owner ?? null,
    },
    {
      label: "Whitelist admin",
      slot: authority.whitelistAdmin,
      mechanism: "WHITELIST_ADMIN role",
      holder: authority.whitelistAdmin.holder ?? authority.whitelistAdmin.owner ?? null,
    },
    {
      label: "Default admin",
      slot: authority.defaultAdmin,
      mechanism: "DEFAULT_ADMIN_ROLE · ACDAR 3-day",
      holder: authority.defaultAdmin.holder ?? null,
    },
  ];
}

function RoleHoldersCard({ authority }: { authority: GovernanceAuthorityResp }) {
  const rows = useMemo(() => roleRows(authority), [authority]);
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <KeyRound className="h-5 w-5 text-primary" /> Role holders
        </CardTitle>
        <CardDescription>
          Live from chain via <code>hasRole</code> / <code>owner()</code> /{" "}
          <code>defaultAdmin()</code>. Ordinary AccessControl roles have no single-holder accessor, so
          their holder is reported through <code>heldByHosted</code> instead.
        </CardDescription>
      </CardHeader>
      <CardContent className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Authority</TableHead>
              <TableHead>Contract</TableHead>
              <TableHead>Holder</TableHead>
              <TableHead>Held by</TableHead>
              <TableHead>Capability</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((r) => (
              <TableRow key={r.label}>
                <TableCell className="font-medium text-onSurface">
                  {r.label}
                  <div className="font-mono text-[11px] font-normal text-muted">{r.mechanism}</div>
                </TableCell>
                <TableCell>{r.slot.target ? <AddrLink addr={r.slot.target} /> : <Dash />}</TableCell>
                <TableCell>
                  {r.holder ? (
                    <span className="flex items-center gap-2">
                      <AddrLink addr={r.holder} />
                      {isGovernanceSigner1(r.holder) && (
                        <Badge variant="success">
                          <CheckCircle2 className="h-3 w-3" /> signer-1
                        </Badge>
                      )}
                    </span>
                  ) : (
                    <span className="text-xs text-muted">no single-holder accessor</span>
                  )}
                </TableCell>
                <TableCell>
                  <HeldByBadge held={r.slot.heldByHosted} />
                </TableCell>
                <TableCell className="text-xs text-muted">{r.slot.capability}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

/** heldByHosted: true → control-plane key holds it (executes); false → held elsewhere (proposes). */
function HeldByBadge({ held }: { held?: boolean | null }) {
  if (held === true) return <Badge variant="success">control plane</Badge>;
  if (held === false) return <Badge variant="warning">external</Badge>;
  return <Badge variant="neutral">unknown</Badge>;
}

// ---------------------------------------------------------------------------------------------------
// Pending two-step transfers — the ACDAR / Ownable2Step in-flight transfers + live timelock countdown.
// ---------------------------------------------------------------------------------------------------

function PendingTransfersCard({
  authority,
  now,
}: {
  authority: GovernanceAuthorityResp;
  now: number;
}) {
  const adminPending = authority.defaultAdmin.pendingTransfer ?? null;
  const ownerPending = authority.factoryOwner.pendingOwner ?? null;
  const none = !adminPending && !ownerPending;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Hourglass className="h-5 w-5 text-primary" /> Pending transfers &amp; timelocks
        </CardTitle>
        <CardDescription>
          In-flight two-step authority transfers and their live timelock countdown. The Phase-2
          DEFAULT_ADMIN handover surfaces here while it is pending.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {none ? (
          <div className="flex items-center gap-2 rounded-md border border-success/30 bg-success/5 px-3 py-3 text-sm text-onSurface">
            <CheckCircle2 className="h-4 w-4 text-success" />
            No pending transfers — governance is settled on signer-1.
          </div>
        ) : (
          <>
            {adminPending && (
              <PendingRow
                icon={<ArrowRightLeft className="h-4 w-4 text-warning" />}
                title="IssuerRegistry DEFAULT_ADMIN — two-step transfer in flight"
                target={authority.defaultAdmin.target}
                newAdmin={adminPending.newAdmin}
                acceptSchedule={adminPending.acceptSchedule}
                now={now}
                note="ACDAR two-step: acceptDefaultAdminTransfer becomes callable once the 3-day timelock elapses."
              />
            )}
            {ownerPending && (
              <PendingRow
                icon={<ArrowRightLeft className="h-4 w-4 text-warning" />}
                title="DogTagIssuerFactory ownership — two-step transfer in flight"
                target={authority.factoryOwner.target}
                newAdmin={ownerPending}
                now={now}
                note="Ownable2Step: acceptOwnership can be called by the pending owner at any time (no timelock)."
              />
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

function PendingRow({
  icon,
  title,
  target,
  newAdmin,
  acceptSchedule,
  now,
  note,
}: {
  icon: ReactNode;
  title: string;
  target?: string;
  newAdmin: string;
  acceptSchedule?: number;
  now: number;
  note: string;
}) {
  const cd = acceptSchedule ? countdown(acceptSchedule, now) : null;
  return (
    <div className="rounded-md border border-warning/40 bg-warning/5 p-3">
      <div className="flex items-center gap-2 text-sm font-medium text-onSurface">
        {icon}
        {title}
      </div>
      <dl className="mt-2 grid gap-3 text-sm sm:grid-cols-2">
        {target && (
          <div>
            <dt className="text-xs text-muted">Contract</dt>
            <dd className="mt-0.5">
              <AddrLink addr={target} />
            </dd>
          </div>
        )}
        <div>
          <dt className="text-xs text-muted">New admin</dt>
          <dd className="mt-0.5 flex items-center gap-2">
            <AddrLink addr={newAdmin} />
            {isGovernanceSigner1(newAdmin) && (
              <Badge variant="success">
                <CheckCircle2 className="h-3 w-3" /> signer-1
              </Badge>
            )}
          </dd>
        </div>
        {acceptSchedule ? (
          <div>
            <dt className="text-xs text-muted">Accept ETA</dt>
            <dd className="mt-0.5 font-mono text-xs text-onSurface" title={absoluteTime(acceptSchedule)}>
              {absoluteTime(acceptSchedule)}
            </dd>
          </div>
        ) : null}
        {cd && (
          <div>
            <dt className="text-xs text-muted">Timelock</dt>
            <dd className="mt-0.5">
              {cd.elapsed ? (
                <Badge variant="success">
                  <CheckCircle2 className="h-3 w-3" /> accept window open
                </Badge>
              ) : (
                <Badge variant="warning">
                  <Clock className="h-3 w-3" /> {cd.label} remaining
                </Badge>
              )}
            </dd>
          </div>
        )}
      </dl>
      <p className="mt-2 text-xs text-muted">{note}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------
// GovernanceAction dispatch model — how privileged writes route (execute-if-held / propose-if-not).
// ---------------------------------------------------------------------------------------------------

function GovernanceActionModelCard({ authority }: { authority: GovernanceAuthorityResp }) {
  const hostedHoldsAny =
    authority.factoryOwner.heldByHosted === true ||
    authority.whitelistAdmin.heldByHosted === true ||
    authority.defaultAdmin.heldByHosted === true;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Send className="h-5 w-5 text-primary" /> How privileged actions route
        </CardTitle>
        <CardDescription>
          Every privileged write in the portal goes through PR-A's key-holder-agnostic{" "}
          <code>GovernanceAction</code> layer. At dispatch time the backend checks who holds the
          required authority and picks a disposition — it never assumes the old deployer EOA.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="rounded-md border border-success/30 bg-success/5 p-3">
            <div className="flex items-center gap-2 text-sm font-semibold text-onSurface">
              <CheckCircle2 className="h-4 w-4 text-success" /> executed
            </div>
            <p className="mt-1 text-xs text-muted">
              The control-plane signer holds the required authority → the call is signed and broadcast,
              returning a tx hash + explorer link. This is the current state: the signer is signer-1.
            </p>
          </div>
          <div className="rounded-md border border-border bg-surface-muted p-3">
            <div className="flex items-center gap-2 text-sm font-semibold text-onSurface">
              <Send className="h-4 w-4 text-primary" /> proposed
            </div>
            <p className="mt-1 text-xs text-muted">
              The authority sits with a key the control plane does not hold → nothing is broadcast; the{" "}
              <code>{"{ target, calldata }"}</code> payload is returned for the holder to execute
              out-of-band (Safe-compatible).
            </p>
          </div>
        </div>

        <div className="flex items-start gap-2 rounded-md border border-border px-3 py-2 text-xs text-muted">
          <Info className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
          <span>
            Routine capabilities (<code>createIssuer</code> on the Issuers page,{" "}
            <code>whitelistFor</code>/<code>delistFor</code> on the Whitelist page) already dispatch
            through this layer.{" "}
            {hostedHoldsAny
              ? "The control-plane signer currently holds authority, so those execute directly."
              : "The control-plane signer holds none of these authorities, so those route as proposals."}{" "}
            Mutating governance actions (admin-transfer begin/accept/cancel, <code>adminRevoke</code>,
            verifier / consent-key swaps) are captain-initiated through the ACDAR two-step flow and are
            intentionally not wired as live buttons in this read-only console.
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------------------------------
// Timelock model reference — the two-step / delay model of each governed contract, as deployed.
// ---------------------------------------------------------------------------------------------------

function TimelockModelCard() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Clock className="h-5 w-5 text-primary" /> Timelock &amp; two-step model
        </CardTitle>
        <CardDescription>
          The authority-transfer mechanism enforced by each governed contract, as deployed.
        </CardDescription>
      </CardHeader>
      <CardContent className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Contract</TableHead>
              <TableHead>Mechanism</TableHead>
              <TableHead>Delay</TableHead>
              <TableHead>Two-step sequence</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {TIMELOCK_MODELS.map((m) => (
              <TableRow key={m.contract}>
                <TableCell className="font-medium text-onSurface">
                  {m.contract}
                  <div className="text-[11px] font-normal text-muted">{m.capability}</div>
                </TableCell>
                <TableCell className="font-mono text-xs">{m.mechanism}</TableCell>
                <TableCell>
                  <Badge variant={m.mechanism === "Ownable2Step" ? "neutral" : "warning"}>
                    {m.delay}
                  </Badge>
                </TableCell>
                <TableCell className="font-mono text-[11px] text-muted">{m.steps}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------------------------------
// Phase-2 handover runbook — the executed migration state.
// ---------------------------------------------------------------------------------------------------

function Phase2RunbookCard() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Gavel className="h-5 w-5 text-primary" /> Phase-2 handover
        </CardTitle>
        <CardDescription>
          The scheduled migration of the governance/admin authority to the governance signer.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2">
          <Badge variant="success">
            <CheckCircle2 className="h-3 w-3" /> handover executed
          </Badge>
          <span className="text-sm text-muted">
            Authority moved off the deployer EOA to signer-1.
          </span>
        </div>
        <ul className="space-y-2 text-sm text-onSurface">
          <RunbookStep
            icon={<ShieldCheck className="h-4 w-4 text-success" />}
            text="Factory ownership, WHITELIST_ADMIN and the IssuerRegistry / VerificationRegistry / DogTagSBT DEFAULT_ADMIN are all held by signer-1."
          />
          <RunbookStep
            icon={<TriangleAlert className="h-4 w-4 text-muted" />}
            text="The original deployer EOA lost those three governance authorities in Phase-2, but it is not role-free: it still holds the retired owner-revealing SBT's ISSUER_ROLE plus historical record-type whitelists, so never reuse it as a neutral key."
          />
          <RunbookStep
            icon={<History className="h-4 w-4 text-primary" />}
            text="Shipped as reviewable code and merged to mainline: contracts/script/GovernanceMigration.sol + MigrateGovernance.s.sol (two-phase begin/accept); runbook docs/GOVERNANCE_MIGRATION.md."
          />
          <RunbookStep
            icon={<Send className="h-4 w-4 text-primary" />}
            text="The control-plane ADMIN_PRIVATE_KEY is now signer-1: privileged writes execute directly; were it the former deployer EOA they would correctly downgrade to GovernanceAction proposals."
          />
        </ul>
      </CardContent>
    </Card>
  );
}

function RunbookStep({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <li className="flex items-start gap-2">
      <span className="mt-0.5 shrink-0">{icon}</span>
      <span className="text-sm text-muted">{text}</span>
    </li>
  );
}

// ---------------------------------------------------------------------------------------------------
// Shared bits.
// ---------------------------------------------------------------------------------------------------

/** A short, explorer-linked address, with an optional copy button. */
function AddrLink({ addr, withCopy }: { addr: string; withCopy?: boolean }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <a
        href={explorerAddressUrl(addr)}
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 font-mono text-xs text-primary hover:underline"
        title={addr}
      >
        {shortAddr(addr)}
        <ExternalLink className="h-3 w-3" />
      </a>
      {withCopy && (
        <Button
          size="sm"
          variant="ghost"
          className="h-6 w-6 p-0"
          onClick={() => void navigator.clipboard?.writeText(addr)}
          title="Copy address"
        >
          <Copy className="h-3.5 w-3.5" />
        </Button>
      )}
    </span>
  );
}

function Dash() {
  return <span className="text-muted">-</span>;
}
