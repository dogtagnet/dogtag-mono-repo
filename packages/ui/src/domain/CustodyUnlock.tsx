import { KeyRound, ShieldAlert, Wand2 } from "lucide-react";
import { useState, type FormEvent, type ReactNode } from "react";
import { Button } from "../components/Button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/Card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../components/Dialog";
import { Input } from "../components/Input";
import { Label } from "../components/Label";
import { ThemeToggle } from "../theme/ThemeToggle";
import type { AccountInfo, AdminLoginResp, UnlockResp } from "../api/types";
import { isNoSealError, isWrongPassphraseError } from "../custody/lock";

/** What the form has CONFIRMED about the seal, via `/admin/login`. */
type Seal = "unknown" | "sealed" | "none";

export interface CustodyUnlockFormProps {
  /** Demo builds prefill both fields so unlocking is a single click. */
  demoMode?: boolean;
  demoAdminPassword?: string;
  demoPassphrase?: string;
  /** `POST /admin/login` — reports the authoritative custody state alongside the admin session. */
  adminLogin: (password: string) => Promise<AdminLoginResp>;
  /** `POST /admin/unlock` — decrypts the seal for this backend process. */
  unlock: (passphrase: string) => Promise<UnlockResp>;
  /**
   * The custody-admin token the host already holds, if any. When present the form skips
   * `/admin/login` entirely — see the rate-limiter note on `submit`.
   */
  adminToken?: string | null;
  /** Hand a freshly minted custody-admin token back to the host. */
  onAdminToken?: (token: string) => void;
  /** Custody was already unlocked when the admin session was established. */
  onAlreadyUnlocked: () => void;
  /** The passphrase decrypted the seal; `accounts` is the backend's derived signer list. */
  onUnlocked: (accounts: AccountInfo[]) => void;
  /**
   * The router link to /setup, rendered UNDER the form's own no-seal explanation. The host supplies
   * only the link: the explanation itself lives in this component so the dialog and the dedicated
   * /unlock page cannot drift into saying different things about a seal-less instance.
   */
  setupLink: ReactNode;
  /** Label on the submit button. */
  submitLabel?: string;
}

/**
 * The custody-unlock form: passphrase in, unlocked out. Router-free and chrome-free so it can be
 * mounted either as the point-of-need dialog (`CustodyUnlockDialog`) or as the standalone page panel
 * (`CustodyUnlockPanel`).
 *
 * `/admin/unlock` is admin-session gated and `POST /admin/login` is the only endpoint that reports
 * `{initialized, unlocked}`, so the form collects the custody-admin password alongside the
 * passphrase. Neither value is persisted: both live in component state for the duration of a submit,
 * and the passphrase never leaves the request body.
 *
 * Three states, never conflated — the caller cannot distinguish a locked seal from a missing one
 * (both merely mean "the backend cannot sign"), so the form claims nothing until `/admin/login`
 * answers: already unlocked passes straight through, a seal asks for the passphrase, and NO seal
 * shows a Setup/genesis pointer instead of ever prompting for one.
 */
