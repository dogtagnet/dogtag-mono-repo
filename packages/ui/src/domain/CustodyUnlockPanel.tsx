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
import { Input } from "../components/Input";
import { Label } from "../components/Label";
import { ThemeToggle } from "../theme/ThemeToggle";
import type { AccountInfo, AdminLoginResp, UnlockResp } from "../api/types";
import { isNoSealError } from "../custody/lock";

export interface CustodyUnlockPanelProps {
  /** Portal name shown above the title, e.g. "Vet Portal". */
  portalLabel: string;
  /** Demo builds prefill both fields so the unlock is a single click. */
  demoMode?: boolean;
  demoAdminPassword?: string;
  demoPassphrase?: string;
  /** `POST /admin/login` — reports the authoritative custody state alongside the admin session. */
  adminLogin: (password: string) => Promise<AdminLoginResp>;
  /** `POST /admin/unlock` — decrypts the seal for this backend process. */
  unlock: (passphrase: string) => Promise<UnlockResp>;
  /** Custody was already unlocked when the admin session was established. */
  onAlreadyUnlocked: () => void;
  /** The passphrase decrypted the seal; `accounts` is the backend's derived signer list. */
  onUnlocked: (accounts: AccountInfo[]) => void;
  /** Called after `onAlreadyUnlocked`/`onUnlocked` is handled, if the host wants the admin token. */
  onAdminToken?: (token: string) => void;
  /** Rendered in the no-seal state — the host supplies its own router link to /setup. */
  setupLink: ReactNode;
}

/**
 * The focused custody-unlock surface: passphrase in, unlocked out. Router-free so every portal can
 * mount it on its own `/unlock` route.
 *
 * It asks for the custody-admin password as well as the passphrase because `/admin/unlock` is
 * admin-session gated — the two are submitted together so the operator sees ONE form and ONE click.
 * Neither value is persisted anywhere: they live in component state for the duration of the submit
 * and the passphrase never leaves the request body.
 *
 * `/admin/login` is what distinguishes the three custody states authoritatively, so the panel routes
 * on its response rather than guessing: already unlocked → straight through; sealed → passphrase;
 * NO seal → a Setup/genesis pointer, never a passphrase prompt.
 */
export function CustodyUnlockPanel({
  portalLabel,
  demoMode = false,
  demoAdminPassword = "",
  demoPassphrase = "",
  adminLogin,
  unlock,
  onAlreadyUnlocked,
  onUnlocked,
  onAdminToken,
  setupLink,
}: CustodyUnlockPanelProps) {
  const [adminPassword, setAdminPassword] = useState(demoMode ? demoAdminPassword : "");
  const [passphrase, setPassphrase] = useState(demoMode ? demoPassphrase : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /**
   * What we have CONFIRMED about the seal, via `/admin/login`. The gate that sends the operator here
   * cannot tell a locked seal from a missing one — both simply mean "the backend cannot sign" — so
   * the panel starts at `unknown` and only claims "locked" or "not set up" once the backend says so.
   */
  const [seal, setSeal] = useState<"unknown" | "sealed" | "none">("unknown");

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const session = await adminLogin(adminPassword);
      onAdminToken?.(session.token);
      if (session.unlocked) {
        onAlreadyUnlocked();
        return;
      }
      if (session.initialized === false) {
        setSeal("none");
        return;
      }
      setSeal("sealed");
      const r = await unlock(passphrase);
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
          {seal === "none" ? (
            <>
              <CardTitle className="flex items-center gap-2">
                <Wand2 className="h-5 w-5 text-primary" /> Custody not set up
              </CardTitle>
              <CardDescription>
                This instance has no seal yet, so there is nothing to unlock. Run genesis in Setup to
                create the signing key first.
              </CardDescription>
            </>
          ) : seal === "sealed" ? (
            <>
              <CardTitle className="flex items-center gap-2">
                <KeyRound className="h-5 w-5 text-primary" /> Custody is locked
              </CardTitle>
              <CardDescription>
                The seal is present but the seed is encrypted. Enter the passphrase to decrypt it for
                this backend process.
              </CardDescription>
            </>
          ) : (
            <>
              <CardTitle className="flex items-center gap-2">
                <KeyRound className="h-5 w-5 text-primary" /> Unlock custody
              </CardTitle>
              <CardDescription>
                This backend is not signing — its seed is not loaded. Enter the custody admin password
                and the unlock passphrase to load it.
              </CardDescription>
            </>
          )}
        </CardHeader>
        <CardContent>
          {seal === "none" ? (
            setupLink
          ) : (
            <form onSubmit={submit} className="space-y-4">
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
              <div className="space-y-1.5">
                <Label htmlFor="custody-passphrase" required>
                  Unlock passphrase
                </Label>
                <Input
                  id="custody-passphrase"
                  type="password"
                  autoComplete="off"
                  value={passphrase}
                  onChange={(e) => setPassphrase(e.target.value)}
                  autoFocus
                  required
                />
                {demoMode && (
                  <p className="text-xs text-muted">Demo defaults prefilled — just click Unlock.</p>
                )}
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
                Unlock
              </Button>
            </form>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

/** Operator-readable text for the failures this form can actually produce. */
function describe(err: unknown): string {
  const status = (err as { status?: number } | null)?.status;
  const raw = (err as Error | null)?.message?.trim() ?? "";
  const msg = raw.toLowerCase();
  if (msg === "wrong passphrase") return "Wrong passphrase — the seal did not decrypt.";
  if (msg === "bad password") return "Wrong custody admin password.";
  if (status === 429) return "Too many attempts — wait a moment and try again.";
  return raw || "Unlock failed.";
}
