import { describe, expect, it } from "vitest";
import {
  parseContentLine,
  parseDuration,
  parseIcs,
  parseInstant,
  unescapeText,
  unfoldLines,
  zonedToUnix,
} from "../src/calendar/ics";

/**
 * `.ics` import parsing. The grammar is the easy half; what these lock down is the awkward half —
 * timezones, all-day events, recurrence and the shapes real exporters actually emit.
 */

/** Wrap VEVENT bodies in a minimal VCALENDAR, CRLF-framed as a real file is. */
function cal(...events: string[]): string {
  return [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//Test//EN",
    ...events,
    "END:VCALENDAR",
  ].join("\r\n");
}

function vevent(...lines: string[]): string {
  return ["BEGIN:VEVENT", ...lines, "END:VEVENT"].join("\r\n");
}

const utc = (iso: string) => Math.floor(Date.parse(iso) / 1000);

// ================================================================================================
// line handling
// ================================================================================================

describe("unfoldLines", () => {
  it("rejoins folded continuations (space and tab)", () => {
    // §3.1: unfolding removes the CRLF AND the single whitespace that follows it — the fold
    // character is framing, not content. A parser that keeps the space corrupts every long value.
    expect(unfoldLines("SUMMARY:hello\r\n world")).toEqual(["SUMMARY:helloworld"]);
    expect(unfoldLines("SUMMARY:hello\r\n\tworld")).toEqual(["SUMMARY:helloworld"]);
    // A SECOND leading space is content, and survives.
    expect(unfoldLines("SUMMARY:hello\r\n  world")).toEqual(["SUMMARY:hello world"]);
  });

  it("accepts bare-LF and bare-CR files, not only CRLF", () => {
    expect(unfoldLines("A:1\nB:2")).toEqual(["A:1", "B:2"]);
    expect(unfoldLines("A:1\rB:2")).toEqual(["A:1", "B:2"]);
  });

  it("strips a UTF-8 BOM so the first property name survives", () => {
    expect(unfoldLines("﻿BEGIN:VCALENDAR")).toEqual(["BEGIN:VCALENDAR"]);
  });

  it("drops blank lines rather than emitting empty properties", () => {
    expect(unfoldLines("A:1\r\n\r\n\r\nB:2")).toEqual(["A:1", "B:2"]);
  });
});

describe("parseContentLine", () => {
  it("splits name, parameters and value", () => {
    const cl = parseContentLine("DTSTART;TZID=Europe/London;VALUE=DATE-TIME:20260330T100000");
    expect(cl?.name).toBe("DTSTART");
    expect(cl?.params).toEqual({ TZID: "Europe/London", VALUE: "DATE-TIME" });
    expect(cl?.value).toBe("20260330T100000");
  });

  it("does not split on a colon inside a QUOTED parameter value", () => {
    // Outlook emits TZID="(UTC+00:00) Dublin, Edinburgh, Lisbon, London".
    const cl = parseContentLine('DTSTART;TZID="GMT+01:00":20260330T100000');
    expect(cl?.params.TZID).toBe("GMT+01:00");
    expect(cl?.value).toBe("20260330T100000");
  });

  it("uppercases the property name so case-varying files parse the same", () => {
    expect(parseContentLine("dtstart:20260330T100000Z")?.name).toBe("DTSTART");
  });

  it("returns null for a line with no colon at all", () => {
    expect(parseContentLine("GARBAGE")).toBeNull();
  });
});

describe("unescapeText", () => {
  it("reverses the RFC 5545 TEXT escapes", () => {
    expect(unescapeText("a\\;b\\,c")).toBe("a;b,c");
    expect(unescapeText("line1\\nline2")).toBe("line1\nline2");
    expect(unescapeText("back\\\\slash")).toBe("back\\slash");
    expect(unescapeText("\\N is also a newline")).toBe("\n is also a newline");
  });

  it("does not lose a trailing lone backslash", () => {
    expect(unescapeText("trailing\\")).toBe("trailing\\");
  });

  it("round-trips the writer's own escaping", () => {
    // The Rust feed writer escapes \ ; , and newline; this must undo exactly that.
    expect(unescapeText("Groom\\; wash\\, dry")).toBe("Groom; wash, dry");
  });
});

