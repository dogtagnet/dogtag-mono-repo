import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  QrCode,
  Spinner,
} from "@dogtag/ui";
import type { AppointmentShareResp, CrmAppointment } from "@dogtag/ui";
import { Copy, Check } from "lucide-react";
import { useEffect, useState } from "react";
import { useApp } from "../app/AppContext";
import { formatDate, formatSlot } from "../lib/time";

/**
 * Hand ONE booking to the client it belongs to.
 *
 * The client scans this at the counter (or follows the link) and lands on a page that offers the
 * appointment as an `.ics` their phone opens natively, or as an add-to-Google link. What they
 * receive carries the service, the slot, the shop and their pet — never this shop's notes, never
 * their own name, and never another client's booking.
 *
 * THE QR IS DRAWN ONLY WHEN THE BACKEND SAYS IT CAN BE. `qrUrl` is populated if and only if this
 * deployment has a base a client's PHONE could reach; on the shipped `localhost` default it is
 * `null` and `qrUnavailableReason` explains why. This component keys the QR off that field ALONE and
 * never falls back to `window.location.origin` or to the link text — a QR built from anything else
 * is exactly the defect that shipped in the receipt QR, where a code encoding an unreachable host
 * still read as a working link to whoever scanned it. Rendering nothing and saying so is the honest
 * answer, and it is what this does.
 */
export function ShareAppointmentDialog({
  appointment,
  onClose,
}: {
  appointment: CrmAppointment | null;
  onClose: () => void;
}) {
  const { api } = useApp();
  const [share, setShare] = useState<AppointmentShareResp | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const id = appointment?.appointmentId ?? null;
  useEffect(() => {
    if (!id) {
      setShare(null);
      setError(null);
      setCopied(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    setShare(null);
    setCopied(false);
    api
      .shareAppointment(id)
      .then((s) => {
        if (!cancelled) setShare(s);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [api, id]);

  async function copy() {
    if (!share?.url) return;
    try {
      await navigator.clipboard.writeText(share.url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // A clipboard the browser refused is not a failure worth an error state — the URL is on
      // screen and selectable either way.
    }
  }

  return (
    <Dialog open={appointment !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex flex-col items-center" data-testid="share-appointment-dialog">
        <DialogHeader>
          <DialogTitle>Send to the client's calendar</DialogTitle>
          <DialogDescription>
            {appointment ? (
              <>
                <strong>
                  {appointment.service || "Appointment"}
                  {appointment.petName ? ` - ${appointment.petName}` : ""}
                </strong>{" "}
                · {formatDate(appointment.startAt)}, {formatSlot(appointment.startAt, appointment.endAt)}
              </>
            ) : (
              "One appointment, handed to the client it belongs to."
            )}
          </DialogDescription>
        </DialogHeader>

        {loading && (
          <p className="flex items-center gap-2 py-6 text-sm text-muted">
            <Spinner className="h-4 w-4" /> Preparing the link…
          </p>
        )}

        {error && (
          <Badge variant="danger" data-testid="share-appointment-error">
            {error}
          </Badge>
        )}

        {share && (
          <div className="flex w-full flex-col items-center gap-3">
            {/* The QR exists only when the backend vouched for the base. */}
            {/* No caption: the copyable field below already shows the same URL, and printing it
                twice reads as two different things at a glance. */}
            {share.qrUrl ? (
              <QrCode value={share.qrUrl} />
            ) : (
              <div
                className="w-full rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm text-onSurface"
                data-testid="share-appointment-no-qr"
              >
                <p className="font-semibold">No QR for this deployment</p>
                <p className="mt-1 text-muted">{share.qrUnavailableReason}</p>
              </div>
            )}

            {share.url && (
              <div className="flex w-full items-center gap-2">
                <code className="flex-1 break-all rounded border border-border bg-surfaceAlt px-2 py-1.5 text-xs">
                  {share.url}
                </code>
                <Button variant="outline" size="sm" onClick={() => void copy()}>
                  {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
            )}

            <p className="text-center text-xs text-muted">
              The client scans this to add the booking to their own calendar. It shows this
              appointment only — not your notes, and not your other clients.
            </p>
            <p className="text-center text-xs text-muted">
              What they add is a <strong>copy</strong>. If you move or cancel this booking their
              calendar will not update on its own — re-opening the same link always shows the
              current details, and they can re-add it to correct their copy.
            </p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
