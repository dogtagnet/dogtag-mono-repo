/**
 * iCalendar (RFC 5545) PARSING for `.ics` import.
 *
 * WHY THIS RUNS IN THE BROWSER. The hard part of reading a real calendar file is not the grammar,
 * it is `DTSTART;TZID=Europe/London:20260330T100000` — turning a wall-clock time in a named zone
 * into an exact instant, with that zone's DST rules for that particular date. The backend crate has
 * no timezone database, and a hand-rolled VTIMEZONE interpreter is a plausible-looking way to book
 * appointments an hour off twice a year. Every browser ships the full IANA database and exposes it
 * through `Intl.DateTimeFormat`, so resolving the offset here is exact and free — see
 * [`zonedToUnix`]. The parser therefore hands the backend UNIX SECONDS, and the backend performs no
 * time-zone interpretation of its own.
 *
 * WHAT THIS DELIBERATELY DOES NOT DO. `RRULE` is NOT expanded: a repeating event yields the single
 * occurrence its `DTSTART` names, and the event is flagged `recurring` so the caller can say so out
 * loud instead of silently turning a weekly standing appointment into one booking. Nothing here
 * invents data to fill a gap — an event that cannot be given a start time is reported as skipped,
 * with a reason, never guessed at.
 */

/** One `VEVENT`, normalized to what the import endpoint accepts. */
export interface IcsImportEvent {
  /** The source `UID`. Stable across exports of the same calendar — this is what dedups a re-import. */
  uid: string;
  summary: string;
  description: string;
  location: string;
  /** Unix seconds (UTC), resolved from the event's own `TZID`/`VALUE=DATE` semantics. */
  startAt: number;
  /** Unix seconds (UTC). `0` when the source gave no usable end. */
  endAt: number;
  /** The source used `VALUE=DATE` (an all-day event). */
  allDay: boolean;
  /** The source carried an `RRULE`. This parser does NOT expand it. */
  recurring: boolean;
  /** The source `STATUS`, lowercased (`""` when absent). */
  status: string;
}

/** An event the parser refused to guess at, with the reason, so the UI can show it. */
export interface IcsSkippedEvent {
  /** Whatever identified it in the file (`UID` or `SUMMARY`), for the operator to find it. */
  label: string;
  reason: string;
}

export interface IcsParseResult {
  events: IcsImportEvent[];
  skipped: IcsSkippedEvent[];
  /** How many events carried an `RRULE` (imported as a single occurrence). */
  recurring: number;
  /** How many events were all-day. */
  allDay: number;
  /** The calendar's `X-WR-CALNAME`, when it names itself. */
  calendarName: string;
  /** True when the payload had no `BEGIN:VCALENDAR` at all — i.e. this is not an iCalendar file. */
  notACalendar: boolean;
}

/** One parsed content line: property name, its parameters, and the raw value. */
interface ContentLine {
  name: string;
  params: Record<string, string>;
  value: string;
}

/**
 * Undo RFC 5545 §3.1 line folding, then split into logical lines.
 *
 * Real files arrive with CRLF, bare LF (hand-edited, or mangled by a transfer), or CR. A
 * continuation is a line break followed by ONE space or tab; both are legal and both appear in the
 * wild (Outlook uses tab).
 */
export function unfoldLines(text: string): string[] {
  return text
    .replace(/^﻿/, "") // strip a UTF-8 BOM: it would corrupt the first property name
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .replace(/\n[ \t]/g, "")
    .split("\n")
    .filter((l) => l.trim() !== "");
}

/**
 * Split one content line into `NAME;PARAM=VALUE:value`.
 *
 * The first COLON separates the property from its value — but a colon inside a QUOTED parameter
 * value (`TZID="GMT+01:00"`, which Outlook emits) does not count, so quoting is tracked rather than
 * naively taking `indexOf(':')`.
 */
