/**
 * Custody lock detection + unlock-route helpers, shared by every portal whose backend holds a
 * custody seal (vet, groomer — the government/admin backends have no seal, see §11.4).
 *
 * The backends expose exactly three custody states and the portal must never conflate them:
 *   - NO SEAL      — genesis has never run. Only the Setup wizard can fix this.
 *   - SEALED+LOCKED— a seal exists (hydrated from CUSTODY_SEAL_PATH after a restart) but the seed is
 *                    not decrypted. The passphrase fixes this, in the point-of-need unlock dialog or
 *                    on the dedicated /unlock page.
 *   - UNLOCKED     — normal operation.
 *
 * Everything here is pure so it can be unit-tested without a DOM or a backend.
 */

/** The dedicated unlock route every custody portal mounts. */
export const UNLOCK_PATH = "/unlock";

/** Query param carrying the destination the operator was heading to when the lock was detected. */
export const NEXT_PARAM = "next";

/**
 * Tri-state view of the backend custody, as the portal currently understands it.
 *
 * `unknown` is the important one: it means "we have not been told otherwise". The portal must never
 * announce a lock on `unknown` — a backend that is merely down or slow is not a locked one, and no
 * passphrase can fix it.
 */
export type CustodyState = "unknown" | "locked" | "unlocked";

/**
 * True when `err` is the backend's "custody is locked" signal.
 *
 * Matched on the MESSAGE, not the status: the backends surface the same `not unlocked` text as a 409
 * from the explicit route guards (`routes.rs`, `verify.rs`) and as a 500 from `CustodyError::Locked`
 * when a signer is derived lazily. Matching the status alone would also be wrong in the other
 * direction — 409 is overloaded (`already initialized`, `not initialized`, `no pending genesis`), and
 * treating `not initialized` as "locked" would bounce a seal-less instance back to /unlock forever.
 */
export function isCustodyLockedError(err: unknown): boolean {
  return custodyErrorMessage(err) === "not unlocked";
}

/**
 * True when `err` is the backend's "this instance has no seal yet" signal (409 from /admin/unlock).
 * That state needs genesis in the Setup wizard, never a passphrase.
 */
export function isNoSealError(err: unknown): boolean {
  return custodyErrorMessage(err) === "not initialized";
}

/**
 * True when `err` is `/admin/unlock`'s WRONG-PASSPHRASE refusal.
 *
 * That route answers a bad passphrase with 401, the same status its admin gate uses for `missing
 * admin session` / `invalid admin session`. Only the MESSAGE separates a rejected credential from a
 * dead session, so the shared client keys its stale-session hook on this predicate rather than on the
 * path - a path-wide exemption would swallow the genuine dead-session 401s the same route emits.
 */
export function isWrongPassphraseError(err: unknown): boolean {
  return custodyErrorMessage(err) === "wrong passphrase";
}

/** Lower-cased error text from an ApiError body (`{ error }`) or a plain Error message. */
function custodyErrorMessage(err: unknown): string {
  if (!err || typeof err !== "object") return "";
  const body = (err as { body?: unknown }).body;
  if (body && typeof body === "object" && "error" in body) {
    return String((body as { error: unknown }).error).trim().toLowerCase();
  }
  const message = (err as { message?: unknown }).message;
  return typeof message === "string" ? message.trim().toLowerCase() : "";
}

/**
 * Classify custody from a `GET /issuer/signers` response — the on-load probe.
 *
 * That route is operator-gated, read-only, and already exists, so the portal learns the custody state
 * without a new endpoint and without an admin session. It emits exactly two shapes: `{ signers: [] }`
 * from the locked short-circuit, and `{ activeSigner, matrix }` when unlocked.
 *
 * BOTH shapes are recognised positively and everything else stays `unknown`: an unrecognised payload
 * (a stubbed test double, a proxy error page, a shape a later backend adds) must not be read as a
 * lock. `activeSigner` is keyed on PRESENCE rather than truthiness — `active_address()` falls back to
 * `""` — since a false "locked" would nag the operator about a lock that is not there, whereas a false
 * "unlocked" costs nothing (the first real request 409s and corrects the state).
 */
export function custodyStateFromSigners(resp: unknown): CustodyState {
  if (!resp || typeof resp !== "object") return "unknown";
  if ("signers" in resp) return "locked";
  if ("activeSigner" in resp) return "unlocked";
  return "unknown";
}

/**
 * Sanitize a `?next=` destination into a safe same-origin path.
 *
 * Rejects anything that is not a plain absolute path (`//evil.com` and `https://…` are open-redirect
 * vectors once this value reaches `navigate()`), and rejects the unlock route itself so a successful
 * unlock can never bounce back to the page it just left.
 */
export function sanitizeNextPath(raw: string | null | undefined, fallback: string): string {
  if (!raw) return fallback;
  // Deliberately NOT decoded here: both call sites already hand over a decoded string (buildUnlockPath
  // takes a live `pathname + search`, and the unlock page reads the param through URLSearchParams,
  // which decodes). Decoding again would turn a destination's own escapes into structure — `%2B`
  // becoming a literal `+`, `%26` splitting off a second query parameter.
  const candidate = raw;
  if (!candidate.startsWith("/")) return fallback;
  // "//host" and "/\host" are protocol-relative URLs, not paths.
  if (candidate.startsWith("//") || candidate.startsWith("/\\")) return fallback;
  const path = candidate.split(/[?#]/)[0] ?? "";
  if (path === UNLOCK_PATH || path.startsWith(`${UNLOCK_PATH}/`)) return fallback;
  return candidate;
}

/** Build the unlock URL that remembers where the operator was heading. */
export function buildUnlockPath(from: string): string {
  const next = sanitizeNextPath(from, "");
  return next ? `${UNLOCK_PATH}?${NEXT_PARAM}=${encodeURIComponent(next)}` : UNLOCK_PATH;
}
