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
} from "@dogtag/ui";
import type { CrmAppointment, CrmClient, CrmVerification } from "@dogtag/ui";
import { ArrowLeft, CalendarPlus, Pencil, ShieldCheck, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import {
  AppointmentStatusBadge,
  ListPlaceholder,
  DisclosureBadge,
  VerificationStatusBadge,
  useAction,
} from "../app/crm";
import { formatDateTime } from "../lib/time";
import { ClientForm } from "./Clients";

/**
 * One customer: their particulars and pets, their booking history, and every verification the shop
 * has performed for them. This is where an operator lands from the directory before booking.
 */
export function ClientDetail() {
  const { id = "" } = useParams();
  const { api } = useApp();
  const navigate = useNavigate();
  const { run, busy } = useAction();

  const [client, setClient] = useState<CrmClient | null>(null);
  const [appointments, setAppointments] = useState<CrmAppointment[]>([]);
  const [verifications, setVerifications] = useState<CrmVerification[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // Three bounded reads rather than one fat endpoint: each is independently pageable and the
      // detail view only ever needs the most recent slice of each.
      const [c, appts, verifs] = await Promise.all([
        api.getClient(id),
        api.listAppointments({ clientId: id, limit: 25 }),
        api.listVerifications({ clientId: id, limit: 25 }),
      ]);
      setClient(c);
      setAppointments(appts.rows);
      setVerifications(verifs.rows);
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

  if (loading && !client) {
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading client…
      </div>
    );
  }
  if (error || !client) {
    return (
      <Card>
        <CardContent className="space-y-3 pt-6">
          <p className="text-sm text-danger">{error ?? "Client not found."}</p>
          <Button variant="outline" size="sm" onClick={() => navigate("/clients")}>
            <ArrowLeft className="h-4 w-4" /> Back to clients
          </Button>
        </CardContent>
      </Card>
    );
  }

  if (editing) {
    return (
      <ClientForm
        title={`Edit ${client.name}`}
        initial={client}
        onCancel={() => setEditing(false)}
        onSubmit={async (body) => {
          await api.updateClient(id, body);
          setEditing(false);
          await load();
        }}
      />
    );
  }

  async function remove() {
    const ok = await run(() => api.deleteClient(id), {
      success: "Client deleted",
      // the backend refuses while bookings exist rather than orphaning them
      failure: "Could not delete the client",
    });
    if (ok) navigate("/clients");
  }

  return (
    <div className="space-y-4">
      <Link
        to="/clients"
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> All clients
      </Link>

      <Card>
        <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle>{client.name}</CardTitle>
            <CardDescription>Client since {formatDateTime(client.createdAt)}</CardDescription>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
              <Pencil className="h-4 w-4" /> Edit
            </Button>
            <Button
              size="sm"
              onClick={() => navigate(`/appointments/new?clientId=${client.clientId}`)}
            >
              <CalendarPlus className="h-4 w-4" /> Book appointment
            </Button>
            <Button variant="ghost" size="sm" loading={busy} onClick={() => void remove()}>
              <Trash2 className="h-4 w-4" /> Delete
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-6">
          <dl className="grid gap-4 text-sm sm:grid-cols-3">
            <Field label="Email" value={client.email} />
            <Field label="Phone" value={client.phone} />
            <Field label="Address" value={client.address} />
          </dl>
          {client.notes && (
            <div className="space-y-1">
              <p className="text-xs font-semibold uppercase tracking-wide text-muted">Notes</p>
              <p className="whitespace-pre-wrap text-sm text-onSurface">{client.notes}</p>
            </div>
          )}

          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-wide text-muted">Pets</p>
            {client.pets.length === 0 ? (
              <p className="text-sm text-muted">No pets recorded.</p>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2">
                {client.pets.map((p) => (
                  <div key={p.petId} className="rounded-md border border-border p-3 text-sm">
                    <p className="font-medium text-onSurface">{p.name}</p>
                    <p className="text-muted">
                      {[p.breed, p.sex, p.dateOfBirth].filter(Boolean).join(" · ") || "—"}
                    </p>
                    {p.dogTagId && (
                      <p className="mt-2">
                        <Badge variant="outline" className="font-mono">
                          DogTag {p.dogTagId}
                        </Badge>
                      </p>
                    )}
                    {p.notes && <p className="mt-2 text-muted">{p.notes}</p>}
                  </div>
                ))}
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Appointments</CardTitle>
          <CardDescription>The most recent bookings for this client.</CardDescription>
        </CardHeader>
        <CardContent>
          <ListPlaceholder
            loading={loading}
            error={null}
            empty={appointments.length === 0}
            emptyMessage="No appointments booked yet."
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>When</TableHead>
                  <TableHead>Service</TableHead>
                  <TableHead>Pet</TableHead>
                  <TableHead>Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {appointments.map((a) => (
                  <TableRow key={a.appointmentId}>
                    <TableCell>
                      <Link
                        to={`/appointments/${a.appointmentId}`}
                        className="text-primary hover:underline"
                      >
                        {formatDateTime(a.startAt)}
                      </Link>
                    </TableCell>
                    <TableCell>{a.service || "—"}</TableCell>
                    <TableCell>{a.petName || "—"}</TableCell>
                    <TableCell>
                      <AppointmentStatusBadge status={a.status} />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </ListPlaceholder>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <ShieldCheck className="h-4 w-4 text-primary" /> Verifications
          </CardTitle>
          <CardDescription>
            Every proof-of-verification recorded for this client.{" "}
            <Link
              to={`/verifications?clientId=${client.clientId}`}
              className="text-primary hover:underline"
            >
              See all
            </Link>
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ListPlaceholder
            loading={loading}
            error={null}
            empty={verifications.length === 0}
            emptyMessage="No verifications recorded for this client yet."
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
                    <TableCell>
                      <VerificationStatusBadge status={v.status} />
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

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs font-semibold uppercase tracking-wide text-muted">{label}</dt>
      <dd className="mt-1 text-onSurface">{value || "—"}</dd>
    </div>
  );
}