export function CustodyUnlockForm({
  demoMode = false,
  demoAdminPassword = "",
  demoPassphrase = "",
  adminLogin,
  unlock,
  adminToken,
  onAdminToken,
  onAlreadyUnlocked,
  onUnlocked,
  setupLink,
  submitLabel = "Unlock",
}: CustodyUnlockFormProps) {
  const [adminPassword, setAdminPassword] = useState(demoMode ? demoAdminPassword : "");
  const [passphrase, setPassphrase] = useState(demoMode ? demoPassphrase : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [seal, setSeal] = useState<Seal>("unknown");
  /** True once a session has been established this mount, so retries reuse it. */
  const [haveSession, setHaveSession] = useState(Boolean(adminToken));

  /**
   * Establish a custody-admin session ONLY when one is not already held.
   *
   * Load-bearing for brute-force protection: `/admin/login` ends in `record_success(&ip)`, which
   * REMOVES the IP's entry from the shared rate limiter that also guards `/admin/unlock`. Logging in
   * before every attempt would therefore wipe the failure tally between guesses and the per-IP
   * lockout could never trip. Retries reuse the session so consecutive wrong passphrases accumulate.
   */
  async function ensureSession(): Promise<AdminLoginResp | null> {
    if (haveSession) return null;
    const session = await adminLogin(adminPassword);
    onAdminToken?.(session.token);
    setHaveSession(true);
    return session;
  }

  async function attemptUnlock(): Promise<UnlockResp> {
    try {
      return await unlock(passphrase);
    } catch (err) {
      // A DEAD SESSION (not a rejected passphrase) is the one case worth re-authenticating for. The
      // two arrive with the same 401, so they are separated by message — never by status.
      if (!isWrongPassphraseError(err) && (err as { status?: number })?.status === 401) {
        setHaveSession(false);
        const session = await adminLogin(adminPassword);
        onAdminToken?.(session.token);
        setHaveSession(true);
        return await unlock(passphrase);
      }
      throw err;
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const session = await ensureSession();
      if (session?.unlocked) {
        onAlreadyUnlocked();
        return;
      }
      if (session?.initialized === false) {
        setSeal("none");
        return;
      }
      if (session) setSeal("sealed");
      // The caller cannot tell a locked seal from a missing one, so the passphrase field is optional
      // until the backend confirms a seal exists. Now that it has, ask for it rather than spending a
      // rate-limiter attempt on an empty one.
      if (!passphrase) {
        setError("This instance has a seal — enter the unlock passphrase to decrypt it.");
        return;
      }
      const r = await attemptUnlock();
      onUnlocked(r.accounts ?? []);
    } catch (err) {
      // A seal can disappear between the admin login and the unlock (or an older backend may omit
      // `initialized`), so honour the backend's own "not initialized" over the login hint.
      if (isNoSealError(err)) setSeal("none");
      else setError(describe(err));
    } finally {
      setBusy(false);
    }
  }

  if (seal === "none")
    return (
      <div className="space-y-3">
        <p className="text-sm text-muted">
          This instance has no seal yet, so there is nothing to unlock. Run genesis in Setup to
          create the signing key first.
        </p>
        {setupLink}
      </div>
    );

  return (
    <form onSubmit={submit} className="space-y-4">
      {!haveSession && (
        <div className="space-y-1.5">
          <Label htmlFor="custody-admin-pw" required>
            Custody admin password
          </Label>
          <Input
            id="custody-admin-pw"
            type="password"
            autoComplete="off"
            value={adminPassword}
            onChange={(e) => setAdminPassword(e.target.value)}
            required
          />
        </div>
      )}
      <div className="space-y-1.5">
        <Label htmlFor="custody-passphrase" required={seal === "sealed"}>
          Unlock passphrase
        </Label>
        <Input
          id="custody-passphrase"
          type="password"
          autoComplete="off"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          autoFocus
          // Optional until the backend confirms a seal: a first-run operator on a never-genesised
          // instance must not be made to invent a passphrase for a seal that does not exist before
          // the admin password alone can reveal the "Custody not set up" state.
          required={seal === "sealed"}
        />
        {demoMode && <p className="text-xs text-muted">Demo defaults prefilled.</p>}
      </div>
      {error && (
        <p
          role="alert"
          className="flex items-start gap-2 rounded-md border border-danger/30 bg-danger/10 p-2 text-sm text-danger"
        >
          <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{error}</span>
        </p>
      )}
      <Button type="submit" className="w-full" loading={busy}>
        {submitLabel}
      </Button>
    </form>
  );
}

export interface CustodyUnlockDialogProps extends CustodyUnlockFormProps {
  open: boolean;
  /** Dismissed without unlocking — the host resolves its pending action as "still locked". */
  onDismiss: () => void;
  /** What the operator was doing, e.g. "Issue credential", shown so the prompt has context. */
  pendingLabel?: string;
}

/**
 * The PRIMARY unlock surface: an in-place prompt raised at the point of need.
 *
 * When an action needs custody and the backend refuses with "not unlocked", the host opens this over
 * the page the operator is already on. Nothing unmounts, so a half-filled form keeps every typed
 * value, and the host retries the refused request once the seal opens. That is the whole point:
 * unlocking should interrupt the work for one passphrase, not discard it.
 *
 * EVERY exit settles the host's pending action exactly once. Esc, the overlay and the close button
 * all reach `onDismiss` through Radix's `onOpenChange`; unlocking (or discovering custody was
 * already open) settles through the form's own callbacks. The no-seal state is the exception Radix
 * cannot see: following the Setup link is a plain in-app navigation, which leaves this controlled
 * dialog `open` and the host's promise pending forever - the awaited refusal would never reject and
 * the originating page would stay busy. So the link is wrapped in a capture-phase click handler that
 * dismisses BEFORE the navigation runs. Settling twice is harmless by contract, so hosts may treat
 * `onDismiss` as idempotent.
 */
export function CustodyUnlockDialog({
  open,
  onDismiss,
  pendingLabel,
  setupLink,
  ...form
}: CustodyUnlockDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onDismiss()}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <KeyRound className="h-5 w-5 text-primary" /> Custody is locked
          </DialogTitle>
          <DialogDescription>
            {pendingLabel
              ? `${pendingLabel} needs the backend signer. Enter the passphrase to unlock and continue - nothing you have entered is lost.`
              : "This action needs the backend signer. Enter the passphrase to unlock and continue - nothing you have entered is lost."}
          </DialogDescription>
        </DialogHeader>
        <CustodyUnlockForm
          {...form}
          setupLink={<div onClickCapture={onDismiss}>{setupLink}</div>}
          submitLabel="Unlock and continue"
        />
      </DialogContent>
    </Dialog>
  );
}

