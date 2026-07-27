import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  APPOINTMENT_STATES,
} from "@dogtag/ui";
import type { AppointmentInput, AppointmentStatus, CrmAppointment, CrmClient } from "@dogtag/ui";
import { ArrowLeft, CalendarPlus } from "lucide-react";
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { useAction, useDebounced } from "../app/crm";
import { fromDateTimeInput, nowSec, toDateTimeInput } from "../lib/time";

/** Sentinel for "no specific pet" — a Select cannot hold an empty-string value. */
const NO_PET = "none";
const DEFAULT_SLOT_SECS = 3600;

/**
 * Book or edit an appointment. The client picker searches the directory server-side rather than
 * loading every customer into a dropdown, so it still works once the shop has thousands of them.
 */
export function AppointmentForm({ mode }: { mode: "create" | "edit" }) {
  const { id = "" } = useParams();
  const [params] = useSearchParams();
  const { api } = useApp();
  const navigate = useNavigate();
  const { run, busy } = useAction();

  const [existing, setExisting] = useState<CrmAppointment | null>(null);
  const [loading, setLoading] = useState(mode === "edit");
  const [loadError, setLoadError] = useState<string | null>(null);

  const [clientId, setClientId] = useState(params.get("clientId") ?? "");
  const [petId, setPetId] = useState<string>(NO_PET);
  const [service, setService] = useState("");
  const [startAt, setStartAt] = useState(() => toDateTimeInput(nowSec() + DEFAULT_SLOT_SECS));
  const [endAt, setEndAt] = useState("");
  const [status, setStatus] = useState<AppointmentStatus>("scheduled");
  const [groomer, setGroomer] = useState("");
  const [notes, setNotes] = useState("");

  // the chosen client, loaded so the pet picker can offer that client's pets
  const [client, setClient] = useState<CrmClient | null>(null);

  useEffect(() => {
    if (mode !== "edit" || !id) return;
    let cancelled = false;
    api
      .getAppointment(id)
      .then((a) => {
        if (cancelled) return;
        setExisting(a);
        setClientId(a.clientId);
        setPetId(a.petId ?? NO_PET);
        setService(a.service);
        setStartAt(toDateTimeInput(a.startAt));
        setEndAt(toDateTimeInput(a.endAt));
        setStatus(a.status);
        setGroomer(a.groomer);
        setNotes(a.notes);
      })
      .catch((e: Error) => !cancelled && setLoadError(e.message))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [api, id, mode]);

  useEffect(() => {
    if (!clientId) {
      setClient(null);
      return;
    }
    let cancelled = false;
    api
      .getClient(clientId)
      .then((c) => !cancelled && setClient(c))
      .catch(() => !cancelled && setClient(null));
    return () => {
      cancelled = true;
    };
  }, [api, clientId]);

  // a pet from a previous client is not a valid choice for the new one
  useEffect(() => {
    if (petId !== NO_PET && client && !client.pets.some((p) => p.petId === petId)) {
      setPetId(NO_PET);
    }
  }, [client, petId]);

  const startSecs = fromDateTimeInput(startAt);
  const endSecs = endAt ? fromDateTimeInput(endAt) : 0;
  const slotInvalid = endSecs !== 0 && endSecs <= startSecs;
  const canSubmit = clientId !== "" && startSecs !== 0 && !slotInvalid;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    const body: AppointmentInput = {
      clientId,
      petId: petId === NO_PET ? null : petId,
      service: service.trim(),
      startAt: startSecs,
      endAt: endSecs || undefined,
      status,
      notes,
      groomer: groomer.trim(),
    };
    const ok = await run(
      async () => {
        const saved =
          mode === "edit"
            ? await api.updateAppointment(id, body)
            : await api.createAppointment(body);
        navigate(`/appointments/${saved.appointmentId}`);
      },
      {
        success: mode === "edit" ? "Appointment updated" : "Appointment booked",
        failure: mode === "edit" ? "Could not update the appointment" : "Could not book the appointment",
      },
    );
    return ok;
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading appointment…
      </div>
    );
  }
  if (loadError) {
    return (
      <Card>
        <CardContent className="space-y-3 pt-6">
          <p className="text-sm text-danger">{loadError}</p>
          <Button variant="outline" size="sm" onClick={() => navigate("/appointments")}>
            <ArrowLeft className="h-4 w-4" /> Back to appointments
          </Button>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Link
        to={existing ? `/appointments/${existing.appointmentId}` : "/appointments"}
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> {existing ? "Back to appointment" : "All appointments"}
      </Link>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <CalendarPlus className="h-5 w-5 text-primary" />
            {mode === "edit" ? "Edit appointment" : "New appointment"}
          </CardTitle>
          <CardDescription>
            Pick the client and pet, then the slot. You will run the vaccination check from the
            appointment itself.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form className="space-y-6" onSubmit={submit}>
            {/* An `.ics`-imported booking arrives UNASSIGNED — a calendar invite names an event, not
                a DogTag client, and the import refuses to invent one. The backend requires a real
                client on every write, so saving is blocked until one is picked; say that here rather
                than leaving the operator staring at a disabled button. */}
            {existing && !existing.clientId && (
              <p className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning">
                This booking came from an imported calendar file and has no client yet. Pick the
                client it belongs to before saving — the verification flow needs one.
              </p>
            )}
            <ClientPicker value={clientId} selected={client} onChange={setClientId} />

            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="appt-pet">Pet</Label>
                <Select value={petId} onValueChange={setPetId} disabled={!client}>
                  <SelectTrigger id="appt-pet">
                    <SelectValue placeholder="No specific pet" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_PET}>No specific pet</SelectItem>
                    {(client?.pets ?? []).map((p) => (
                      <SelectItem key={p.petId} value={p.petId}>
                        {p.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="appt-service">Service</Label>
                <Input
                  id="appt-service"
                  value={service}
                  onChange={(e) => setService(e.target.value)}
                  placeholder="Full groom"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="appt-start">Starts</Label>
                <Input
                  id="appt-start"
                  type="datetime-local"
                  value={startAt}
                  onChange={(e) => setStartAt(e.target.value)}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="appt-end">Ends</Label>
                <Input
                  id="appt-end"
                  type="datetime-local"
                  value={endAt}
                  invalid={slotInvalid}
                  onChange={(e) => setEndAt(e.target.value)}
                />
                <p className="text-xs text-muted">
                  {slotInvalid
                    ? "The end time must be after the start time."
                    : "Leave blank for a one-hour slot."}
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="appt-groomer">Groomer</Label>
                <Input
                  id="appt-groomer"
                  value={groomer}
                  onChange={(e) => setGroomer(e.target.value)}
                  placeholder="Who is doing the groom"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="appt-status">Status</Label>
                <Select value={status} onValueChange={(v) => setStatus(v as AppointmentStatus)}>
                  <SelectTrigger id="appt-status">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {APPOINTMENT_STATES.map((s) => (
                      <SelectItem key={s} value={s}>
                        {s.replace(/_/g, " ")}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="appt-notes">Notes</Label>
              <textarea
                id="appt-notes"
                className="min-h-20 w-full rounded-md border border-input bg-surface px-3 py-2 text-sm text-onSurface placeholder:text-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder="Handling notes for this visit…"
              />
            </div>

            <div className="flex flex-wrap gap-2">
              <Button type="submit" loading={busy} disabled={!canSubmit}>
                {mode === "edit" ? "Save changes" : "Book appointment"}
              </Button>
              <Button type="button" variant="ghost" onClick={() => navigate(-1)}>
                Cancel
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

/**
 * Search-as-you-type client picker. Queries the backend on each pause instead of holding the whole
 * directory in a `<select>`, which is what keeps this usable at real customer volumes.
 */
function ClientPicker({
  value,
  selected,
  onChange,
}: {
  value: string;
  selected: CrmClient | null;
  onChange: (clientId: string) => void;
}) {
  const { api } = useApp();
  const [search, setSearch] = useState("");
  const [results, setResults] = useState<CrmClient[]>([]);
  const [open, setOpen] = useState(false);
  const q = useDebounced(search);

  const load = useCallback(async () => {
    const page = await api.listClients({ q, limit: 8 });
    setResults(page.rows);
  }, [api, q]);

  useEffect(() => {
    if (!open) return;
    void load();
  }, [load, open]);

  if (value && selected && !open) {
    return (
      <div className="space-y-2">
        <Label>Client</Label>
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-surface-muted px-3 py-2">
          <div className="text-sm">
            <span className="font-medium text-onSurface">{selected.name}</span>
            <span className="ml-2 text-muted">{selected.phone || selected.email}</span>
          </div>
          <Button type="button" variant="ghost" size="sm" onClick={() => setOpen(true)}>
            Change
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <Label htmlFor="appt-client">Client</Label>
      <Input
        id="appt-client"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        onFocus={() => setOpen(true)}
        placeholder="Search a client by name, phone or pet…"
        autoComplete="off"
      />
      <div className="max-h-56 space-y-1 overflow-y-auto rounded-md border border-border p-1">
        {results.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted">
            No clients found.{" "}
            <Link to="/clients" className="text-primary hover:underline">
              Add one
            </Link>
            .
          </p>
        ) : (
          results.map((c) => (
            <button
              key={c.clientId}
              type="button"
              onClick={() => {
                onChange(c.clientId);
                setOpen(false);
                setSearch("");
              }}
              className="flex w-full flex-col items-start rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-surface-muted"
            >
              <span className="font-medium text-onSurface">{c.name}</span>
              <span className="text-xs text-muted">
                {[c.phone, c.pets.map((p) => p.name).join(", ")].filter(Boolean).join(" · ")}
              </span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
