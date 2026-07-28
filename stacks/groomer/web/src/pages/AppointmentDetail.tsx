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
  VerifyFlow,
} from "@dogtag/ui";
import type { CrmAppointment } from "@dogtag/ui";
import { ArrowLeft, CalendarPlus, Pencil, ShieldCheck, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import {
  AppointmentClient,
  AppointmentStatusBadge,
  DisclosureBadge,
  ListPlaceholder,
  VerificationStatusBadge,
  useAction,
} from "../app/crm";
import { formatDate, formatDateTime, formatSlot } from "../lib/time";
import { ShareAppointmentDialog } from "./ShareAppointmentDialog";
import { VERIFY_PURPOSES } from "./verifyPurposes";

/**
 * One booking — and the place a verification is STARTED FROM (the captain's primary flow).
 *
 * Starting the check here passes this appointment's id to `POST /verify/session/start`, so the
 * resulting verification is stored against both this appointment and its client and shows up in
 * "All verifications" filtered either way. The owner-hidden ZK consent mechanics are untouched:
 * the QR, the owner's on-device proof and the on-chain record are exactly as before — this page
 * only supplies the business context around them.
 */
export function AppointmentDetail() {
  const { id = "" } = useParams();
  const { api } = useApp();
  const navigate = useNavigate();
  const { run, busy } = useAction();

  const [appt, setAppt] = useState<CrmAppointment | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [sharing, setSharing] = useState(false);

  const load = useCallback(async () => {
    try {
      // the single-appointment read already carries this appointment's verifications
      const a = await api.getAppointment(id);
      setAppt(a);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [api, id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading && !appt) {
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading appointment…
      </div>
    );
  }
  if (error || !appt) {
    return (
      <Card>
        <CardContent className="space-y-3 pt-6">
          <p className="text-sm text-danger">{error ?? "Appointment not found."}</p>
          <Button variant="outline" size="sm" onClick={() => navigate("/appointments")}>
            <ArrowLeft className="h-4 w-4" /> Back to appointments
          </Button>
        </CardContent>
      </Card>
    );
  }

  async function remove() {
    const ok = await run(() => api.deleteAppointment(id), {
      success: "Appointment deleted",
      // the backend refuses once verifications hang off it — cancel instead of deleting evidence
      failure: "Could not delete the appointment",
    });
    if (ok) navigate("/appointments");
  }

  const verifications = appt.verifications ?? [];

  return (
    <div className="space-y-4">
      <ShareAppointmentDialog
        appointment={sharing ? appt : null}
        onClose={() => setSharing(false)}
      />
      <Link
        to="/appointments"
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> All appointments
      </Link>

      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div className="space-y-1">
            <CardTitle className="flex flex-wrap items-center gap-2">
              {appt.service || "Appointment"}
              <AppointmentStatusBadge status={appt.status} />
            </CardTitle>
            <CardDescription>
              {formatDate(appt.startAt)} · {formatSlot(appt.startAt, appt.endAt)}
            </CardDescription>
          </div>
          <div className="flex flex-wrap gap-2">
            {/* The client half of the calendar: hand THIS booking to the person it belongs to. */}
            <Button size="sm" onClick={() => setSharing(true)}>
              <CalendarPlus className="h-4 w-4" /> Send to client
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => navigate(`/appointments/${appt.appointmentId}/edit`)}
            >
              <Pencil className="h-4 w-4" /> Edit
            </Button>
            <Button variant="ghost" size="sm" loading={busy} onClick={() => void remove()}>
              <Trash2 className="h-4 w-4" /> Delete
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-4 text-sm sm:grid-cols-4">
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Client</dt>
              <dd className="mt-1">
                <AppointmentClient appointment={appt} className="text-primary hover:underline" />
              </dd>
            </div>
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Pet</dt>
              <dd className="mt-1 text-onSurface">{appt.petName || "—"}</dd>
            </div>
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Groomer</dt>
              <dd className="mt-1 text-onSurface">{appt.groomer || "—"}</dd>
            </div>
            <div>
              <dt className="text-xs font-semibold uppercase tracking-wide text-muted">Booked</dt>
              <dd className="mt-1 text-onSurface">{formatDateTime(appt.createdAt)}</dd>
            </div>
          </dl>
          {appt.notes && (
            <div className="mt-4 space-y-1">
              <p className="text-xs font-semibold uppercase tracking-wide text-muted">Notes</p>
              <p className="whitespace-pre-wrap text-sm text-onSurface">{appt.notes}</p>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <ShieldCheck className="h-4 w-4 text-primary" /> Verification
            </CardTitle>
            <CardDescription>
              Check this pet's vaccination status for this visit. The owner approves on their phone;
              nothing about them is revealed to you beyond what they choose to share.
            </CardDescription>
          </div>
          {!verifying && (
            <Button onClick={() => setVerifying(true)}>
              <ShieldCheck className="h-4 w-4" /> Start verification
            </Button>
          )}
        </CardHeader>
        <CardContent className="space-y-4">
          {verifying && (
            <div className="space-y-3">
              <VerifyFlow
                client={api}
                purposes={VERIFY_PURPOSES}
                appointmentId={appt.appointmentId}
                showDemo={false}
                pollSession={async (sessionId) => {
                  const s = await api.verifySessionStatus(sessionId);
                  return { status: s.status, txHash: s.txHash ?? undefined };
                }}
                onSettled={() => void load()}
              />
              <Button variant="ghost" size="sm" onClick={() => setVerifying(false)}>
                Close
              </Button>
            </div>
          )}

          <ListPlaceholder
            loading={loading}
            error={null}
            empty={verifications.length === 0}
            emptyMessage="No verification has been run for this appointment yet."
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>When</TableHead>
                  <TableHead>Purpose</TableHead>
                  <TableHead>Disclosed</TableHead>
                  <TableHead>Result</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {verifications.map((v) => (
                  <TableRow key={v.verificationId}>
                    <TableCell>
                      <Link
                        to={`/verifications/${v.verificationId}`}
                        className="text-primary hover:underline"
                      >
                        {formatDateTime(v.createdAt)}
                      </Link>
                    </TableCell>
                    <TableCell>{v.purpose}</TableCell>
                    <TableCell>
                      <DisclosureBadge disclosedKeyPaths={v.disclosedKeyPaths} />
                    </TableCell>
                    <TableCell className="space-x-2">
                      <VerificationStatusBadge status={v.status} />
                      {v.dogTagId && (
                        <Badge variant="outline" className="font-mono">
                          tag {v.dogTagId}
                        </Badge>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </ListPlaceholder>
        </CardContent>
      </Card>
    </div>
  );
}
