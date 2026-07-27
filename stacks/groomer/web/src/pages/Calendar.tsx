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
import {
  CalendarDays,
  CalendarPlus,
  ChevronLeft,
  ChevronRight,
  RefreshCw,
} from "lucide-react";
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { AppointmentStatusBadge, useList } from "../app/crm";
import {
  addDays,
  addMonths,
  formatDate,
  formatMonth,
  formatSlot,
  formatTime,
  monthGrid,
  nowSec,
  startOfDay,
  startOfMonth,
  startOfWeek,
} from "../lib/time";

type View = "day" | "week" | "month";

const VIEWS: { key: View; label: string }[] = [
  { key: "day", label: "Day" },
  { key: "week", label: "Week" },
  { key: "month", label: "Month" },
];

/** How many bookings a month cell lists before it collapses the rest into a "+N more" link. */
const MONTH_CELL_LIMIT = 3;

/** The visible window for a view, as a local-midnight start plus a whole number of local days. */
function windowFor(view: View, anchor: number): { from: number; days: number } {
  if (view === "day") return { from: startOfDay(anchor), days: 1 };
  if (view === "week") return { from: startOfWeek(anchor), days: 7 };
  const { start, days } = monthGrid(anchor);
  return { from: start, days };
}

/**
 * The groomer's working surface: a day, week or MONTH grid of bookings.
 *
 * Month is the view that answers "what does my work look like" — a day or a list answers a different
 * question — so it shares the day/week grid's interaction exactly: the same range query, the same
 * navigation, and a booking is still a link straight to its appointment.
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

  const { from, days } = windowFor(view, anchor);
  // Both ends are derived by CALENDAR arithmetic, never `from + days * 86_400`: a local day is 23h
  // or 25h across a DST transition, and a fixed-seconds window would clip or double-count the
  // bookings on the boundary day. See `addDays` in lib/time.
  const to = addDays(from, days);

  const { page, loading, error } = useList<CrmAppointment>(
    // MAX_PAGE-sized: a month of one shop's bookings comfortably fits one page, and the pager on the
    // Appointments list is the right surface for anything beyond that.
    () => api.listAppointments({ from, to, limit: 200 }),
    [from, to],
  );

  // The window is capped at one page, so a very busy range can hold more bookings than the grid
  // renders. A calendar that quietly drops a booking is worse than one that admits it did — say so
  // rather than letting the grid look complete.
  const shown = page?.rows.length ?? 0;
  const total = page?.total ?? 0;
  const hidden = Math.max(0, total - shown);

  // bucket the window's appointments by local day so each cell renders in one pass
  const byDay = useMemo(() => {
    const buckets = new Map<number, CrmAppointment[]>();
    for (let i = 0; i < days; i += 1) buckets.set(addDays(from, i), []);
    for (const a of page?.rows ?? []) {
      // Both sides of this lookup are true local midnights, which is exactly why the cells are
      // enumerated with `addDays` — a DST-drifted key would silently match nothing.
      const bucket = buckets.get(startOfDay(a.startAt));
      if (bucket) bucket.push(a);
    }
    return buckets;
  }, [page, from, days]);

  function step(direction: 1 | -1) {
    setAnchor((a) =>
      view === "month"
        ? addMonths(a, direction)
        : addDays(a, direction * (view === "day" ? 1 : 7)),
    );
  }

  const rangeLabel =
    view === "day"
      ? formatDate(from)
      : view === "week"
        ? `${formatDate(from)} – ${formatDate(addDays(from, days - 1))}`
        : formatMonth(startOfMonth(anchor));

  return (
    <Card>
      <CardHeader className="space-y-4">
        <div className="flex flex-row flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="flex items-center gap-2">
              <CalendarDays className="h-5 w-5 text-primary" /> Calendar
            </CardTitle>
            <CardDescription>{rangeLabel}</CardDescription>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" asChild>
              <Link to="/calendar/sync">
                <RefreshCw className="h-4 w-4" /> Calendar sync
              </Link>
            </Button>
            <Button onClick={() => navigate("/appointments/new")}>
              <CalendarPlus className="h-4 w-4" /> New appointment
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1">
            <Button
              variant="outline"
              size="icon"
              aria-label={view === "month" ? "Previous month" : "Previous period"}
              onClick={() => step(-1)}
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <Button variant="outline" size="sm" onClick={() => setAnchor(nowSec())}>
              Today
            </Button>
            <Button
              variant="outline"
              size="icon"
              aria-label={view === "month" ? "Next month" : "Next period"}
              onClick={() => step(1)}
            >
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
          <div className="flex items-center gap-1">
            {VIEWS.map((v) => (
              <Button
                key={v.key}
                variant={view === v.key ? "primary" : "outline"}
                size="sm"
                aria-pressed={view === v.key}
                onClick={() => setView(v.key)}
              >
                {v.label}
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
          <>
            {hidden > 0 && (
              <p
                role="status"
                className="mb-3 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning"
              >
                Showing {shown} of {total} bookings in this range — {hidden} more{" "}
                {hidden === 1 ? "is" : "are"} not on this grid.{" "}
                {view === "month"
                  ? "Switch to Week or Day view to see them all."
                  : "Switch to Day view or narrow the range to see them all."}
              </p>
            )}
            {view === "month" ? (
              <MonthGrid
                byDay={byDay}
                month={startOfMonth(anchor)}
                onOpenDay={(dayStart) => {
                  setAnchor(dayStart);
                  setView("day");
                }}
              />
            ) : (
              <div
                className={
                  view === "day"
                    ? "grid grid-cols-1 gap-3"
                    : "grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-7"
                }
              >
                {[...byDay.entries()].map(([dayStart, appts]) => (
                  <DayColumn
                    key={dayStart}
                    dayStart={dayStart}
                    appointments={appts}
                    compact={view === "week"}
                  />
                ))}
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/** Monday-first weekday headings, localized. */
function weekdayLabels(gridStart: number): string[] {
  return Array.from({ length: 7 }, (_, i) =>
    new Date(addDays(gridStart, i) * 1000).toLocaleDateString(undefined, { weekday: "short" }),
  );
}

