import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Spinner,
  parseIcs,
  useToast,
} from "@dogtag/ui";
import type { IcsFeedResp, IcsImportResp, IcsParseResult } from "@dogtag/ui";
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  Copy,
  Link2,
  RefreshCw,
  Trash2,
  Upload,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ChangeEvent } from "react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { useAction } from "../app/crm";
import { env } from "../lib/env";

/**
 * Calendar sync: publish the shop's schedule as a subscribable `.ics` feed, and import an `.ics`
 * file into real bookings.
 *
 * This page is deliberately blunt about what the feed is and is not. It is ONE URL that Google
 * Calendar, Apple Calendar and Luma can all subscribe to, with no OAuth and no API keys. It is also
 * READ-ONLY and refreshed on the subscriber's schedule, and the URL is a credential — all three are
 * stated here rather than left for the operator to discover after they have posted the link
 * somewhere.
 */
export function CalendarSync() {
  return (
    <div className="space-y-4">
      <Link
        to="/calendar"
        className="inline-flex items-center gap-1 text-sm text-muted hover:text-onSurface"
      >
        <ArrowLeft className="h-4 w-4" /> Back to calendar
      </Link>
      <FeedCard />
      <ImportCard />
    </div>
  );
}

/**
 * Compose the absolute URL a calendar client subscribes to.
 *
 * Built against the BROWSER's origin plus the configured API base, because that is the address the
 * operator is actually reachable at — the backend's own `DEPLOYMENT_URL` can legitimately differ
 * (a dev proxy, a tunnel), and a link that is right in config but wrong in the browser is a link
 * that silently fails to subscribe.
 */
function absoluteFeedUrl(path: string): string {
  const base = env.groomerApiBase.replace(/\/$/, "");
  return new URL(`${base}${path}`, window.location.origin).toString();
}

