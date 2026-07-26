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
  explorerTxUrl,
} from "@dogtag/ui";
import type { CrmVerification } from "@dogtag/ui";
import { ArrowLeft, Lock, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { DisclosureBadge, VerificationStatusBadge } from "../app/crm";
import { formatDateTime } from "../lib/time";

/**
 * The full evidence for one verification: what was checked, for whom, the on-chain facts, and
 * whatever the owner chose to disclose.
 *
 * On the ZK path the disclosed list is EMPTY and this page says so explicitly — that emptiness is
 * the privacy guarantee, and an operator should be able to see it stated rather than infer it from
 * a blank table.
 */
export function VerificationDetail() {
  const { id = "" } = useParams();
  const { api } = useApp();
  const navigate = useNavigate();
  const [v, setV] = useState<CrmVerification | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getVerification(id)
      .then((row) => !cancelled && setV(row))
      .catch((e: Error) => !cancelled && setError(e.message))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [api, id]);

  if (loading) {
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading verification…
      </div>
    );
  }
  if (error || !v) {
    return (
      <Card>
        <CardContent className="space-y-3 pt-6">
          <p className="text-sm text-danger">{error ?? "Verification not found."}</p>
          <Button variant="outline" size="sm" onClick={() => navigate("/verifications")}>
            <ArrowLeft className="h-4 w-4" /> Back to all verifications
          </Button>
        </CardContent>
      </Card>
    );
  }

  // On `error` the backend stores the failure message in the txHash field (mirroring the verify
  // session), so only a real 0x hash may be rendered as an explorer link.
  const isTxHash = !!v.txHash && /^0x[0-9a-fA-F]{64}$/.test(v.txHash);

  return (
    <div className="space-y-4">
      <Link
        to="/verifications"
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> All verifications
      </Link>

      <Card>
        <CardHeader>
          <CardTitle className="flex flex-wrap items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" /> {v.purpose}
            <VerificationStatusBadge status={v.status} />
            <DisclosureBadge disclosedKeyPaths={v.disclosedKeyPaths} />
          </CardTitle>
          <CardDescription>Started {formatDateTime(v.createdAt)}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <dl className="grid gap-4 text-sm sm:grid-cols-3">
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Client</dt>
              <dd className="mt-1">
                {v.clientId ? (
                  <Link to={`/clients/${v.clientId}`} className="text-primary hover:underline">
                    {v.clientName || v.clientId}
                  </Link>
                ) : (
                  <span className="text-muted">Ad-hoc (not linked to a client)</span>
                )}
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">
                Appointment
              </dt>
              <dd className="mt-1">
                {v.appointmentId ? (
                  <Link
                    to={`/appointments/${v.appointmentId}`}
                    className="text-primary hover:underline"
                  >
                    Open appointment
                  </Link>
                ) : (
                  <span className="text-muted">Not linked</span>
                )}
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Pet</dt>
              <dd className="mt-1 text-onSurface">{v.petName || "—"}</dd>
            </div>
          </dl>

          <div className="space-y-3">
            <p className="text-xs font-semibold uppercase tracking-wide text-muted">
              On-chain evidence
            </p>
            <dl className="grid gap-4 text-sm sm:grid-cols-2">
              <div className="min-w-0">
                <dt className="text-xs text-muted">Record type</dt>
                <dd className="mt-1 text-onSurface">{v.recordType}</dd>
              </div>
              <div className="min-w-0">
                <dt className="text-xs text-muted">DogTag id</dt>
                <dd className="mt-1 break-all font-mono text-onSurface">{v.dogTagId ?? "—"}</dd>
              </div>
              <div className="min-w-0 sm:col-span-2">
                <dt className="text-xs text-muted">
                  {v.status === "error" ? "Failure" : "Transaction"}
                </dt>
                <dd className="mt-1 break-all font-mono text-sm">
                  {!v.txHash ? (
                    <span className="text-muted">—</span>
                  ) : isTxHash ? (
                    <a
                      href={explorerTxUrl(v.txHash)}
                      target="_blank"
                      rel="noreferrer"
                      className="text-primary hover:underline"
                    >
                      {v.txHash}
                    </a>
                  ) : (
                    <span className="text-danger">{v.txHash}</span>
                  )}
                </dd>
              </div>
              <div className="min-w-0 sm:col-span-2">
                <dt className="text-xs text-muted">Nullifier (consumed on chain)</dt>
                <dd className="mt-1 break-all font-mono text-onSurface">{v.nullifier ?? "—"}</dd>
              </div>
              <div className="min-w-0">
                <dt className="text-xs text-muted">Last updated</dt>
                <dd className="mt-1 text-onSurface">{formatDateTime(v.updatedAt)}</dd>
              </div>
            </dl>
          </div>

          <div className="space-y-3">
            <p className="text-xs font-semibold uppercase tracking-wide text-muted">
              Disclosed by the owner
            </p>
            {v.disclosedKeyPaths.length === 0 ? (
              <div className="flex items-start gap-2 rounded-md border border-border bg-surface-muted p-3 text-sm">
                <Lock className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                <p className="text-muted">
                  Nothing. The owner proved the credential is valid without revealing its contents,
                  and no credential data was written on chain.
                  <Badge variant="neutral" className="ml-2">
                    Private by design
                  </Badge>
                </p>
              </div>
            ) : (
              <>
                <p className="text-sm text-muted">
                  The owner chose to reveal these fields. Their VALUES were shown to the operator at
                  the time of the check and are deliberately not stored here.
                </p>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Field</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {v.disclosedKeyPaths.map((path) => (
                      <TableRow key={path}>
                        <TableCell className="font-mono text-xs">{path}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
