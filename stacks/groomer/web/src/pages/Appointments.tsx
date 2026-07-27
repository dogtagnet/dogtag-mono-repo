import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  APPOINTMENT_STATES,
} from "@dogtag/ui";
import type { AppointmentStatus, CrmAppointment } from "@dogtag/ui";
import { CalendarPlus, ListChecks, Search } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useApp } from "../app/AppContext";
import {
  AppointmentClient,
  AppointmentStatusBadge,
  FilterBar,
  FilterField,
  ListPlaceholder,
  PAGE_SIZE,
  Pager,
  statusLabel,
  useDebounced,
  useList,
} from "../app/crm";
import { addDays, formatDate, formatSlot, startOfDayFromInput } from "../lib/time";

/** Sentinel for "no status filter" — a Select cannot hold an empty-string value. */
const ANY = "any";

/**
 * The booking list (impl §5.2): every appointment, searchable by client / pet / service / groomer
 * and filterable by status and date range. All filtering happens on the backend.
 */
export function Appointments() {
  const { api } = useApp();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<AppointmentStatus | typeof ANY>(ANY);
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [offset, setOffset] = useState(0);
  const q = useDebounced(search);

  // The date inputs are inclusive days; `to` becomes the START of the day AFTER, because the
  // backend window is half-open [from, to). Without that, "to = today" would exclude today. That
  // next day is stepped with `addDays`, never `+ 86_400`: a DST day is 23h or 25h, so a fixed step
  // would clip the selected end date's last hour or spill an hour of the day after into it.
  const from = fromDate ? startOfDayFromInput(fromDate) : undefined;
  const to = toDate ? addDays(startOfDayFromInput(toDate), 1) : undefined;

  const { page, loading, error } = useList<CrmAppointment>(
    () =>
      api.listAppointments({
        q,
        status: status === ANY ? undefined : status,
        from,
        to,
        limit: PAGE_SIZE,
        offset,
      }),
    [q, status, from, to, offset],
  );

  function resetPaging<T>(setter: (v: T) => void) {
    return (v: T) => {
      setter(v);
      setOffset(0);
    };
  }

  const rows = page?.rows ?? [];
  const filtered = q !== "" || status !== ANY || !!from || !!to;

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ListChecks className="h-5 w-5 text-primary" /> Appointments
          </CardTitle>
          <CardDescription>
            Every booking in the shop. Open one to run the vaccination check for that visit.
          </CardDescription>
        </div>
        <Button onClick={() => navigate("/appointments/new")}>
          <CalendarPlus className="h-4 w-4" /> New appointment
        </Button>
      </CardHeader>
      <CardContent className="space-y-4">
        <FilterBar>
          {/* The search box carries the longest placeholder and the longest values, so it takes the
              largest share; From/To get a whole column EACH rather than splitting one between them,
              which is what made the date range unreadable. */}
          <FilterField span="sm:col-span-2 xl:col-span-5" label="Search" htmlFor="appt-search">
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
              <Input
                id="appt-search"
                className="pl-9"
                placeholder="Search client, pet, service or groomer…"
                value={search}
                onChange={(e) => resetPaging(setSearch)(e.target.value)}
                aria-label="Search appointments"
              />
            </div>
          </FilterField>
          <FilterField span="xl:col-span-3" label="Status">
            <Select
              value={status}
              onValueChange={(v) => resetPaging(setStatus)(v as AppointmentStatus | typeof ANY)}
            >
              <SelectTrigger aria-label="Filter by status">
                <SelectValue placeholder="Any status" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ANY}>Any status</SelectItem>
                {APPOINTMENT_STATES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {statusLabel(s)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FilterField>
          <FilterField span="xl:col-span-2" label="From" htmlFor="appt-from">
            <Input
              id="appt-from"
              type="date"
              value={fromDate}
              onChange={(e) => resetPaging(setFromDate)(e.target.value)}
            />
          </FilterField>
          <FilterField span="xl:col-span-2" label="To" htmlFor="appt-to">
            <Input
              id="appt-to"
              type="date"
              value={toDate}
              onChange={(e) => resetPaging(setToDate)(e.target.value)}
            />
          </FilterField>
        </FilterBar>

        {filtered && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setSearch("");
              setStatus(ANY);
              setFromDate("");
              setToDate("");
              setOffset(0);
            }}
          >
            Clear filters
          </Button>
        )}

        <ListPlaceholder
          loading={loading}
          error={error}
          empty={rows.length === 0}
          emptyMessage={
            filtered
              ? "No appointments match these filters."
              : "No appointments booked yet — create one to get started."
          }
        >
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="whitespace-nowrap">Date</TableHead>
                <TableHead className="whitespace-nowrap">Time</TableHead>
                <TableHead>Client</TableHead>
                <TableHead>Pet</TableHead>
                <TableHead>Service</TableHead>
                <TableHead>Groomer</TableHead>
                <TableHead className="whitespace-nowrap">Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((a) => (
                <TableRow key={a.appointmentId}>
                  <TableCell className="whitespace-nowrap">
                    <Link
                      to={`/appointments/${a.appointmentId}`}
                      className="font-medium text-primary hover:underline"
                    >
                      {formatDate(a.startAt)}
                    </Link>
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-sm text-muted">
                    {formatSlot(a.startAt, a.endAt)}
                  </TableCell>
                  <TableCell>
                    <AppointmentClient appointment={a} />
                  </TableCell>
                  <TableCell>{a.petName || "—"}</TableCell>
                  <TableCell>{a.service || "—"}</TableCell>
                  <TableCell className="text-muted">{a.groomer || "—"}</TableCell>
                  <TableCell className="whitespace-nowrap">
                    <AppointmentStatusBadge status={a.status} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </ListPlaceholder>

        <Pager
          total={page?.total ?? 0}
          offset={offset}
          limit={page?.limit ?? PAGE_SIZE}
          onOffset={setOffset}
        />
      </CardContent>
    </Card>
  );
}