export function parseContentLine(line: string): ContentLine | null {
  let colon = -1;
  let quoted = false;
  for (let i = 0; i < line.length; i += 1) {
    const c = line[i];
    if (c === '"') quoted = !quoted;
    else if (c === ":" && !quoted) {
      colon = i;
      break;
    }
  }
  if (colon === -1) return null;

  const head = line.slice(0, colon);
  const value = line.slice(colon + 1);
  const parts = splitUnquoted(head, ";");
  const name = (parts.shift() ?? "").trim().toUpperCase();
  if (name === "") return null;

  const params: Record<string, string> = {};
  for (const p of parts) {
    const eq = p.indexOf("=");
    if (eq === -1) continue;
    const key = p.slice(0, eq).trim().toUpperCase();
    // A quoted parameter value keeps its contents verbatim, minus the quotes.
    params[key] = p.slice(eq + 1).trim().replace(/^"(.*)"$/, "$1");
  }
  return { name, params, value };
}

function splitUnquoted(s: string, sep: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  for (const c of s) {
    if (c === '"') quoted = !quoted;
    if (c === sep && !quoted) {
      out.push(cur);
      cur = "";
    } else {
      cur += c;
    }
  }
  out.push(cur);
  return out;
}

/** Reverse the TEXT escaping of RFC 5545 §3.3.11. */
export function unescapeText(v: string): string {
  let out = "";
  for (let i = 0; i < v.length; i += 1) {
    if (v[i] !== "\\") {
      out += v[i];
      continue;
    }
    const next = v[i + 1];
    if (next === "n" || next === "N") out += "\n";
    else if (next === undefined) out += "\\"; // a trailing lone backslash is literal
    else out += next; // covers \\ \; \, and any other escaped literal
    i += 1;
  }
  return out;
}

/**
 * Resolve a wall-clock time in an IANA zone to unix seconds — exactly, DST included.
 *
 * `Intl.DateTimeFormat` with a `timeZone` renders an instant AS that zone reads it. So: guess the
 * instant as if the wall clock were UTC, ask the zone what wall clock that instant actually shows,
 * and correct by the difference. One correction is enough for every offset; the second pass exists
 * only for instants sitting inside a DST transition, where the first correction can land on the
 * other side of the jump.
 *
 * Returns `null` for a zone the runtime does not know, so the caller can report it rather than
 * silently booking in the wrong offset.
 */
export function zonedToUnix(
  y: number,
  mo: number,
  d: number,
  h: number,
  mi: number,
  s: number,
  timeZone: string,
): number | null {
  let fmt: Intl.DateTimeFormat;
  try {
    fmt = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hour12: false,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return null; // unknown/invalid TZID
  }

  const target = Date.UTC(y, mo - 1, d, h, mi, s);
  let guess = target;
  for (let pass = 0; pass < 2; pass += 1) {
    const parts: Record<string, number> = {};
    for (const p of fmt.formatToParts(new Date(guess))) {
      if (p.type !== "literal") parts[p.type] = Number(p.value);
    }
    const [py, pmo, pd, ph, pmi, ps] = [
      parts.year,
      parts.month,
      parts.day,
      parts.hour,
      parts.minute,
      parts.second,
    ];
    if (
      py === undefined ||
      pmo === undefined ||
      pd === undefined ||
      ph === undefined ||
      pmi === undefined ||
      ps === undefined ||
      !Number.isFinite(py)
    ) {
      return null;
    }
    // `hour` formats as 24 for midnight under hour12:false in some runtimes.
    const asZone = Date.UTC(py, pmo - 1, pd, ph % 24, pmi, ps);
    const drift = target - asZone;
    if (drift === 0) break;
    guess += drift;
  }
  return Math.floor(guess / 1000);
}

/** Local-time interpretation of a wall clock, for a floating (zone-less) value. */
function floatingToUnix(y: number, mo: number, d: number, h: number, mi: number, s: number): number {
  return Math.floor(new Date(y, mo - 1, d, h, mi, s).getTime() / 1000);
}

export interface ParsedInstant {
  unix: number;
  /** The value was a `VALUE=DATE` (all-day). */
  date: boolean;
}

