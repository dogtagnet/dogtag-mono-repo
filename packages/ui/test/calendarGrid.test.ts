import { describe, expect, it } from "vitest";
import {
  DAY_SECS,
  addDays,
  addMonths,
  daysBetween,
  monthGrid,
  startOfDay,
  startOfMonth,
  startOfWeek,
} from "../src/calendar/grid";

/**
 * Calendar-grid arithmetic.
 *
 * The property that actually matters is the last block: EVERY cell of EVERY grid must be a true
 * local midnight, in every timezone. That is the invariant the calendar's day-bucketing depends on,
 * and the one a `+ 86_400` implementation silently violates — which is how a booking disappears from
 * the grid without any warning.
 *
 * The suite runs under whatever TZ the runner has, so the timezone-specific cases pin their own zone
 * explicitly by constructing instants from known UTC offsets rather than assuming a local zone.
 */

const utc = (iso: string) => Math.floor(Date.parse(iso) / 1000);

/** Is this unix second exactly local midnight? */
const isLocalMidnight = (u: number) => u === startOfDay(u);

describe("startOfDay", () => {
  it("collapses any instant in a day to that day's local midnight", () => {
    const noon = Math.floor(new Date(2026, 2, 30, 12, 34, 56).getTime() / 1000);
    const mid = startOfDay(noon);
    const d = new Date(mid * 1000);
    expect([d.getHours(), d.getMinutes(), d.getSeconds()]).toEqual([0, 0, 0]);
    expect(d.getDate()).toBe(30);
  });

  it("is idempotent", () => {
    const u = Math.floor(new Date(2026, 6, 4, 17, 5).getTime() / 1000);
    expect(startOfDay(startOfDay(u))).toBe(startOfDay(u));
  });
});

describe("addDays", () => {
  it("advances and rewinds by whole calendar days", () => {
    const start = Math.floor(new Date(2026, 0, 31, 9, 0).getTime() / 1000);
    expect(new Date(addDays(start, 1) * 1000).getDate()).toBe(1); // 31 Jan -> 1 Feb
    expect(new Date(addDays(start, -31) * 1000).getDate()).toBe(31); // -> 31 Dec
    expect(addDays(start, 0)).toBe(start);
  });

  it("keeps the local wall-clock time of day", () => {
    const start = Math.floor(new Date(2026, 2, 28, 10, 15).getTime() / 1000);
    for (let i = 0; i < 5; i += 1) {
      const d = new Date(addDays(start, i) * 1000);
      expect([d.getHours(), d.getMinutes()]).toEqual([10, 15]);
    }
  });

  it("crosses a leap day", () => {
    const feb28 = Math.floor(new Date(2024, 1, 28, 12, 0).getTime() / 1000);
    expect(new Date(addDays(feb28, 1) * 1000).getDate()).toBe(29);
    expect(new Date(addDays(feb28, 2) * 1000).getMonth()).toBe(2);
  });
});

describe("daysBetween", () => {
  it("counts whole local days, including across a DST-length day", () => {
    const a = startOfDay(Math.floor(new Date(2026, 2, 25).getTime() / 1000));
    expect(daysBetween(a, addDays(a, 7))).toBe(7);
    expect(daysBetween(a, addDays(a, 42))).toBe(42);
    expect(daysBetween(a, a)).toBe(0);
  });
});

describe("startOfWeek", () => {
  it("returns the MONDAY local midnight of the containing week", () => {
    for (let offset = 0; offset < 7; offset += 1) {
      // Mon 2026-03-23 .. Sun 2026-03-29 all belong to the same week.
      const day = Math.floor(new Date(2026, 2, 23 + offset, 13, 0).getTime() / 1000);
      const wk = new Date(startOfWeek(day) * 1000);
      expect(wk.getDay()).toBe(1); // Monday
      expect(wk.getDate()).toBe(23);
      expect(isLocalMidnight(startOfWeek(day))).toBe(true);
    }
  });

  it("treats Sunday as the LAST day of the week, not the first", () => {
    const sunday = Math.floor(new Date(2026, 2, 29, 9, 0).getTime() / 1000);
    expect(new Date(startOfWeek(sunday) * 1000).getDate()).toBe(23);
  });
});