/**
 * The month grid: seven columns of whole weeks, leading/trailing days from the neighbouring months
 * dimmed so the shape of the month reads at a glance.
 *
 * Cells scroll horizontally as one block below `sm` rather than reflowing: a month IS a 7-column
 * shape, and stacking it into a list would turn it back into the view it exists to be different from.
 */
function MonthGrid({
  byDay,
  month,
  onOpenDay,
}: {
  byDay: Map<number, CrmAppointment[]>;
  month: number;
  onOpenDay: (dayStart: number) => void;
}) {
  const cells = [...byDay.entries()];
  const gridStart = cells[0]?.[0] ?? month;
  const monthIndex = new Date(month * 1000).getMonth();

  return (
    <div className="overflow-x-auto">
      <div className="min-w-[42rem]">
        <div className="grid grid-cols-7 gap-px">
          {weekdayLabels(gridStart).map((label) => (
            <div
              key={label}
              className="pb-1 text-center text-xs font-semibold uppercase tracking-wide text-muted"
            >
              {label}
            </div>
          ))}
        </div>
        <div className="grid grid-cols-7 gap-2">
          {cells.map(([dayStart, appts]) => (
            <MonthCell
              key={dayStart}
              dayStart={dayStart}
              appointments={appts}
              inMonth={new Date(dayStart * 1000).getMonth() === monthIndex}
              onOpenDay={onOpenDay}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function MonthCell({
  dayStart,
  appointments,
  inMonth,
  onOpenDay,
}: {
  dayStart: number;
  appointments: CrmAppointment[];
  inMonth: boolean;
  onOpenDay: (dayStart: number) => void;
}) {
  const isToday = dayStart === startOfDay(nowSec());
  const sorted = [...appointments].sort((a, b) => a.startAt - b.startAt);
  const visible = sorted.slice(0, MONTH_CELL_LIMIT);
  const overflow = sorted.length - visible.length;

  return (
    <div
      className={`flex min-h-24 min-w-0 flex-col rounded-md border p-1.5 ${
        isToday
          ? "border-primary bg-primary/5"
          : inMonth
            ? "border-border"
            : "border-border/50 bg-surface-muted/40"
      }`}
    >
      <button
        type="button"
        onClick={() => onOpenDay(dayStart)}
        title="Open this day"
        className={`mb-1 self-start rounded px-1 text-xs font-semibold transition-colors hover:bg-surface-muted ${
          isToday ? "text-primary" : inMonth ? "text-onSurface" : "text-muted"
        }`}
      >
        {new Date(dayStart * 1000).getDate()}
      </button>
      <ul className="space-y-1">
        {visible.map((a) => (
          <li key={a.appointmentId}>
            <Link
              to={`/appointments/${a.appointmentId}`}
              title={`${formatSlot(a.startAt, a.endAt)} · ${a.clientName || "Unassigned"}${
                a.service ? ` · ${a.service}` : ""
              }`}
              className="block min-w-0 truncate rounded border border-border bg-surface px-1.5 py-0.5 text-xs transition-colors hover:bg-surface-muted"
            >
              <span className="font-medium text-onSurface">{formatTime(a.startAt)}</span>{" "}
              <span className="text-muted">{a.clientName || a.service || "Unassigned"}</span>
            </Link>
          </li>
        ))}
      </ul>
      {overflow > 0 && (
        <button
          type="button"
          onClick={() => onOpenDay(dayStart)}
          className="mt-1 self-start rounded px-1 text-xs text-primary hover:underline"
        >
          +{overflow} more
        </button>
      )}
    </div>
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
                <p className="truncate text-muted">{a.clientName || "Unassigned"}</p>
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