/**
 * Parse a `DATE-TIME`/`DATE` value into unix seconds.
 *
 * Three shapes, per RFC 5545 §3.3.5:
 *   `20260330T100000Z`  — UTC, unambiguous.
 *   `20260330T100000`   — FLOATING or `TZID=`-qualified. With a TZID we resolve that zone exactly;
 *                         without one the spec says it is local wall time, so the shop's own
 *                         timezone is the correct reading.
 *   `20260330`          — a DATE (all-day). Becomes local midnight, because a shop's "all day" is
 *                         its own day, not UTC's.
 *
 * Returns `null` for anything unparseable or a TZID this runtime cannot resolve — the caller reports
 * it instead of booking a guess.
 */
export function parseInstant(value: string, params: Record<string, string>): ParsedInstant | null {
  const v = value.trim();
  const dateOnly = /^(\d{4})(\d{2})(\d{2})$/.exec(v);
  if (dateOnly || params.VALUE === "DATE") {
    const m = dateOnly ?? /^(\d{4})(\d{2})(\d{2})/.exec(v);
    if (!m) return null;
    return {
      unix: floatingToUnix(Number(m[1]), Number(m[2]), Number(m[3]), 0, 0, 0),
      date: true,
    };
  }

  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})(Z?)$/.exec(v);
  if (!m) return null;
  // Every group is guaranteed by the pattern above; `Number` of the captured digits is total.
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  const h = Number(m[4]);
  const mi = Number(m[5]);
  const s = Number(m[6]);
  const z = m[7];

  if (z === "Z") {
    return { unix: Math.floor(Date.UTC(y, mo - 1, d, h, mi, s) / 1000), date: false };
  }

  const tzid = params.TZID;
  if (tzid) {
    const resolved = zonedToUnix(y, mo, d, h, mi, s, tzid);
    return resolved === null ? null : { unix: resolved, date: false };
  }
  // Floating: RFC 5545 §3.3.5 says it means local time wherever it is read — for a shop's calendar
  // that is the shop's own clock, which is this browser's.
  return { unix: floatingToUnix(y, mo, d, h, mi, s), date: false };
}

/**
 * Parse an RFC 5545 `DURATION` (§3.3.6) into seconds. Used when an event gives `DTSTART` +
 * `DURATION` rather than `DTEND`, which is how most all-day and many recurring events are written.
 */
export function parseDuration(value: string): number | null {
  const m = /^([+-])?P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/.exec(
    value.trim().toUpperCase(),
  );
  if (!m) return null;
  const n = (i: number) => Number(m[i] ?? 0);
  const total = (n(2) * 7 + n(3)) * 86_400 + n(4) * 3600 + n(5) * 60 + n(6);
  if (total === 0 && !/\d/.test(value)) return null;
  return m[1] === "-" ? -total : total;
}

