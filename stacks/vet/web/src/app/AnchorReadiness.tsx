// The portal side of the dog-tag ANCHOR gate: one probe, one rendering of each state, consumed by
// the Register pet screen (which refuses IN PLACE on `blocked`) and by Setup (where the backend's
// boot warning would otherwise scroll away in a terminal the operator never reads again).
//
// The decision itself is `dogTagAnchorReadiness` in @dogtag/ui — this file only fetches and renders.
// Two rules it must keep: only a DEFINITE `blocked` answer replaces a form (could-not-check is not
// a refusal, and the backend's own session-start gate enforces the real rule on every attempt), and
// nothing here claims the contracts WORK — `ready` is a configuration fact.
import {
  ANCHOR_UNKNOWN_NOTICE,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  dogTagAnchorProbeFailure,
  dogTagAnchorReadiness,
  type DogTagAnchorReadiness,
} from "@dogtag/ui";
import { AlertTriangle } from "lucide-react";
import { useEffect, useState } from "react";
import { useApp } from "./AppContext";

/** Probe `GET /health` once per mount. `null` = still checking — render neither verdict. */
export function useDogTagAnchorReadiness(): DogTagAnchorReadiness | null {
  const { api } = useApp();
  const [readiness, setReadiness] = useState<DogTagAnchorReadiness | null>(null);
  useEffect(() => {
    let cancelled = false;
    api
      .health()
      .then((h) => {
        if (!cancelled) setReadiness(dogTagAnchorReadiness(h));
      })
      .catch((err) => {
        if (!cancelled) setReadiness(dogTagAnchorProbeFailure(err));
      });
    return () => {
      cancelled = true;
    };
  }, [api]);
  return readiness;
}

/**
 * The refusal card for a DEFINITE `blocked` answer. Rendered IN PLACE of the Register pet form:
 * the operator is the one who can act, so this is where the refusal belongs — not on the owner's
 * phone after a tag was allocated and a QR scanned.
 */
export function AnchorBlockedCard({ readiness }: { readiness: DogTagAnchorReadiness }) {
  if (readiness.state !== "blocked") return null;
  return (
    <Card data-testid="anchor-blocked">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-warning">
          <AlertTriangle className="h-5 w-5" /> {readiness.headline}
        </CardTitle>
        <CardDescription>
          Registering a pet allocates a dog tag and asks the owner to scan a QR — an issuance this
          backend cannot complete. Nothing has been allocated.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {readiness.problems.map((p) => (
          <p key={p} className="rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-onSurface">
            {p}
          </p>
        ))}
        <p className="text-xs text-muted">
          This warning stays on the Register pet screen and the Setup page until the configuration
          is fixed; the backend prints the same at boot. Every other part of the portal keeps
          working.
        </p>
      </CardContent>
    </Card>
  );
}

/**
 * The could-not-check line for an `unknown` answer. States what was not established and what still
 * stands — the form stays usable, because a refusal needs a definite answer and the backend refuses
 * an unanchorable start itself, allocating nothing.
 */
export function AnchorUnknownNotice({ readiness }: { readiness: DogTagAnchorReadiness }) {
  if (readiness.state !== "unknown") return null;
  return (
    <div
      data-testid="anchor-unknown"
      className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning"
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
      <span>
        {ANCHOR_UNKNOWN_NOTICE} — {readiness.reason}. Starting an issuance is still offered; a
        backend that cannot anchor refuses it up front with nothing allocated.
      </span>
    </div>
  );
}
