import { describe, expect, it } from "vitest";
import { parseIcs } from "../src/calendar/ics";

/**
 * The per-appointment CLIENT handoff `.ics`, read back by this repo's own parser.
 *
 * The document is WRITTEN in Rust (`stacks/vet/api/src/appointment_share.rs`, through the shared
 * serializer in `stacks/vet/api/src/ics.rs`) and that side's tests assert what it emits. What a
 * same-language string assertion cannot check is whether a real PARSER agrees: a grammar mistake in
 * the folding, the escaping, or the newly-added `URL`/`LOCATION`/`SEQUENCE` properties would read
 * perfectly as `assert!(out.contains(...))` and still deliver a corrupted event to the phone that
 * opens it.
 *
 * So the fixtures below are the writer's EXACT bytes — captured from its own output, fold points and
 * all, not hand-typed — and this reads them with the same parser the shop's `.ics` IMPORT uses. They
 * are the contract between the two halves: a change on the Rust side that would break a client's
 * calendar surfaces here as a failing parse rather than as a support ticket.
 */

/**
 * Verbatim `appointment_share::ics_document` output for a LIVE booking, 2026-02-25 12:00–13:00 UTC.
 *
 * The `DESCRIPTION` is folded across four physical lines at the 75-octet limit; the leading space on
 * each continuation is the RFC 5545 §3.1 fold marker, and the SECOND space on the first continuation
 * is a real space in the sentence. Both must survive unfolding exactly, or the client reads a
 * calendar entry with words jammed together.
 */
const LIVE_HANDOFF = [
  "BEGIN:VCALENDAR",
  "VERSION:2.0",
  "PRODID:-//DogTag//Pampered Paws//EN",
  "CALSCALE:GREGORIAN",
  "METHOD:PUBLISH",
  "X-WR-CALNAME:Full groom - Rex",
  "REFRESH-INTERVAL;VALUE=DURATION:PT15M",
  "X-PUBLISHED-TTL:PT15M",
  "BEGIN:VEVENT",
  "UID:appt-1@groomer.example",
  "DTSTAMP:20260225T061320Z",
  "DTSTART:20260225T120000Z",
  "DTEND:20260225T130000Z",
  "SUMMARY:Full groom - Rex",
  "DESCRIPTION:With Pampered Paws.\\nStatus: Scheduled\\nYour groomer: Sam\\nThis",
  "  is a copy of the booking as it stood when you added it. If the shop chang",
  " es or cancels it\\, your calendar will not update on its own.\\nCheck the cu",
  " rrent details: https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef",
  "LOCATION:Pampered Paws",
  "URL:https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef",
  "STATUS:CONFIRMED",
  "SEQUENCE:3600",
  "LAST-MODIFIED:20260225T071320Z",
  "END:VEVENT",
  "END:VCALENDAR",
  "",
].join("\r\n");

/** Verbatim output for a booking the shop DELETED — the tombstone that clears the client's copy. */
const CANCELLED_HANDOFF = [
  "BEGIN:VCALENDAR",
  "VERSION:2.0",
  "PRODID:-//DogTag//Pampered Paws//EN",
  "CALSCALE:GREGORIAN",
  "METHOD:PUBLISH",
  "X-WR-CALNAME:Cancelled appointment",
  "REFRESH-INTERVAL;VALUE=DURATION:PT15M",
  "X-PUBLISHED-TTL:PT15M",
  "BEGIN:VEVENT",
  "UID:appt-1@groomer.example",
  "DTSTAMP:20260225T061320Z",
  "DTSTART:20260225T120000Z",
  "SUMMARY:Cancelled appointment",
  "DESCRIPTION:This appointment is no longer on Pampered Paws's calendar.",
  "LOCATION:Pampered Paws",
  "STATUS:CANCELLED",
  "SEQUENCE:2147483647",
  "END:VEVENT",
  "END:VCALENDAR",
  "",
].join("\r\n");

describe("the client handoff .ics, read back by this repo's parser", () => {
  it("parses as a calendar holding exactly the one booking", () => {
    const r = parseIcs(LIVE_HANDOFF);
    expect(r.notACalendar).toBe(false);
    expect(r.skipped).toEqual([]);
    expect(r.events).toHaveLength(1);
    expect(r.calendarName).toBe("Full groom - Rex");
  });

  it("round-trips the exact instants the shop booked", () => {
    // If the writer's DTSTART/DTEND grammar were wrong, this is where a client's calendar would
    // silently land on the wrong hour.
    const [e] = parseIcs(LIVE_HANDOFF).events;
    expect(e.startAt).toBe(Math.floor(Date.parse("2026-02-25T12:00:00Z") / 1000));
    expect(e.endAt).toBe(Math.floor(Date.parse("2026-02-25T13:00:00Z") / 1000));
    expect(e.allDay).toBe(false);
    expect(e.recurring).toBe(false);
  });

  it("carries a stable UID, which is what lets a re-add supersede rather than duplicate", () => {
    expect(parseIcs(LIVE_HANDOFF).events[0].uid).toBe("appt-1@groomer.example");
    // ...and the tombstone reuses it, or it would cancel nothing.
    expect(parseIcs(CANCELLED_HANDOFF).events[0].uid).toBe("appt-1@groomer.example");
  });

  it("unfolds the long description back to a readable sentence, caveat intact", () => {
    const [e] = parseIcs(LIVE_HANDOFF).events;
    expect(e.description).toContain("With Pampered Paws.");
    expect(e.description).toContain("Your groomer: Sam");
    // The fold fell inside both "This| is" and "chang|es". Asserting the whole sentence is what
    // catches an unfolder that drops or keeps the wrong space.
    expect(e.description).toContain(
      "This is a copy of the booking as it stood when you added it.",
    );
    expect(e.description).toContain("If the shop changes or cancels it, your calendar");
    expect(e.description).toContain(
      "https://shop.example/a/deadbeefdeadbeefdeadbeefdeadbeef",
    );
    // no fold artefacts left behind
    expect(e.description).not.toContain("\r");
    expect(e.description).not.toMatch(/chang\s+es/);
    expect(e.description).not.toMatch(/This\s{2,}is/);
  });

  it("carries the shop as the location and nothing of the shop's own book", () => {
    const r = parseIcs(LIVE_HANDOFF);
    expect(r.events[0].location).toBe("Pampered Paws");
    // The two fields the SHOP's feed publishes and the client handoff must not. The source booking
    // these bytes were generated from carried both.
    const whole = JSON.stringify(r);
    expect(whole).not.toContain("muzzle");
    expect(whole).not.toContain("Alice");
  });

  it("reads a cancelled booking as cancelled, which is what removes the client's copy", () => {
    const r = parseIcs(CANCELLED_HANDOFF);
    expect(r.events).toHaveLength(1);
    expect(r.events[0].status).toBe("cancelled");
    // A tombstone has no end; the parser must not invent one.
    expect(r.events[0].endAt).toBe(0);
  });

  it("does not read the live booking as a cancelled one", () => {
    // The counter-assertion: the test above would pass just as well if EVERY event parsed as
    // cancelled.
    expect(parseIcs(LIVE_HANDOFF).events[0].status).toBe("confirmed");
  });

  it("keeps every physical line inside the 75-octet limit real clients enforce", () => {
    const enc = new TextEncoder();
    for (const [name, doc] of [
      ["live", LIVE_HANDOFF],
      ["cancelled", CANCELLED_HANDOFF],
    ] as const) {
      for (const line of doc.split("\r\n")) {
        expect(enc.encode(line).length, `${name}: ${line}`).toBeLessThanOrEqual(75);
      }
    }
  });
});
