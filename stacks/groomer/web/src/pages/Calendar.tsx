import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
} from "@dogtag/ui";
import type { CrmAppointment } from "@dogtag/ui";
import { CalendarDays, CalendarPlus, ChevronLeft, ChevronRight } from "lucide-react";
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { AppointmentStatusBadge, useList } from "../app/crm";
import {
  DAY_SECS,
  formatDate,
  formatSlot,
  formatTime,
  nowSec,
  startOfDay,
  startOfWeek,
} from "../lib/time";

type View = "day" | "week";

/**
 * The groomer's daily working surface: a day or week grid of bookings.
 *
 * The window is fetched as a single half-open `[from, to)` range query against the backend's indexed
 * `startAt`, so the calendar pulls one bounded page for the visible period rather than the whole
 * booking history.
 */
export function Calendar() {
  const { api } = useApp();
  const navigate = useNavigate();
  const [view, setView] = useState<View>("week");
  const [anchor, setAnchor] = useState(() => nowSec());

  const from = view === "day" ? startOfDay(anchor) : startOfWeek(anchor);
  const days = view === "day" ? 1 : 7;
  const to = from + days * DAY_SECS;

  const { page, loading, error } = useList<CrmAppointment>(
    // MAX_PAGE-sized: a week of one shop's bookings comfortably fits one page, and the pager on the
    // Appointments list is the right surface for anything beyond that.
    () => api.listAppointments({ from, to, limit: 200 }),
    [from, to],
  );

  // bucket the window's appointments by local day so each column renders in one pass
  const byDay = useMemo(() => {
    const buckets = new Map<number, CrmAppointment[]>();
    for (let i = 0; i < days; i += 1) buckets.set(from + i * DAY_SECS, []);
    for (const a of page?.rows ?? []) {
      const key = startOfDay(a.startAt);
      // guard against a DST-shifted day boundary falling outside the pre-seeded keys
      const bucket = buckets.get(key);
      if (bucket) bucket.push(a);
    }
    return buckets;
  }, [page, from, days]);

  const step = days * DAY_SECS;
  const rangeLabel =
    view === "day"
      ? formatDate(from)
      : `${formatDate(from)} – ${formatDate(to - DAY_SECS)}`;

  return (
    <Card>
      <CardHeader className="space-y-4">
        <div className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2">
              <CalendarDays className="h-5 w-5 text-primary" /> Calendar
            </CardTitle>
            <CardDescription>{rangeLabel}</CardDescription>
          </div>
          <Button onClick={() => navigate("/appointments/new")}>
            <CalendarPlus className="h-4 w-4" /> New appointment
          </Button>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1">
            <Button
              variant="outline"
              size="icon"
              aria-label="Previous period"
              onClick={() => setAnchor((a) => a - step)}
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <Button variant="outline" size="sm" onClick={() => setAnchor(nowSec())}>
              Today
            </Button>
            <Button
              variant="outline"
              size="icon"
              aria-label="Next period"
              onClick={() => setAnchor((a) => a + step)}
            >
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
          <div className="flex items-center gap-1">
            {(["day", "week"] as View[]).map((v) => (
              <Button
                key={v}
                variant={view === v ? "primary" : "outline"}
                size="sm"
                onClick={() => setView(v)}
              >
                {v === "day" ? "Day" : "Week"}
              </Button>
            ))}
          </div>
          {loading && <Spinner className="h-4 w-4 text-muted" />}
        </div>
      </CardHeader>

      <CardContent>
        {error ? (
          <p className="py-8 text-center text-sm text-danger">{error}</p>
        ) : (
          <div
            className={
              view === "day"
                ? "grid grid-cols-1 gap-3"
                : "grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-7"
            }
          >
            {[...byDay.entries()].map(([dayStart, appts]) => (
              <DayColumn key={dayStart} dayStart={dayStart} appointments={appts} compact={view === "week"} />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function DayColumn({
  dayStart,
  appointments,
  compact,
}: {
  dayStart: number;
  appointments: CrmAppointment[];
  compact: boolean;
}) {
  const isToday = dayStart === startOfDay(nowSec());
  const sorted = [...appointments].sort((a, b) => a.startAt - b.startAt);
  return (
    <div
      className={`min-w-0 rounded-md border p-2 ${
        isToday ? "border-primary bg-primary/5" : "border-border"
      }`}
    >
      <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
        {new Date(dayStart * 1000).toLocaleDateString(undefined, {
          weekday: "short",
          day: "numeric",
          month: compact ? undefined : "short",
        })}
      </p>
      {sorted.length === 0 ? (
        <p className="py-3 text-center text-xs text-muted">No bookings</p>
      ) : (
        <ul className="space-y-2">
          {sorted.map((a) => (
            <li key={a.appointmentId}>
              <Link
                to={`/appointments/${a.appointmentId}`}
                className="block min-w-0 rounded-md border border-border bg-surface p-2 text-sm transition-colors hover:bg-surface-muted"
              >
                {/* A week column is too narrow for a full range without clipping the time itself,
                    which is the one thing that must stay legible — so week cells show the START
                    time and the day view shows the whole slot. */}
                <p className="font-medium text-onSurface">
                  {compact ? formatTime(a.startAt) : formatSlot(a.startAt, a.endAt)}
                </p>
                <p className="truncate text-muted">{a.clientName}</p>
                {a.petName && <p className="truncate text-xs text-muted">{a.petName}</p>}
                {a.service && <p className="truncate text-xs text-muted">{a.service}</p>}
                <span className="mt-1 inline-block">
                  <AppointmentStatusBadge status={a.status} />
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
