/**
 * Local-calendar arithmetic for calendar GRIDS (day / week / month).
 *
 * WHY THIS IS NOT `+ 86_400`. A local day is not always 86400 seconds: on a DST spring-forward it is
 * 23 hours, on a fall-back 25. So enumerating a grid as `start + i * DAY_SECS` drifts off local
 * midnight from the transition onwards, while the bucket KEY for a booking is its true local
 * midnight (`startOfDay`). The two stop matching and the booking vanishes from the grid with no
 * warning — the worst kind of calendar bug, because the grid still looks complete.
 *
 * Measured, before the fix: in the March 2026 month grid for `Europe/London`, 7 of the 42 cells
 * landed at 01:00 instead of 00:00, and a booking at 10:00 on Mon 30 Mar matched no cell at all.
 *
 * Everything here therefore steps the `Date`'s own day/month component, which re-resolves the zone
 * offset for each new date — which is what a calendar means by "the next day". These live in the
 * shared package rather than one portal so day, week and month views cannot drift apart, and so the
 * property is covered by `packages/ui/test/calendarGrid.test.ts`.
 */

export const DAY_SECS = 86_400;

/** Unix seconds of local midnight starting the day that contains `unixSec`. */
export function startOfDay(unixSec: number): number {
  const d = new Date(unixSec * 1000);
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

/** Move `n` CALENDAR days, keeping the local wall-clock time of day. */
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
 * in March), which would make "next month" skip February. Month navigation only ever needs a month
 * START, so normalizing to the 1st removes the trap rather than working around it.
 */
export function addMonths(unixSec: number, n: number): number {
  const d = new Date(startOfMonth(unixSec) * 1000);
  d.setMonth(d.getMonth() + n);
  return Math.floor(d.getTime() / 1000);
}

/**
 * The Monday that starts the month grid containing `unixSec`, and how many days it spans — always
 * whole weeks, so the grid is a clean 7-column block (28, 35 or 42 cells).
 */
export function monthGrid(unixSec: number): { start: number; days: number } {
  const start = startOfWeek(startOfMonth(unixSec));
  const days = daysBetween(start, addMonths(unixSec, 1));
  return { start, days: Math.ceil(days / 7) * 7 };
}