// ================================================================================================
// timezones — the reason this parser lives in the browser
// ================================================================================================

describe("zonedToUnix", () => {
  it("resolves a named zone exactly, on both sides of a DST transition", () => {
    // Europe/London springs forward 2026-03-29 01:00 UTC.
    expect(zonedToUnix(2026, 3, 28, 10, 0, 0, "Europe/London")).toBe(utc("2026-03-28T10:00:00Z"));
    // The day AFTER, the same wall clock is an hour earlier in UTC — this is the case a fixed
    // offset (or a hand-rolled VTIMEZONE reader) gets wrong.
    expect(zonedToUnix(2026, 3, 30, 10, 0, 0, "Europe/London")).toBe(utc("2026-03-30T09:00:00Z"));
  });

  it("resolves southern-hemisphere and half-hour zones", () => {
    // Australia/Adelaide is UTC+10:30 in January (ACDT).
    expect(zonedToUnix(2026, 1, 15, 9, 0, 0, "Australia/Adelaide")).toBe(
      utc("2026-01-14T22:30:00Z"),
    );
    // Asia/Kolkata is a fixed UTC+05:30 with no DST at all.
    expect(zonedToUnix(2026, 6, 1, 12, 0, 0, "Asia/Kolkata")).toBe(utc("2026-06-01T06:30:00Z"));
  });

  it("resolves a zone with no DST identically year-round", () => {
    expect(zonedToUnix(2026, 1, 1, 9, 0, 0, "Asia/Singapore")).toBe(utc("2026-01-01T01:00:00Z"));
    expect(zonedToUnix(2026, 7, 1, 9, 0, 0, "Asia/Singapore")).toBe(utc("2026-07-01T01:00:00Z"));
  });

  it("returns null for a zone this runtime cannot resolve", () => {
    // Reporting it is the point: booking a guess would put the appointment in the wrong hour.
    expect(zonedToUnix(2026, 3, 30, 10, 0, 0, "Mars/Olympus_Mons")).toBeNull();
    expect(zonedToUnix(2026, 3, 30, 10, 0, 0, "")).toBeNull();
  });
});

describe("parseInstant", () => {
  it("reads a UTC DATE-TIME exactly", () => {
    expect(parseInstant("20260330T100000Z", {})).toEqual({
      unix: utc("2026-03-30T10:00:00Z"),
      date: false,
    });
  });

  it("reads a TZID-qualified DATE-TIME through the IANA database", () => {
    expect(parseInstant("20260330T100000", { TZID: "Europe/London" })).toEqual({
      unix: utc("2026-03-30T09:00:00Z"),
      date: false,
    });
  });

  it("reads a VALUE=DATE as an all-day marker", () => {
    const r = parseInstant("20260330", { VALUE: "DATE" });
    expect(r?.date).toBe(true);
    // Local midnight, because a shop's "all day" is its own day, not UTC's.
    const d = new Date((r as { unix: number }).unix * 1000);
    expect([d.getFullYear(), d.getMonth() + 1, d.getDate()]).toEqual([2026, 3, 30]);
    expect([d.getHours(), d.getMinutes()]).toEqual([0, 0]);
  });

  it("treats a bare 8-digit value as a DATE even without VALUE=DATE", () => {
    expect(parseInstant("20260330", {})?.date).toBe(true);
  });

  it("reads a floating DATE-TIME as local wall time", () => {
    const r = parseInstant("20260330T100000", {});
    expect(r?.date).toBe(false);
    const d = new Date((r as { unix: number }).unix * 1000);
    expect([d.getHours(), d.getMinutes()]).toEqual([10, 0]);
  });

  it("returns null rather than guessing at an unresolvable value", () => {
    expect(parseInstant("not-a-date", {})).toBeNull();
    expect(parseInstant("20260330T100000", { TZID: "Nowhere/Nothing" })).toBeNull();
    expect(parseInstant("20261330T100000Z", {})).not.toBeNull(); // month 13 rolls, but parses
  });
});