function FeedCard() {
  const { api } = useApp();
  const { toast } = useToast();
  const { run, busy } = useAction();
  const [feed, setFeed] = useState<IcsFeedResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    api
      .getIcsFeed()
      .then((f) => {
        setFeed(f);
        setError(null);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setLoading(false));
  }, [api]);

  useEffect(load, [load]);

  const url = feed?.enabled && feed.path ? absoluteFeedUrl(feed.path) : null;

  async function copy() {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast({
        title: "Could not copy",
        description: "Select the link and copy it by hand.",
        variant: "danger",
      });
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Link2 className="h-5 w-5 text-primary" /> Subscribe from Google, Apple or Luma
        </CardTitle>
        <CardDescription>
          Publish this shop's bookings at one web address. Google Calendar, Apple Calendar and Luma
          can all subscribe to it — no account linking, no app permissions, nothing to reconnect.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {error && <p className="text-sm text-danger">{error}</p>}

        {loading ? (
          <div className="flex items-center gap-2 py-4 text-sm text-muted">
            <Spinner className="h-4 w-4" /> Loading…
          </div>
        ) : url ? (
          <>
            <div className="space-y-2">
              <label
                htmlFor="feed-url"
                className="block text-xs font-medium uppercase tracking-wide text-muted"
              >
                Your calendar address
              </label>
              <div className="flex flex-wrap gap-2">
                <Input
                  id="feed-url"
                  readOnly
                  value={url}
                  onFocus={(e) => e.currentTarget.select()}
                  className="min-w-0 flex-1 font-mono text-xs"
                />
                <Button variant="outline" onClick={() => void copy()}>
                  {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
            </div>

            {/* The single most important thing on this page: this URL is a password. */}
            <div className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning">
              <p className="flex items-center gap-2 font-semibold">
                <AlertTriangle className="h-4 w-4 shrink-0" /> Treat this address like a password
              </p>
              <p className="mt-1">
                Anyone who has it can read your whole schedule — client names, pets and times —
                without logging in. That is what lets a calendar app subscribe without an account.
                Share it only with people who should see the shop's diary, and replace it below if it
                ever gets out.
              </p>
            </div>

            <HowToSubscribe url={url} />

            <div className="flex flex-wrap gap-2 border-t border-border pt-3">
              <Button
                variant="outline"
                size="sm"
                loading={busy}
                onClick={() =>
                  void run(
                    async () => setFeed(await api.rotateIcsFeed()),
                    {
                      success: "New address published — re-subscribe every calendar",
                      failure: "Could not replace the address",
                    },
                  )
                }
              >
                <RefreshCw className="h-4 w-4" /> Replace address
              </Button>
              <Button
                variant="ghost"
                size="sm"
                loading={busy}
                onClick={() =>
                  void run(async () => setFeed(await api.revokeIcsFeed()), {
                    success: "Calendar address switched off",
                    failure: "Could not switch the address off",
                  })
                }
              >
                <Trash2 className="h-4 w-4" /> Switch off
              </Button>
            </div>
            <p className="text-xs text-muted">
              Replacing or switching off takes effect immediately: the old address stops working and
              every calendar subscribed to it stops updating.
            </p>
          </>
        ) : (
          <>
            <p className="text-sm text-muted">
              No calendar address has been published yet. Publishing one creates a secret web address
              that any calendar app can subscribe to.
            </p>
            <Button
              loading={busy}
              onClick={() =>
                void run(async () => setFeed(await api.rotateIcsFeed()), {
                  success: "Calendar address published",
                  failure: "Could not publish the address",
                })
              }
            >
              <Link2 className="h-4 w-4" /> Publish calendar address
            </Button>
          </>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * What subscribing does and does not get you, per provider.
 *
 * The refresh caveat is the one that generates support questions: a subscribed calendar is polled on
 * the PROVIDER's schedule, and Google's has historically been hours rather than minutes. Saying so
 * up front is the difference between a known limitation and an apparent bug.
 */
function HowToSubscribe({ url }: { url: string }) {
  const webcal = url.replace(/^https?:/, "webcal:");
  return (
    <div className="space-y-3 rounded-md border border-border bg-surface-muted/50 p-3 text-sm">
      <div>
        <p className="font-semibold text-onSurface">Apple Calendar</p>
        <p className="text-muted">
          File → New Calendar Subscription, paste the address. Set "Auto-refresh" to how often you
          want it checked. Or open{" "}
          <a href={webcal} className="text-primary hover:underline">
            this webcal link
          </a>
          .
        </p>
      </div>
      <div>
        <p className="font-semibold text-onSurface">Google Calendar</p>
        <p className="text-muted">
          Other calendars → From URL, paste the address. Google needs the address to be reachable
          from the internet, and refreshes it on its own schedule — often only every few hours, which
          it does not let you change.
        </p>
      </div>
      <div>
        <p className="font-semibold text-onSurface">Luma</p>
        <p className="text-muted">
          Add the address as an external calendar subscription in your calendar settings.
        </p>
      </div>
      <p className="border-t border-border pt-2 text-muted">
        <strong className="text-onSurface">One direction only.</strong> Subscribed calendars are
        read-only: bookings you make here appear there, but an event you create or move in Google,
        Apple or Luma does <em>not</em> come back to DogTag. Book and reschedule in this portal.
      </p>
    </div>
  );
}

function ImportCard() {
  const { api } = useApp();
  const { toast } = useToast();
  const fileInput = useRef<HTMLInputElement>(null);
  const [fileName, setFileName] = useState("");
  const [parsed, setParsed] = useState<IcsParseResult | null>(null);
  const [preview, setPreview] = useState<IcsImportResp | null>(null);
  const [result, setResult] = useState<IcsImportResp | null>(null);
  const [busy, setBusy] = useState(false);

  function reset() {
    setParsed(null);
    setPreview(null);
    setResult(null);
    setFileName("");
    if (fileInput.current) fileInput.current.value = "";
  }

  async function onFile(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setResult(null);
    setPreview(null);
    setFileName(file.name);
    setBusy(true);
    try {
      // Parsed HERE, in the browser, so `TZID=Europe/London` resolves through the full IANA
      // database the browser already ships. See packages/ui/src/calendar/ics.ts.
      const r = parseIcs(await file.text());
      setParsed(r);
      if (r.notACalendar) {
        toast({
          title: "That is not a calendar file",
          description: "An .ics file starts with BEGIN:VCALENDAR.",
          variant: "danger",
        });
        return;
      }
      if (r.events.length > 0) {
        // Dry run first: the operator sees exactly what would happen before anything is written.
        setPreview(await api.importIcsEvents(r.events, true));
      }
    } catch (err) {
      toast({
        title: "Could not read the file",
        description: (err as Error).message,
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

  async function confirmImport() {
    if (!parsed) return;
    setBusy(true);
    try {
      const r = await api.importIcsEvents(parsed.events, false);
      setResult(r);
      setPreview(null);
      toast({
        title: `Imported ${r.created + r.updated} of ${r.total} events`,
        variant: "success",
      });
    } catch (err) {
      toast({
        title: "Import failed",
        description: (err as Error).message,
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

  const summary = result ?? preview;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Upload className="h-5 w-5 text-primary" /> Import an .ics file
        </CardTitle>
        <CardDescription>
          Bring bookings in from another calendar, or from a calendar invite you were sent. The file
          is read in your browser, so times in any timezone come across exactly.
        </CardDescription>
      </CardHeader>
      <CardContent className="max-w-3xl space-y-4">
        <div className="flex flex-wrap items-center gap-3">
          <Input
            ref={fileInput}
            id="ics-file"
            type="file"
            accept=".ics,text/calendar"
            onChange={(e) => void onFile(e)}
            aria-label="Choose an .ics file"
            className="max-w-sm"
          />
          {busy && <Spinner className="h-4 w-4 text-muted" />}
          {fileName && !busy && (
            <Button variant="ghost" size="sm" onClick={reset}>
              Clear
            </Button>
          )}
        </div>

        {parsed && !parsed.notACalendar && parsed.events.length === 0 && (
          <p className="text-sm text-muted">
            {fileName} is a calendar file, but it has no events to import.
          </p>
        )}

        {summary && (
          <div className="space-y-3 rounded-md border border-border p-3 text-sm">
            <p className="font-semibold text-onSurface">
              {result ? "Imported" : `${fileName} — nothing has been saved yet`}
            </p>
            <ul className="space-y-1 text-muted">
              <li>
                <strong className="text-onSurface">{summary.created}</strong> new booking
                {summary.created === 1 ? "" : "s"}
                {result ? " created" : " would be created"}
              </li>
              <li>
                <strong className="text-onSurface">{summary.updated}</strong> existing booking
                {summary.updated === 1 ? "" : "s"} {result ? "updated" : "would be updated"} — this
                file has been imported before
              </li>
              {summary.cancelled > 0 && (
                <li>
                  <strong className="text-onSurface">{summary.cancelled}</strong>{" "}
                  {result ? "cancelled" : "would be cancelled"}
                </li>
              )}
              {summary.skipped > 0 && (
                <li>
                  <strong className="text-onSurface">{summary.skipped}</strong>{" "}
                  {result ? "skipped" : "would be skipped"} (repeated ids, or a cancellation for an
                  event that was never imported)
                </li>
              )}
            </ul>

            {/* The caveats. Stated as counts, not as a footnote, because they change what the
                operator ends up with. */}
            {summary.recurringNotExpanded > 0 && (
              <p className="rounded border border-warning/40 bg-warning/10 px-2 py-1.5 text-warning">
                <strong>{summary.recurringNotExpanded}</strong> of these events{" "}
                {summary.recurringNotExpanded === 1 ? "repeats" : "repeat"} on a schedule. Repeats
                are <strong>not</strong> expanded: you get the first occurrence only, not every
                future one. Book the rest here, or import them as separate events.
              </p>
            )}
            {summary.allDay > 0 && (
              <p className="text-muted">
                <strong className="text-onSurface">{summary.allDay}</strong> all-day event
                {summary.allDay === 1 ? "" : "s"} booked as a full day, since a booking is a slot.
              </p>
            )}
            <p className="text-muted">
              Imported bookings have <strong className="text-onSurface">no client</strong> attached —
              a calendar invite names an event, not one of your customers. Open each one and pick the
              client before running a vaccination check.
            </p>

            {parsed && parsed.skipped.length > 0 && (
              <div className="border-t border-border pt-2">
                <p className="font-semibold text-onSurface">
                  {parsed.skipped.length} event{parsed.skipped.length === 1 ? "" : "s"} in the file
                  could not be read
                </p>
                <ul className="mt-1 list-disc space-y-0.5 pl-5 text-muted">
                  {parsed.skipped.map((s) => (
                    <li key={`${s.label}-${s.reason}`}>
                      <span className="text-onSurface">{s.label}</span> — {s.reason}
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {!result && (
              <div className="flex flex-wrap gap-2 border-t border-border pt-3">
                <Button loading={busy} onClick={() => void confirmImport()}>
                  <Upload className="h-4 w-4" /> Import {summary.total} event
                  {summary.total === 1 ? "" : "s"}
                </Button>
                <Button variant="ghost" onClick={reset}>
                  Cancel
                </Button>
              </div>
            )}
            {result && (
              <div className="border-t border-border pt-3">
                <Button variant="outline" size="sm" asChild>
                  <Link to="/appointments">View appointments</Link>
                </Button>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
