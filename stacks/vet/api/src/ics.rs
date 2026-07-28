//! iCalendar (RFC 5545) SERIALIZATION for the shop's published `.ics` subscription feed.
//!
//! WHY THIS DIRECTION ONLY. Google Calendar, Apple Calendar and Luma can all SUBSCRIBE to an ICS
//! URL, so one published feed gives the shop all three with no OAuth, no API keys and nothing to
//! re-certify when a provider changes its terms. Emitting a calendar is a pure projection of rows
//! this backend already holds, so it lives here, in Rust, close to the store.
//!
//! The opposite direction — READING an uploaded `.ics` — deliberately does NOT live here. Parsing a
//! real-world calendar means resolving `TZID=Europe/London` to an exact UTC instant on a given date,
//! DST transitions included, and this crate has no timezone database (no `chrono-tz`); a hand-rolled
//! VTIMEZONE interpreter would be a plausible-looking source of silently-wrong appointment times.
//! The portal parses the file in the BROWSER instead, where `Intl.DateTimeFormat` resolves any IANA
//! zone exactly, and POSTs already-normalized UNIX SECONDS to `POST /calendar/import`. See
//! `packages/ui/src/calendar/ics.ts`.
//!
//! Everything below is pure: no clock, no store, no I/O — so it is exhaustively unit-testable, and
//! is exercised end-to-end by `tests/calendar_ics.rs`.

/// One `VEVENT`, already reduced to the fields the feed publishes.
#[derive(Clone, Debug)]
pub struct IcsEvent {
    /// Globally-unique, STABLE across regenerations (RFC 5545 §3.8.4.7) — this is what lets a
    /// subscriber update an event in place instead of accumulating duplicates on every refresh.
    pub uid: String,
    /// Event start, UNIX SECONDS (UTC).
    pub start: u64,
    /// Event end, UNIX SECONDS (UTC). Emitted only when it is after `start`.
    pub end: u64,
    pub summary: String,
    pub description: String,
    /// `CONFIRMED` | `TENTATIVE` | `CANCELLED`.
    pub status: IcsStatus,
    /// Unix seconds the underlying row last changed -> `LAST-MODIFIED`.
    pub last_modified: u64,
    /// Where this event can be read live -> `URL`. Empty to omit.
    ///
    /// The one property that gives a calendar entry a way BACK to the truth. A downloaded `.ics` is a
    /// snapshot; the shop can move or cancel the booking afterwards and nothing pushes that into a
    /// calendar the file was imported into. Carrying the handoff page here means the stale entry
    /// still names the page that states the current answer.
    pub url: String,
    /// Human place -> `LOCATION`. Empty to omit.
    pub location: String,
    /// Revision of THIS `UID`, per RFC 5545 §3.8.7.4 — monotonically non-decreasing across
    /// regenerations, so a later publication supersedes an earlier one in a client that already
    /// holds it rather than sitting beside it.
    ///
    /// `0` for a feed whose subscribers re-read the whole calendar every refresh and therefore need
    /// no per-event revision. It is load-bearing for a SINGLE event handed to a client once and
    /// re-downloaded later, which is the only way that client's copy can learn the booking moved.
    ///
    /// Deliberately NOT a unix timestamp: iCalendar's INTEGER is a SIGNED 32-BIT value, which unix
    /// seconds overflow in 2038. Callers derive a small monotonic revision instead (see
    /// `appointment_share::client_sequence`).
    pub sequence: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcsStatus {
    Confirmed,
    Tentative,
    Cancelled,
}

impl IcsStatus {
    fn as_str(self) -> &'static str {
        match self {
            IcsStatus::Confirmed => "CONFIRMED",
            IcsStatus::Tentative => "TENTATIVE",
            IcsStatus::Cancelled => "CANCELLED",
        }
    }
}

/// How often a subscriber is ASKED to re-poll. Advisory in both directions: Google in particular
/// refreshes a subscribed URL on its own schedule (historically hours, not minutes) regardless of
/// what the feed requests, which is exactly why the portal states the lag rather than implying this
/// value is honoured.
const REFRESH_INTERVAL: &str = "PT15M";

/// RFC 5545 §3.1: content lines are folded at 75 OCTETS.
const FOLD_OCTETS: usize = 75;