describe("parseDuration", () => {
  it("reads the common forms", () => {
    expect(parseDuration("PT1H")).toBe(3600);
    expect(parseDuration("PT30M")).toBe(1800);
    expect(parseDuration("PT1H30M")).toBe(5400);
    expect(parseDuration("P1D")).toBe(86_400);
    expect(parseDuration("P1W")).toBe(7 * 86_400);
    expect(parseDuration("PT45S")).toBe(45);
  });

  it("reads a signed duration", () => {
    expect(parseDuration("-PT1H")).toBe(-3600);
  });

  it("returns null for something that is not a duration", () => {
    expect(parseDuration("1 hour")).toBeNull();
    expect(parseDuration("")).toBeNull();
  });
});

// ================================================================================================
// whole-file parsing
// ================================================================================================

describe("parseIcs", () => {
  it("says plainly when the payload is not an iCalendar file", () => {
    const r = parseIcs("this is a text file, not a calendar");
    expect(r.notACalendar).toBe(true);
    expect(r.events).toEqual([]);
  });

  it("parses a real-shaped event", () => {
    const r = parseIcs(
      cal(
        vevent(
          "UID:evt-1@google.com",
          "DTSTAMP:20260201T090000Z",
          "DTSTART:20260330T100000Z",
          "DTEND:20260330T113000Z",
          "SUMMARY:Full groom - Rex",
          "DESCRIPTION:bring the muzzle",
          "LOCATION:Paws & Claws",
          "STATUS:CONFIRMED",
        ),
      ),
    );
    expect(r.skipped).toEqual([]);
    expect(r.events).toHaveLength(1);
    expect(r.events[0]).toMatchObject({
      uid: "evt-1@google.com",
      summary: "Full groom - Rex",
      description: "bring the muzzle",
      location: "Paws & Claws",
      startAt: utc("2026-03-30T10:00:00Z"),
      endAt: utc("2026-03-30T11:30:00Z"),
      allDay: false,
      recurring: false,
      status: "confirmed",
    });
  });

  it("reads the calendar's own name when it has one", () => {
    const withName = cal("X-WR-CALNAME:Paws & Claws bookings", vevent("UID:a", "DTSTART:20260330T100000Z"));
    expect(parseIcs(withName).calendarName).toBe("Paws & Claws bookings");
  });

  it("derives the end from DURATION when there is no DTEND", () => {
    const r = parseIcs(
      cal(vevent("UID:a", "DTSTART:20260330T100000Z", "DURATION:PT90M", "SUMMARY:Bath")),
    );
    expect(r.events[0].endAt).toBe(utc("2026-03-30T11:30:00Z"));
  });

  it("leaves endAt at 0 when the file gives no end at all", () => {
    // The backend applies the portal's own one-hour default; the parser does not invent one.
    const r = parseIcs(cal(vevent("UID:a", "DTSTART:20260330T100000Z", "SUMMARY:Nail trim")));
    expect(r.events[0].endAt).toBe(0);
  });

  it("unfolds a folded SUMMARY back into one value", () => {
    const folded = [
      "BEGIN:VCALENDAR",
      "BEGIN:VEVENT",
      "UID:a",
      "DTSTART:20260330T100000Z",
      "SUMMARY:A very long service description that the exporter",
      "  wrapped across two physical lines",
      "END:VEVENT",
      "END:VCALENDAR",
    ].join("\r\n");
    expect(parseIcs(folded).events[0].summary).toBe(
      "A very long service description that the exporter wrapped across two physical lines",
    );
  });

  // ---- all-day ----

  it("marks an all-day event and spans exactly its own local day", () => {
    const r = parseIcs(
      cal(vevent("UID:a", "DTSTART;VALUE=DATE:20260330", "DTEND;VALUE=DATE:20260331", "SUMMARY:Closed")),
    );
    const e = r.events[0];
    expect(e.allDay).toBe(true);
    expect(r.allDay).toBe(1);
    const start = new Date(e.startAt * 1000);
    const end = new Date(e.endAt * 1000);
    expect([start.getHours(), end.getHours()]).toEqual([0, 0]);
    expect(end.getDate()).toBe(31);
  });

  it("gives an all-day event with no DTEND exactly one CALENDAR day", () => {
    // Not +86400: a DST day is 23h or 25h, and adding fixed seconds would end at 23:00 or 01:00.
    const r = parseIcs(cal(vevent("UID:a", "DTSTART;VALUE=DATE:20260329", "SUMMARY:Closed")));
    const end = new Date(r.events[0].endAt * 1000);
    expect([end.getHours(), end.getMinutes()]).toEqual([0, 0]);
    expect(end.getDate()).toBe(30);
  });

  // ---- recurrence ----

  it("flags a recurring event instead of silently importing one occurrence", () => {
    const r = parseIcs(
      cal(
        vevent(
          "UID:weekly@old.example",
          "DTSTART:20260330T100000Z",
          "RRULE:FREQ=WEEKLY;BYDAY=MO",
          "SUMMARY:Standing groom",
        ),
      ),
    );
    expect(r.events).toHaveLength(1);
    expect(r.events[0].recurring).toBe(true);
    expect(r.recurring).toBe(1);
    // The single occurrence it DOES yield is the DTSTART one, not a guess at the next.
    expect(r.events[0].startAt).toBe(utc("2026-03-30T10:00:00Z"));
  });

  // ---- skipping, honestly ----

  it("skips an event with no DTSTART and says why", () => {
    const r = parseIcs(cal(vevent("UID:no-start@old.example", "SUMMARY:Someday")));
    expect(r.events).toEqual([]);
    expect(r.skipped).toEqual([
      { label: "no-start@old.example", reason: expect.stringContaining("no DTSTART") },
    ]);
  });

  it("skips an event whose TZID this runtime cannot resolve, naming the zone", () => {
    const r = parseIcs(
      cal(vevent("UID:bad-tz", "DTSTART;TZID=Nowhere/Nothing:20260330T100000", "SUMMARY:x")),
    );
    expect(r.events).toEqual([]);
    expect(r.skipped[0].reason).toContain("Nowhere/Nothing");
  });

  it("falls back to the SUMMARY as the label when there is no UID to name", () => {
    const r = parseIcs(cal(vevent("SUMMARY:Mystery booking", "DTSTART:garbage")));
    expect(r.skipped[0].label).toBe("Mystery booking");
  });

  it("skips a pre-1970 start rather than sending an instant no booking can hold", () => {
    // Ordinary in a real export — a birthday or anniversary — and it resolves NEGATIVE. Sent on, it
    // is a number the import must reject, and rejecting it there costs the WHOLE file.
    const r = parseIcs(
      cal(vevent("UID:birthday@old.example", "DTSTART:19420704T090000Z", "SUMMARY:Born")),
    );
    expect(r.events).toEqual([]);
    expect(r.skipped[0].label).toBe("birthday@old.example");
    expect(r.skipped[0].reason).toContain("1970");
  });

  it("skips a year the runtime cannot represent as a real instant", () => {
    // `00010101` is well-formed to the grammar and maps to no usable instant.
    const r = parseIcs(cal(vevent("UID:ancient", "DTSTART;VALUE=DATE:00010101", "SUMMARY:x")));
    expect(r.events).toEqual([]);
    expect(r.skipped).toHaveLength(1);
  });

  it("keeps a good event when a sibling in the same file is unstorable", () => {
    const r = parseIcs(
      cal(
        vevent("UID:birthday@old.example", "DTSTART:19420704T090000Z", "SUMMARY:Born"),
        vevent("UID:good@old.example", "DTSTART:20260330T100000Z", "SUMMARY:Full groom"),
      ),
    );
    expect(r.events).toHaveLength(1);
    expect(r.events[0].uid).toBe("good@old.example");
    expect(r.events[0].startAt).toBeGreaterThan(0);
    expect(r.skipped).toHaveLength(1);
  });

  it("never emits a start or end the import would have to refuse", () => {
    const r = parseIcs(
      cal(
        vevent("UID:a", "DTSTART:20260330T100000Z", "DTEND:19600101T000000Z"),
        vevent("UID:b", "DTSTART:20260330T100000Z", "DTEND:20260330T113000Z"),
      ),
    );
    for (const e of r.events) {
      expect(Number.isFinite(e.startAt) && e.startAt > 0).toBe(true);
      expect(Number.isFinite(e.endAt) && e.endAt >= 0).toBe(true);
    }
    // A pre-epoch DTEND is no end at all; the import applies its own default slot length.
    expect(r.events[0].endAt).toBe(0);
  });

  // ---- identity ----

  it("synthesizes a STABLE uid for an event the file left unidentified", () => {
    const file = cal(vevent("DTSTART:20260330T100000Z", "SUMMARY:Walk-in"));
    const a = parseIcs(file).events[0].uid;
    const b = parseIcs(file).events[0].uid;
    expect(a).toBe(b); // re-importing the same file must still dedup
    expect(a).toContain("@dogtag.local");
    // A different event gets a different id.
    const other = parseIcs(cal(vevent("DTSTART:20260330T100000Z", "SUMMARY:Different"))).events[0];
    expect(other.uid).not.toBe(a);
  });

  // ---- component nesting ----

  it("ignores DTSTART inside a VTIMEZONE, using the event's own", () => {
    // Every real Google/Apple export embeds a VTIMEZONE whose STANDARD/DAYLIGHT blocks carry their
    // own DTSTART. Reading one of those as the booking time would be badly wrong.
    const file = [
      "BEGIN:VCALENDAR",
      "BEGIN:VTIMEZONE",
      "TZID:Europe/London",
      "BEGIN:DAYLIGHT",
      "DTSTART:19700329T010000",
      "TZOFFSETFROM:+0000",
      "TZOFFSETTO:+0100",
      "END:DAYLIGHT",
      "END:VTIMEZONE",
      vevent("UID:a", "DTSTART:20260330T100000Z", "SUMMARY:Real booking"),
      "END:VCALENDAR",
    ].join("\r\n");
    const r = parseIcs(file);
    expect(r.events).toHaveLength(1);
    expect(r.events[0].startAt).toBe(utc("2026-03-30T10:00:00Z"));
  });

  it("ignores a VALARM's own properties", () => {
    const r = parseIcs(
      cal(
        [
          "BEGIN:VEVENT",
          "UID:a",
          "DTSTART:20260330T100000Z",
          "SUMMARY:Real booking",
          "BEGIN:VALARM",
          "TRIGGER:-PT15M",
          "DESCRIPTION:Reminder",
          "END:VALARM",
          "END:VEVENT",
        ].join("\r\n"),
      ),
    );
    expect(r.events[0].summary).toBe("Real booking");
    expect(r.events[0].description).toBe("");
  });

  it("parses several events and keeps them in file order", () => {
    const r = parseIcs(
      cal(
        vevent("UID:a", "DTSTART:20260330T100000Z", "SUMMARY:First"),
        vevent("UID:b", "DTSTART:20260331T100000Z", "SUMMARY:Second"),
      ),
    );
    expect(r.events.map((e) => e.uid)).toEqual(["a", "b"]);
  });

  it("carries a cancellation through so the import can act on it", () => {
    const r = parseIcs(
      cal(vevent("UID:a", "DTSTART:20260330T100000Z", "STATUS:CANCELLED", "SUMMARY:Dropped")),
    );
    expect(r.events[0].status).toBe("cancelled");
  });

  it("returns an empty result for a calendar with no events at all", () => {
    // An empty calendar imports nothing — it never invents a sample booking.
    const r = parseIcs(cal());
    expect(r.events).toEqual([]);
    expect(r.skipped).toEqual([]);
    expect(r.notACalendar).toBe(false);
  });
});
