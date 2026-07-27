/**
 * Time helpers for the booking surfaces.
 *
 * Appointments are stored as UNIX SECONDS (the backend range-queries `startAt` for the calendar), so
 * everything here converts between that and the operator's LOCAL wall clock — a groomer books "10am
 * Tuesday" in the shop's timezone, never in UTC.
 */

import { startOfDay } from "@dogtag/ui";

const pad = (n: number) => String(n).padStart(2, "0");

/** Unix seconds -> the `YYYY-MM-DDTHH:mm` value an `<input type="datetime-local">` expects (local). */
export function toDateTimeInput(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(
    d.getMinutes(),
  )}`;
}

/** `YYYY-MM-DDTHH:mm` (local) -> unix seconds. Returns 0 for an empty/unparseable value. */
export function fromDateTimeInput(value: string): number {
  if (!value) return 0;
  const ms = new Date(value).getTime();
  return Number.isNaN(ms) ? 0 : Math.floor(ms / 1000);
}

/** Unix seconds -> `YYYY-MM-DD` (local), the value a `<input type="date">` expects. */
export function toDateInput(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** `YYYY-MM-DD` (local) -> the unix seconds of that day's 00:00 local. */
export function startOfDayFromInput(value: string): number {
  const [y, m, d] = value.split("-").map(Number);
  if (!y || !m || !d) return startOfDay(nowSec());
  return Math.floor(new Date(y, m - 1, d, 0, 0, 0, 0).getTime() / 1000);
}

export const nowSec = () => Math.floor(Date.now() / 1000);

/**
 * Calendar-GRID arithmetic lives in `@dogtag/ui` and is re-exported here so every existing call site
 * keeps its import.
 *
 * It is shared rather than app-local for one reason: none of it may step by a fixed `86_400`. A
 * local day is 23h or 25h across a DST transition, so fixed-seconds enumeration drifts off local
 * midnight and silently drops bookings out of the grid. Keeping the day/week/month math in one
 * tested place is what stops the three views drifting apart. See `packages/ui/src/calendar/grid.ts`
 * and its property tests in `packages/ui/test/calendarGrid.test.ts`.
 */
export {
  DAY_SECS,
  addDays,
  addMonths,
  daysBetween,
  monthGrid,
  startOfDay,
  startOfMonth,
  startOfWeek,
} from "@dogtag/ui";

export function formatTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatDate(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

export function formatDateTime(unixSec: number): string {
  if (!unixSec) return "—";
  return `${formatDate(unixSec)}, ${formatTime(unixSec)}`;
}

/** "10:00 – 11:00" for a slot. */
export function formatSlot(startAt: number, endAt: number): string {
  return endAt > startAt ? `${formatTime(startAt)} – ${formatTime(endAt)}` : formatTime(startAt);
}

/** "March 2026" — the month view's range label. */
export function formatMonth(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
}