/// Serialize a whole `VCALENDAR`. `now` is the generation instant (`DTSTAMP`), passed in rather than
/// read from the clock so the output is a pure function of its inputs.
pub fn calendar(prodid: &str, cal_name: &str, now: u64, events: &[IcsEvent]) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        format!("PRODID:{}", escape_text(prodid)),
        "CALSCALE:GREGORIAN".to_string(),
        // PUBLISH, not REQUEST: this is a read-only broadcast of the shop's schedule. It is NOT an
        // invitation, so no subscriber is ever prompted to accept or decline.
        "METHOD:PUBLISH".to_string(),
        format!("X-WR-CALNAME:{}", escape_text(cal_name)),
        format!("REFRESH-INTERVAL;VALUE=DURATION:{REFRESH_INTERVAL}"),
        format!("X-PUBLISHED-TTL:{REFRESH_INTERVAL}"),
    ];
    for e in events {
        lines.extend(event_lines(e, now));
    }
    lines.push("END:VCALENDAR".to_string());

    let mut out = String::new();
    for l in lines {
        out.push_str(&fold_line(&l));
        // CRLF is mandatory (§3.1); a bare LF is rejected by strict parsers, Apple Calendar included.
        out.push_str("\r\n");
    }
    out
}

fn event_lines(e: &IcsEvent, now: u64) -> Vec<String> {
    let mut v = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape_text(&e.uid)),
        format!("DTSTAMP:{}", format_utc(now)),
        format!("DTSTART:{}", format_utc(e.start)),
    ];
    // A zero-length or inverted slot has no meaningful DTEND; omitting it makes the event last its
    // default duration rather than publishing an end that precedes its own start.
    if e.end > e.start {
        v.push(format!("DTEND:{}", format_utc(e.end)));
    }
    v.push(format!("SUMMARY:{}", escape_text(&e.summary)));
    if !e.description.is_empty() {
        v.push(format!("DESCRIPTION:{}", escape_text(&e.description)));
    }
    if !e.location.is_empty() {
        v.push(format!("LOCATION:{}", escape_text(&e.location)));
    }
    if !e.url.is_empty() {
        // URL is a URI value (§3.3.13), NOT TEXT: it is not backslash-escaped, because escaping a
        // query string's `,`/`;` would corrupt the link. Only the characters that would break line
        // FRAMING are removed, which no well-formed URI contains anyway.
        v.push(format!("URL:{}", sanitize_uri(&e.url)));
    }
    v.push(format!("STATUS:{}", e.status.as_str()));
    if e.sequence > 0 {
        v.push(format!("SEQUENCE:{}", e.sequence));
    }
    if e.last_modified > 0 {
        v.push(format!("LAST-MODIFIED:{}", format_utc(e.last_modified)));
    }
    v.push("END:VEVENT".to_string());
    v
}

/// Strip what would break RFC 5545 line framing from a URI value.
///
/// A URI is emitted verbatim rather than TEXT-escaped, so the grammar's only remaining exposure is a
/// control character splitting the content line. Dropping those cannot corrupt a legitimate URI —
/// none may contain a raw CR, LF or space (RFC 3986 §2).
pub fn sanitize_uri(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && *c != ' ').collect()
}

