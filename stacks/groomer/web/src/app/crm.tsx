/**
 * Shared CRM data plumbing + presentation atoms for the booking surfaces.
 *
 * Every list here is fetched with SERVER-SIDE search, filter and paging: the hook re-queries the
 * backend whenever a filter changes and only ever holds one bounded page. Nothing pulls a whole
 * collection into the browser to narrow it locally.
 */

import { Badge, Button, Spinner, useToast } from "@dogtag/ui";
import type { AppointmentStatus, CrmAppointment, CrmPage, CrmVerification } from "@dogtag/ui";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

/** How many rows a list page holds. Matches what the operator can scan without a pager marathon. */
export const PAGE_SIZE = 25;

/**
 * Debounce a value so typing in a search box issues one request per pause, not one per keystroke.
 */
export function useDebounced<T>(value: T, delayMs = 250): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(t);
  }, [value, delayMs]);
  return debounced;
}

export interface ListResult<T> {
  page: CrmPage<T> | null;
  loading: boolean;
  error: string | null;
  reload: () => void;
}

/**
 * Run a paged list query, re-running it whenever `deps` change.
 *
 * Responses are sequenced: a slow earlier request can never overwrite a newer one's results (the
 * classic search-box race where an old page flashes back after you have typed more).
 */
export function useList<T>(fetcher: () => Promise<CrmPage<T>>, deps: unknown[]): ListResult<T> {
  const [page, setPage] = useState<CrmPage<T> | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);
  const latest = useRef(0);

  useEffect(() => {
    const seq = ++latest.current;
    let cancelled = false;
    setLoading(true);
    fetcher()
      .then((p) => {
        if (cancelled || seq !== latest.current) return;
        setPage(p);
        setError(null);
      })
      .catch((e: Error) => {
        if (cancelled || seq !== latest.current) return;
        setError(e.message);
      })
      .finally(() => {
        if (!cancelled && seq === latest.current) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { page, loading, error, reload };
}

/** Surface a thrown API error as a toast; returns a runner that swallows the rejection. */
export function useAction() {
  const { toast } = useToast();
  const [busy, setBusy] = useState(false);
  const run = useCallback(
    async (fn: () => Promise<unknown>, opts: { success?: string; failure: string }) => {
      setBusy(true);
      try {
        await fn();
        if (opts.success) toast({ title: opts.success, variant: "success" });
        return true;
      } catch (e) {
        toast({
          title: opts.failure,
          description: (e as Error).message,
          variant: "danger",
        });
        return false;
      } finally {
        setBusy(false);
      }
    },
    [toast],
  );
  return { run, busy };
}

// --------------------------------------------------------------------------------------------
// presentation atoms
// --------------------------------------------------------------------------------------------

const APPOINTMENT_STATUS_LABEL: Record<AppointmentStatus, string> = {
  scheduled: "Scheduled",
  confirmed: "Confirmed",
  in_progress: "In progress",
  completed: "Completed",
  cancelled: "Cancelled",
  no_show: "No show",
};

type BadgeVariant = "neutral" | "success" | "warning" | "danger";

const APPOINTMENT_STATUS_VARIANT: Record<AppointmentStatus, BadgeVariant> = {
  scheduled: "neutral",
  confirmed: "success",
  in_progress: "warning",
  completed: "success",
  cancelled: "danger",
  no_show: "danger",
};

export function AppointmentStatusBadge({ status }: { status: AppointmentStatus }) {
  return (
    <Badge variant={APPOINTMENT_STATUS_VARIANT[status] ?? "neutral"}>
      {APPOINTMENT_STATUS_LABEL[status] ?? status}
    </Badge>
  );
}

export function statusLabel(status: AppointmentStatus): string {
  return APPOINTMENT_STATUS_LABEL[status] ?? status;
}

const VERIFICATION_STATUS_VARIANT: Record<CrmVerification["status"], BadgeVariant> = {
  pending: "neutral",
  recording: "warning",
  recorded: "success",
  error: "danger",
};

const VERIFICATION_STATUS_LABEL: Record<CrmVerification["status"], string> = {
  pending: "Awaiting owner",
  recording: "Recording",
  recorded: "Verified",
  error: "Failed",
};

export function VerificationStatusBadge({ status }: { status: CrmVerification["status"] }) {
  return (
    <Badge variant={VERIFICATION_STATUS_VARIANT[status] ?? "neutral"}>
      {VERIFICATION_STATUS_LABEL[status] ?? status}
    </Badge>
  );
}

/** What the owner chose to disclose, with the privacy meaning spelled out rather than left as
 * jargon. There is no verification mode to report — only how much the owner revealed. */
export function DisclosureBadge({
  disclosedKeyPaths,
}: {
  disclosedKeyPaths: CrmVerification["disclosedKeyPaths"];
}) {
  const n = disclosedKeyPaths?.length ?? 0;
  return n === 0 ? (
    <Badge variant="neutral" title="Owner-hidden: the owner revealed nothing and no credential data was written on chain">
      Nothing revealed
    </Badge>
  ) : (
    <Badge variant="neutral" title={disclosedKeyPaths.join(", ")}>
      {n} field{n === 1 ? "" : "s"} revealed
    </Badge>
  );
}

/**
 * The client an appointment belongs to, or an honest "Unassigned" when it has none.
 *
 * A booking created by an `.ics` import carries no client (a calendar invite names an event, not a
 * customer), so `clientId` is empty until the operator links one. Rendering that as a link would
 * produce `/clients/` with no label — a dead link that reads as a bug. Every surface that shows an
 * appointment's client goes through here so they cannot drift apart.
 */
export function AppointmentClient({
  appointment,
  className,
}: {
  appointment: Pick<CrmAppointment, "clientId" | "clientName" | "source">;
  className?: string;
}) {
  const { clientId, clientName, source } = appointment;
  if (!clientId) {
    return (
      <span
        className={className ? `text-muted ${className}` : "text-muted"}
        title={
          source === "ics"
            ? "Imported from a calendar file. Edit the appointment to link a client."
            : "No client linked to this booking."
        }
      >
        Unassigned
      </span>
    );
  }
  return (
    <Link to={`/clients/${clientId}`} className={className ?? "hover:underline"}>
      {clientName || "—"}
    </Link>
  );
}

/**
 * The filter toolbar shared by every list page.
 *
 * A twelve-column track at `xl` is what stops the controls squeezing each other: a search box, a
 * couple of selects and a date range have genuinely different natural widths, and a 4-up equal grid
 * gives the two date inputs a QUARTER of the row between them. Children declare their own
 * `xl:col-span-*`; below `xl` this collapses to two columns and then to one, so it holds up narrow.
 */
export function FilterBar({ children }: { children: ReactNode }) {
  return <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-12">{children}</div>;
}

/** One labelled control inside a [`FilterBar`]. `min-w-0` keeps a long value from forcing overflow. */
export function FilterField({
  label,
  htmlFor,
  span,
  children,
}: {
  label?: string;
  htmlFor?: string;
  /** Tailwind col-span classes for the `xl` track, e.g. `"xl:col-span-4"`. */
  span: string;
  children: ReactNode;
}) {
  return (
    <div className={`min-w-0 space-y-1 ${span}`}>
      {label && (
        <label
          htmlFor={htmlFor}
          className="block text-xs font-medium uppercase tracking-wide text-muted"
        >
          {label}
        </label>
      )}
      {children}
    </div>
  );
}

/** Empty / loading / error placeholder for a list body. */
export function ListPlaceholder({
  loading,
  error,
  empty,
  emptyMessage,
  children,
}: {
  loading: boolean;
  error: string | null;
  empty: boolean;
  emptyMessage: string;
  children: ReactNode;
}) {
  if (error) return <p className="py-8 text-center text-sm text-danger">{error}</p>;
  if (loading && empty)
    return (
      <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
        <Spinner className="h-4 w-4" /> Loading…
      </div>
    );
  if (empty) return <p className="py-10 text-center text-sm text-muted">{emptyMessage}</p>;
  return <>{children}</>;
}

/** Offset pager. Hidden entirely when everything fits on one page. */
export function Pager({
  total,
  offset,
  limit,
  onOffset,
}: {
  total: number;
  offset: number;
  limit: number;
  onOffset: (next: number) => void;
}) {
  if (total <= limit) return null;
  const from = offset + 1;
  const to = Math.min(offset + limit, total);
  return (
    <div className="flex items-center justify-between gap-3 border-t border-border pt-3 text-sm text-muted">
      <span>
        {from}–{to} of {total}
      </span>
      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={offset === 0}
          onClick={() => onOffset(Math.max(0, offset - limit))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={to >= total}
          onClick={() => onOffset(offset + limit)}
        >
          Next
        </Button>
      </div>
    </div>
  );
}