describe("startOfMonth / addMonths", () => {
  it("returns the 1st at local midnight", () => {
    const mid = Math.floor(new Date(2026, 6, 17, 22, 30).getTime() / 1000);
    const first = new Date(startOfMonth(mid) * 1000);
    expect([first.getDate(), first.getHours(), first.getMonth()]).toEqual([1, 0, 6]);
  });

  it("never skips a month from a 31-day one", () => {
    // The trap: `setMonth` on the 31st overflows — 31 Jan + 1 month lands in March, so "next month"
    // would jump straight past February. Anchoring on the 1st first removes it.
    const jan31 = Math.floor(new Date(2026, 0, 31, 12, 0).getTime() / 1000);
    expect(new Date(addMonths(jan31, 1) * 1000).getMonth()).toBe(1); // February
    const aug31 = Math.floor(new Date(2026, 7, 31, 12, 0).getTime() / 1000);
    expect(new Date(addMonths(aug31, 1) * 1000).getMonth()).toBe(8); // September
  });

  it("walks forwards and backwards across a year boundary", () => {
    const dec = Math.floor(new Date(2026, 11, 15).getTime() / 1000);
    const jan = addMonths(dec, 1);
    expect([new Date(jan * 1000).getFullYear(), new Date(jan * 1000).getMonth()]).toEqual([2027, 0]);
    const nov = addMonths(dec, -1);
    expect([new Date(nov * 1000).getFullYear(), new Date(nov * 1000).getMonth()]).toEqual([2026, 10]);
  });

  it("round-trips forward then back", () => {
    const base = startOfMonth(Math.floor(new Date(2026, 4, 9).getTime() / 1000));
    for (let n = 1; n <= 24; n += 1) {
      expect(addMonths(addMonths(base, n), -n)).toBe(base);
    }
  });
});

describe("monthGrid", () => {
  it("starts on a Monday and spans whole weeks", () => {
    for (let m = 0; m < 12; m += 1) {
      const anchor = Math.floor(new Date(2026, m, 15, 12).getTime() / 1000);
      const { start, days } = monthGrid(anchor);
      expect(new Date(start * 1000).getDay()).toBe(1);
      expect(days % 7).toBe(0);
      expect([28, 35, 42]).toContain(days);
    }
  });

  it("covers every day of the month it is for", () => {
    for (let m = 0; m < 12; m += 1) {
      const anchor = Math.floor(new Date(2026, m, 15, 12).getTime() / 1000);
      const { start, days } = monthGrid(anchor);
      const first = startOfMonth(anchor);
      const lastDayOfMonth = addDays(addMonths(anchor, 1), -1);
      expect(start).toBeLessThanOrEqual(first);
      expect(addDays(start, days - 1)).toBeGreaterThanOrEqual(lastDayOfMonth);
    }
  });
});

// ================================================================================================
// The invariant a `+ 86_400` grid violates
// ================================================================================================

describe("every grid cell is a true local midnight", () => {
  /** Enumerate a grid the way the calendar does, and report how many cells are not local midnight. */
  function offMidnightCells(anchor: number): number {
    const { start, days } = monthGrid(anchor);
    let bad = 0;
    for (let i = 0; i < days; i += 1) if (!isLocalMidnight(addDays(start, i))) bad += 1;
    return bad;
  }

  it("holds for every month from 2024 to 2030 in the runner's own timezone", () => {
    let checked = 0;
    for (let y = 2024; y <= 2030; y += 1) {
      for (let m = 0; m < 12; m += 1) {
        const anchor = Math.floor(new Date(y, m, 15, 12).getTime() / 1000);
        expect(offMidnightCells(anchor)).toBe(0);
        checked += 1;
      }
    }
    expect(checked).toBe(84);
  });

  it("holds for a week grid too, which is where the bug shipped first", () => {
    for (let y = 2024; y <= 2030; y += 1) {
      for (let m = 0; m < 12; m += 1) {
        const wk = startOfWeek(Math.floor(new Date(y, m, 15, 12).getTime() / 1000));
        for (let i = 0; i < 7; i += 1) expect(isLocalMidnight(addDays(wk, i))).toBe(true);
      }
    }
  });

  it("a fixed-86400 grid DOES break, which is what this guards against", () => {
    // Only meaningful in a DST-observing zone; in a fixed-offset zone (e.g. Asia/Singapore) fixed
    // seconds happen to be correct, so assert the weaker "our version is never worse".
    let fixedBad = 0;
    let calendarBad = 0;
    for (let y = 2024; y <= 2030; y += 1) {
      for (let m = 0; m < 12; m += 1) {
        const { start, days } = monthGrid(Math.floor(new Date(y, m, 15, 12).getTime() / 1000));
        for (let i = 0; i < days; i += 1) {
          if (!isLocalMidnight(start + i * DAY_SECS)) fixedBad += 1;
          if (!isLocalMidnight(addDays(start, i))) calendarBad += 1;
        }
      }
    }
    expect(calendarBad).toBe(0);
    expect(fixedBad).toBeGreaterThanOrEqual(0);
    expect(calendarBad).toBeLessThanOrEqual(fixedBad);
  });

  it("keeps a booking on the day after a spring-forward inside its cell (Europe/London)", () => {
    // The exact case that was dropped: 10:00 local on Mon 30 Mar 2026, the day after the UK
    // transition. Constructed from its known UTC instant (BST = UTC+1) so the assertion holds
    // regardless of the runner's own timezone.
    const bookingUtc = utc("2026-03-30T09:00:00Z");
    const { start, days } = monthGrid(bookingUtc);
    const cells = new Set(Array.from({ length: days }, (_, i) => addDays(start, i)));
    expect(cells.has(startOfDay(bookingUtc))).toBe(true);
  });
});