/// Unix seconds -> the RFC 5545 UTC form `YYYYMMDDTHHMMSSZ`.
///
/// Hand-rolled because this crate has no date library, using Howard Hinnant's `civil_from_days` —
/// exact for every proleptic-Gregorian date, no lookup tables, no leap-second fiction (POSIX time
/// has none, and neither does iCalendar's UTC form).
pub fn format_utc(unix_sec: u64) -> String {
    let days = (unix_sec / 86_400) as i64;
    let secs_of_day = unix_sec % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

/// Days since 1970-01-01 -> (year, month, day). Hinnant's civil-calendar algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Escape a TEXT value per RFC 5545 §3.3.11.
///
/// The backslash MUST be escaped first: doing it after the others would double-escape the
/// backslashes this function itself just introduced.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            // A bare CR carries no meaning of its own in a TEXT value and would corrupt the CRLF
            // framing if emitted literally.
            '\r' => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Fold one content line to <= 75 OCTETS per line (§3.1), continuations prefixed with a single space.
///
/// The limit is on OCTETS, not characters, but a fold may not land inside a multi-byte UTF-8
/// sequence — a client that splits a codepoint gets mojibake in a client name. So the break is taken
/// at the last character boundary that still fits.
pub fn fold_line(line: &str) -> String {
    if line.len() <= FOLD_OCTETS {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + line.len() / FOLD_OCTETS * 3);
    let bytes = line.as_bytes();
    let mut start = 0usize;
    // The first line may use all 75 octets; every continuation spends one on its leading space.
    let mut budget = FOLD_OCTETS;
    while start < bytes.len() {
        let mut end = (start + budget).min(bytes.len());
        // walk back to a char boundary (never splits a codepoint)
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }
        // A single character wider than the remaining budget would otherwise loop forever; give it
        // its own (over-long) continuation rather than emitting an empty one.
        if end == start {
            end = start + 1;
            while end < bytes.len() && !line.is_char_boundary(end) {
                end += 1;
            }
        }
        if start > 0 {
            out.push_str("\r\n ");
        }
        out.push_str(&line[start..end]);
        start = end;
        budget = FOLD_OCTETS - 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev() -> IcsEvent {
        IcsEvent {
            uid: "appt-1@groomer.example.com".into(),
            start: 1_772_020_800, // 2026-02-25T12:00:00Z
            end: 1_772_024_400,   // 2026-02-25T13:00:00Z
            summary: "Full groom - Rex".into(),
            description: "Status: Scheduled".into(),
            status: IcsStatus::Confirmed,
            last_modified: 1_772_000_000,
            url: String::new(),
            location: String::new(),
            sequence: 0,
        }
    }

    // ---- format_utc / civil_from_days ---------------------------------------------------------

    #[test]
    fn format_utc_matches_known_instants() {
        assert_eq!(format_utc(0), "19700101T000000Z");
        assert_eq!(format_utc(1_772_020_800), "20260225T120000Z");
        // last second of a year
        assert_eq!(format_utc(1_767_225_599), "20251231T235959Z");
    }

    #[test]
    fn format_utc_handles_leap_days_and_century_rules() {
        // 2024 is a leap year: Feb 29 exists.
        assert_eq!(format_utc(1_709_164_800), "20240229T000000Z");
        // 2000 was a leap year (divisible by 400); 1900 was not — check the 2000 side, which is the
        // one inside the unix epoch.
        assert_eq!(format_utc(951_782_400), "20000229T000000Z");
        assert_eq!(format_utc(951_868_800), "20000301T000000Z");
    }

    #[test]
    fn civil_from_days_round_trips_every_day_of_a_decade() {
        // Walk ten years a day at a time and check the calendar advances exactly as a calendar does.
        let mut expect = (2020i64, 1u32, 1u32);
        let start_days = 18_262i64; // 2020-01-01
        let is_leap =
            |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let dim = |y: i64, m: u32| match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            _ => {
                if is_leap(y) {
                    29
                } else {
                    28
                }
            }
        };
        for i in 0..3653 {
            assert_eq!(civil_from_days(start_days + i), expect, "day offset {i}");
            let (y, m, d) = expect;
            expect = if d < dim(y, m) {
                (y, m, d + 1)
            } else if m < 12 {
                (y, m + 1, 1)
            } else {
                (y + 1, 1, 1)
            };
        }
    }

    // ---- escape_text --------------------------------------------------------------------------

    #[test]
    fn escape_text_escapes_the_rfc_special_characters() {
        assert_eq!(escape_text("a;b,c"), "a\\;b\\,c");
        assert_eq!(escape_text("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_text("plain text"), "plain text");
    }

    #[test]
    fn escape_text_escapes_backslash_before_the_others() {
        // "a\;b" must become "a\\\;b" — the literal backslash doubled, THEN the semicolon escaped.
        // Escaping in the wrong order yields "a\\;b", which a parser reads as a backslash followed
        // by an unescaped field separator.
        assert_eq!(escape_text("a\\;b"), "a\\\\\\;b");
        assert_eq!(escape_text("100% \\ pure"), "100% \\\\ pure");
    }

    #[test]
    fn escape_text_drops_bare_cr_so_line_framing_survives() {
        assert_eq!(escape_text("a\r\nb"), "a\\nb");
        assert_eq!(escape_text("a\rb"), "ab");
    }

    // ---- fold_line ----------------------------------------------------------------------------

    #[test]
    fn fold_line_leaves_short_lines_untouched() {
        assert_eq!(fold_line("SUMMARY:short"), "SUMMARY:short");
        let exactly_75 = "X".repeat(75);
        assert_eq!(fold_line(&exactly_75), exactly_75);
    }

    #[test]
    fn fold_line_keeps_every_physical_line_within_75_octets() {
        let long = format!("DESCRIPTION:{}", "abcdefghij".repeat(40));
        let folded = fold_line(&long);
        for (i, seg) in folded.split("\r\n").enumerate() {
            assert!(
                seg.len() <= 75,
                "segment {i} is {} octets: {seg}",
                seg.len()
            );
            if i > 0 {
                assert!(seg.starts_with(' '), "continuation {i} must start with a space");
            }
        }
    }

    #[test]
    fn fold_line_unfolds_back_to_the_original() {
        // Unfolding (§3.1) is "delete every CRLF followed by a single whitespace" — the round trip
        // is the property that actually matters to a consumer.
        let long = format!("DESCRIPTION:{}", "the quick brown fox ".repeat(12));
        assert_eq!(fold_line(&long).replace("\r\n ", ""), long);
    }

    #[test]
    fn fold_line_never_splits_a_multibyte_character() {
        // Emoji are 4 octets each: a naive 75-octet cut lands mid-sequence.
        let long = format!("SUMMARY:{}", "🐕".repeat(60));
        let folded = fold_line(&long);
        for seg in folded.split("\r\n") {
            assert!(seg.len() <= 75);
            // Each segment must still be valid UTF-8 with no replacement characters, which it is by
            // construction if we only ever cut on boundaries.
            assert!(!seg.contains('\u{FFFD}'));
        }
        assert_eq!(folded.replace("\r\n ", ""), long);
    }

    // ---- calendar -----------------------------------------------------------------------------

    #[test]
    fn calendar_emits_a_wellformed_vcalendar_envelope() {
        let out = calendar("-//DogTag//Groomer//EN", "Paws & Claws", 1_772_000_000, &[ev()]);
        assert!(out.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        assert!(out.contains("VERSION:2.0\r\n"));
        assert!(out.contains("PRODID:-//DogTag//Groomer//EN\r\n"));
        assert!(out.contains("METHOD:PUBLISH\r\n"));
        assert!(out.contains("X-WR-CALNAME:Paws & Claws\r\n"));
        // every physical line is CRLF-terminated, none bare-LF
        assert_eq!(out.matches('\n').count(), out.matches("\r\n").count());
    }

    #[test]
    fn calendar_emits_the_event_fields_including_a_stable_uid() {
        let out = calendar("p", "c", 1_772_000_000, &[ev()]);
        assert!(out.contains("BEGIN:VEVENT\r\n"));
        assert!(out.contains("UID:appt-1@groomer.example.com\r\n"));
        assert!(out.contains("DTSTAMP:20260225T061320Z\r\n"));
        assert!(out.contains("DTSTART:20260225T120000Z\r\n"));
        assert!(out.contains("DTEND:20260225T130000Z\r\n"));
        assert!(out.contains("SUMMARY:Full groom - Rex\r\n"));
        assert!(out.contains("STATUS:CONFIRMED\r\n"));
        assert!(out.contains("LAST-MODIFIED:20260225T061320Z\r\n"));
        assert!(out.contains("END:VEVENT\r\n"));
    }

    #[test]
    fn calendar_omits_dtend_when_the_slot_is_empty_or_inverted() {
        let mut e = ev();
        e.end = e.start;
        assert!(!calendar("p", "c", 1, &[e.clone()]).contains("DTEND"));
        e.end = e.start - 60;
        assert!(!calendar("p", "c", 1, &[e]).contains("DTEND"));
    }

    #[test]
    fn calendar_omits_empty_description_and_zero_last_modified() {
        let mut e = ev();
        e.description = String::new();
        e.last_modified = 0;
        let out = calendar("p", "c", 1, &[e]);
        assert!(!out.contains("DESCRIPTION"));
        assert!(!out.contains("LAST-MODIFIED"));
    }

    #[test]
    fn calendar_marks_a_cancelled_booking_so_subscribers_remove_it() {
        let mut e = ev();
        e.status = IcsStatus::Cancelled;
        assert!(calendar("p", "c", 1, &[e]).contains("STATUS:CANCELLED\r\n"));
    }

    #[test]
    fn calendar_with_no_events_is_still_a_valid_empty_calendar() {
        // An empty shop calendar is an empty calendar — never a fabricated sample booking.
        let out = calendar("p", "c", 1_772_000_000, &[]);
        assert!(out.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        assert!(!out.contains("BEGIN:VEVENT"));
    }

    // ---- URL / LOCATION / SEQUENCE (the single-event handoff's supersede machinery) -------------

    #[test]
    fn calendar_omits_url_location_and_sequence_when_they_carry_nothing() {
        // The published FEED sets none of them; its output must be byte-for-byte what it always was.
        let out = calendar("p", "c", 1, &[ev()]);
        assert!(!out.contains("URL:"));
        assert!(!out.contains("LOCATION:"));
        assert!(!out.contains("SEQUENCE:"));
    }

    #[test]
    fn calendar_emits_url_location_and_sequence_when_set() {
        let mut e = ev();
        e.url = "https://shop.example/a/deadbeef".into();
        e.location = "Pampered Paws".into();
        e.sequence = 7;
        let out = calendar("p", "c", 1, &[e]);
        assert!(out.contains("URL:https://shop.example/a/deadbeef\r\n"));
        assert!(out.contains("LOCATION:Pampered Paws\r\n"));
        assert!(out.contains("SEQUENCE:7\r\n"));
    }

    #[test]
    fn a_url_is_not_text_escaped_but_a_location_is() {
        // The asymmetry is the point: URL is a URI value (§3.3.13) and backslash-escaping its query
        // string would corrupt the link, while LOCATION is TEXT and must be escaped.
        let mut e = ev();
        e.url = "https://shop.example/a/tok?x=1,2;3".into();
        e.location = "Paws; Claws, Ltd".into();
        let out = calendar("p", "c", 1, &[e]);
        assert!(out.contains("URL:https://shop.example/a/tok?x=1,2;3\r\n"), "{out}");
        assert!(out.contains("LOCATION:Paws\\; Claws\\, Ltd\r\n"), "{out}");
    }

    #[test]
    fn sanitize_uri_drops_only_what_would_break_line_framing() {
        assert_eq!(
            sanitize_uri("https://a.example/x?y=1,2;3"),
            "https://a.example/x?y=1,2;3",
            "a legitimate URI passes through untouched"
        );
        // A newline would end the content line early and inject a forged property.
        assert_eq!(
            sanitize_uri("https://a.example/x\r\nSUMMARY:forged"),
            "https://a.example/xSUMMARY:forged"
        );
        assert_eq!(sanitize_uri("https://a.example/a b"), "https://a.example/ab");
    }

    #[test]
    fn a_url_carrying_a_newline_cannot_inject_a_property() {
        let mut e = ev();
        e.url = "https://shop.example/a/tok\r\nSUMMARY:FORGED".into();
        let out = calendar("p", "c", 1, &[e]);
        // A property is only a property when it STARTS a content line. The forged text survives as
        // inert characters inside the URL value, which is harmless; what must not happen is it
        // beginning a line of its own.
        assert!(
            !out.contains("\r\nSUMMARY:FORGED"),
            "the injected text began its own content line: {out}"
        );
        // exactly one SUMMARY property, the real one
        assert_eq!(out.matches("\r\nSUMMARY:").count(), 1, "{out}");
    }

    #[test]
    fn calendar_escapes_operator_text_rather_than_breaking_the_grammar() {
        let mut e = ev();
        e.summary = "Groom; wash, dry".into();
        e.description = "note\nsecond line".into();
        let out = calendar("p", "c", 1, &[e]);
        assert!(out.contains("SUMMARY:Groom\\; wash\\, dry\r\n"));
        assert!(out.contains("DESCRIPTION:note\\nsecond line\r\n"));
    }
}
