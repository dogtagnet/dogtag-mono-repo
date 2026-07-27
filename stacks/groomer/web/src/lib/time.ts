/**
 * Time helpers for the booking surfaces.
 *
 * Appointments are stored as UNIX SECONDS (the backend range-queries `startAt` for the calendar), so
 * everything here converts between that and the operator's LOCAL wall clock — a groomer books "10am
 * Tuesday" in the shop's timezone, never in UTC.
 */

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
export const DAY_SECS = 86_400;

/** Unix seconds of local midnight starting the day that contains `unixSec`. */
export function startOfDay(unixSec: number): number {
  const d = new Date(unixSec * 1000);
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

/**
 * Move `n` CALENDAR days, keeping the local wall-clock time of day.
 *
 * This is the reason the calendar grids do not step by `86_400`. A local day is not always 86400
 * seconds: on a DST spring-forward it is 23 hours, on a fall-back it is 25. Enumerating a grid as
 * `start + i * DAY_SECS` therefore drifts off local midnight from the transition onwards — in the
 * March 2026 grid for Europe/London, 7 of the 42 cells land at 01:00 instead of 00:00, so a booking
 * bucketed under its true local midnight matches NO cell and disappears from the calendar with no
 * warning at all. Stepping the Date's day component instead re-resolves the offset per day, which is
 * what a calendar means by "the next day".
 */
export function addDays(unixSec: number, n: number): number {
  const d = new Date(unixSec * 1000);
  d.setDate(d.getDate() + n);
  return Math.floor(d.getTime() / 1000);
}

/** Whole local days between two local midnights. Rounded, because a DST day is 23h or 25h. */
export function daysBetween(fromSec: number, toSec: number): number {
  return Math.round((toSec - fromSec) / DAY_SECS);
}

/** Unix seconds of local midnight starting the MONDAY of the week containing `unixSec`. */
export function startOfWeek(unixSec: number): number {
  const start = startOfDay(unixSec);
  // getDay(): 0=Sun..6=Sat. Shift so Monday is the first column.
  const shift = (new Date(start * 1000).getDay() + 6) % 7;
  return addDays(start, -shift);
}

/** Unix seconds of local midnight on the 1st of the month containing `unixSec`. */
export function startOfMonth(unixSec: number): number {
  const d = new Date(unixSec * 1000);
  d.setDate(1);
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

/**
 * Move `n` calendar months, anchored on the 1st.
 *
 * Anchoring first is what makes this total: `setMonth` on the 31st overflows (31 Jan + 1 month lands
 * in March), which would make "next month" skip February entirely. Month navigation only ever needs
 * a month START, so normalizing to the 1st removes the trap rather than working around it.
 */
export function addMonths(unixSec: number, n: number): number {
  const d = new Date(startOfMonth(unixSec) * 1000);
  d.setMonth(d.getMonth() + n);
  return Math.floor(d.getTime() / 1000);
}

/**
 * The Monday that starts the month grid containing `unixSec`, and how many days the grid spans —
 * always whole weeks, so the grid is a clean 7-column block (28, 35 or 42 cells).
 */
export function monthGrid(unixSec: number): { start: number; days: number } {
  const start = startOfWeek(startOfMonth(unixSec));
  const days = daysBetween(start, addMonths(unixSec, 1));
  return { start, days: Math.ceil(days / 7) * 7 };
}

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