export interface CustodyUnlockPanelProps extends CustodyUnlockFormProps {
  /** Portal name shown above the title, e.g. "Vet Portal". */
  portalLabel: string;
}

/**
 * The standalone `/unlock` page — the fallback for arriving at an already-locked backend, and the
 * target of a direct link. The point-of-need dialog above is the primary path.
 */
export function CustodyUnlockPanel({ portalLabel, ...form }: CustodyUnlockPanelProps) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <div className="absolute right-4 top-4">
        <ThemeToggle />
      </div>
      <Card className="w-full max-w-sm">
        <CardHeader>
          <div className="mb-2 flex items-center gap-2">
            <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary font-bold text-onPrimary">
              DT
            </span>
            <span className="text-sm font-semibold uppercase tracking-wide text-muted">
              {portalLabel}
            </span>
          </div>
          <CardTitle className="flex items-center gap-2">
            <KeyRound className="h-5 w-5 text-primary" /> Unlock custody
          </CardTitle>
          <CardDescription>
            This backend is not signing — its seed is not loaded. Enter the passphrase to load it.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <CustodyUnlockForm {...form} />
        </CardContent>
      </Card>
    </div>
  );
}

export interface CustodyLockedBannerProps {
  /** Opens the unlock dialog. */
  onUnlock: () => void;
}

/**
 * A non-blocking notice that the backend cannot sign.
 *
 * This is the arrival-time discovery mechanism now that no route redirects: read-only work (records,
 * traceability, verification history) stays reachable while locked, which matters because the
 * operator password and the custody-admin password are SEPARATE credentials — front-desk staff who
 * hold the former but not the latter must not be shut out of the app by a lock they cannot clear.
 */
export function CustodyLockedBanner({ onUnlock }: CustodyLockedBannerProps) {
  return (
    <div className="mb-4 flex flex-wrap items-center gap-3 rounded-lg border border-warning/40 bg-warning/10 px-4 py-3 text-sm">
      <Wand2 className="h-4 w-4 shrink-0 text-warning" />
      <span className="flex-1 text-onSurface">
        Custody is locked — signing and issuance are unavailable until it is unlocked.
      </span>
      <Button size="sm" variant="outline" onClick={onUnlock}>
        Unlock
      </Button>
    </div>
  );
}

/** Operator-readable text for the failures this form can actually produce. */
function describe(err: unknown): string {
  if (isWrongPassphraseError(err)) return "Wrong passphrase — the seal did not decrypt.";
  const status = (err as { status?: number } | null)?.status;
  const raw = (err as Error | null)?.message?.trim() ?? "";
  if (raw.toLowerCase() === "bad password") return "Wrong custody admin password.";
  if (status === 429) return "Too many attempts — wait a moment and try again.";
  return raw || "Unlock failed.";
}
