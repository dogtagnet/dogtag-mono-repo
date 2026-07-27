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

/** Unix seconds of local midnight starting the MONDAY of the week containing `unixSec`. */
export function startOfWeek(unixSec: number): number {
  const d = new Date(startOfDay(unixSec) * 1000);
  // getDay(): 0=Sun..6=Sat. Shift so Monday is the first column.
  const shift = (d.getDay() + 6) % 7;
  return startOfDay(unixSec) - shift * DAY_SECS;
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