/** A stable synthetic id for an event whose file gave it no `UID`, so re-import still dedups. */
function synthesizeUid(summary: string, startAt: number): string {
  // FNV-1a over the two fields that identify the event to a human. Not a security hash — just a
  // deterministic, collision-resistant-enough label so the same file yields the same id twice.
  let hash = 0x811c9dc5;
  for (const ch of `${summary}|${startAt}`) {
    hash ^= ch.charCodeAt(0);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `dogtag-nouid-${hash.toString(16)}-${startAt}@dogtag.local`;
}

/**
 * Parse a whole `.ics` payload into events ready for `POST /calendar/import`.
 *
 * Never throws: a malformed file yields a result whose `skipped` list says what could not be read.
 */
export function parseIcs(text: string): IcsParseResult {
  const result: IcsParseResult = {
    events: [],
    skipped: [],
    recurring: 0,
    allDay: 0,
    calendarName: "",
    notACalendar: false,
  };

  const lines = unfoldLines(text);
  if (!lines.some((l) => l.toUpperCase().startsWith("BEGIN:VCALENDAR"))) {
    result.notACalendar = true;
    return result;
  }

  let current: Record<string, ContentLine> | null = null;
  // VEVENT is not the only component with DTSTART — VTIMEZONE's STANDARD/DAYLIGHT sub-components
  // carry their own, and an alarm carries a TRIGGER. Track depth so only top-level VEVENT
  // properties are collected.
  let depth = 0;

  for (const line of lines) {
    const upper = line.toUpperCase();
    if (upper.startsWith("BEGIN:VEVENT")) {
      current = {};
      depth = 0;
      continue;
    }
    if (upper.startsWith("END:VEVENT")) {
      if (current) finishEvent(current, result);
      current = null;
      continue;
    }
    if (current) {
      if (upper.startsWith("BEGIN:")) depth += 1;
      else if (upper.startsWith("END:")) depth = Math.max(0, depth - 1);
      else if (depth === 0) {
        const cl = parseContentLine(line);
        // First occurrence wins: a duplicated property is a malformed file, not a list.
        if (cl && !(cl.name in current)) current[cl.name] = cl;
      }
      continue;
    }
    if (upper.startsWith("X-WR-CALNAME")) {
      const cl = parseContentLine(line);
      if (cl) result.calendarName = unescapeText(cl.value);
    }
  }

  return result;
}

function finishEvent(props: Record<string, ContentLine>, out: IcsParseResult): void {
  const summary = props.SUMMARY ? unescapeText(props.SUMMARY.value).trim() : "";
  const uidRaw = props.UID ? unescapeText(props.UID.value).trim() : "";
  const label = uidRaw || summary || "(untitled event)";

  const dtstart = props.DTSTART;
  if (!dtstart) {
    out.skipped.push({ label, reason: "the event has no DTSTART, so it has no time to book" });
    return;
  }
  const start = parseInstant(dtstart.value, dtstart.params);
  if (!start) {
    const tzid = dtstart.params.TZID;
    out.skipped.push({
      label,
      reason: tzid
        ? `its timezone "${tzid}" is not one this browser can resolve, so the exact time is unknown`
        : `its start time "${dtstart.value}" is not a date this parser recognizes`,
    });
    return;
  }

  // An instant no booking can be made at. A `DTSTART` before 1970 is ordinary in a real export (a
  // birthday, or a year like `00010101` that JS maps to 1901) and resolves NEGATIVE; a value the
  // runtime cannot render resolves to NaN. Reported with a reason here, exactly like an unresolvable
  // TZID, rather than shipped to the import as a number it would have to reject.
  if (!Number.isFinite(start.unix) || start.unix <= 0) {
    out.skipped.push({
      label,
      reason: Number.isFinite(start.unix)
        ? `its start time "${dtstart.value}" is at or before 1 January 1970, which is not a date a booking can be made at`
        : `its start time "${dtstart.value}" does not resolve to a real point in time`,
    });
    return;
  }

  const allDay = start.date;
  let endAt = 0;
  const dtend = props.DTEND;
  if (dtend) {
    const end = parseInstant(dtend.value, dtend.params);
    if (end) endAt = end.unix;
  } else if (props.DURATION) {
    const secs = parseDuration(props.DURATION.value);
    if (secs !== null && secs > 0) endAt = start.unix + secs;
  }
  // An all-day event with no end spans exactly its own local day. Adding 86400 would be wrong across
  // a DST boundary, so step the calendar day instead.
  if (allDay && endAt <= start.unix) {
    const d = new Date(start.unix * 1000);
    d.setDate(d.getDate() + 1);
    endAt = Math.floor(d.getTime() / 1000);
  }
  // An end the import cannot hold is simply NO end — it applies its own default slot length rather
  // than the caller sending a number that is refused. The start is what an event cannot do without.
  if (!Number.isFinite(endAt) || endAt <= 0) endAt = 0;

  const recurring = "RRULE" in props;
  if (recurring) out.recurring += 1;
  if (allDay) out.allDay += 1;

  out.events.push({
    uid: uidRaw || synthesizeUid(summary, start.unix),
    summary,
    description: props.DESCRIPTION ? unescapeText(props.DESCRIPTION.value).trim() : "",
    location: props.LOCATION ? unescapeText(props.LOCATION.value).trim() : "",
    startAt: start.unix,
    endAt,
    allDay,
    recurring,
    status: props.STATUS ? props.STATUS.value.trim().toLowerCase() : "",
  });
}
