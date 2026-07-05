import type { ActivityEventType } from "@dogtag/ui";

/** Presentation metadata for each on-chain event kind (label + badge colour + one-line meaning). */
export interface EventMeta {
  label: string;
  /** Badge variant conveying the event's operational tone. */
  variant: "default" | "neutral" | "success" | "warning" | "danger";
  /** Plain-language description of what the event means (row subtitle / filter hint). */
  meaning: string;
}

/** The seven oversight event kinds, in the order they appear in a credential's lifecycle. */
export const EVENT_TYPES: ActivityEventType[] = [
  "issuerCreated",
  "rootRegistered",
  "whitelisted",
  "delisted",
  "rootIssued",
  "rootRevoked",
  "verified",
];

export const EVENT_META: Record<ActivityEventType, EventMeta> = {
  issuerCreated: {
    label: "Issuer created",
    variant: "default",
    meaning: "A new issuer clone was deployed from the factory.",
  },
  rootRegistered: {
    label: "Root registered",
    variant: "neutral",
    meaning: "A credential root was bound to its issuer clone (write-once).",
  },
  whitelisted: {
    label: "Whitelisted",
    variant: "success",
    meaning: "A signer was granted the right to issue or verify a record type.",
  },
  delisted: {
    label: "Delisted",
    variant: "warning",
    meaning: "A signer's issue/verify right was revoked.",
  },
  rootIssued: {
    label: "Root issued",
    variant: "success",
    meaning: "A credential was issued (a new root anchored on-chain).",
  },
  rootRevoked: {
    label: "Root revoked",
    variant: "danger",
    meaning: "A previously-issued credential was revoked.",
  },
  verified: {
    label: "Verified",
    variant: "default",
    meaning: "A consent-gated verification was recorded against a credential.",
  },
};

/** Fallback metadata for an unknown/forward-compatible event kind. */
export function eventMeta(type: string): EventMeta {
  return (
    EVENT_META[type as ActivityEventType] ?? {
      label: type,
      variant: "neutral",
      meaning: "On-chain event.",
    }
  );
}

const RELATIVE = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
const DIVISIONS: [number, Intl.RelativeTimeFormatUnit][] = [
  [60, "seconds"],
  [60, "minutes"],
  [24, "hours"],
  [7, "days"],
  [4.34524, "weeks"],
  [12, "months"],
  [Number.POSITIVE_INFINITY, "years"],
];

/** A compact relative time ("3 min ago") for a unix-seconds block timestamp, computed against `now`. */
export function relativeTime(unixSeconds: number, now: number = Date.now()): string {
  if (!unixSeconds) return "-";
  let duration = unixSeconds - Math.floor(now / 1000);
  for (const [amount, unit] of DIVISIONS) {
    if (Math.abs(duration) < amount) return RELATIVE.format(Math.round(duration), unit);
    duration /= amount;
  }
  return "-";
}

/** The absolute local timestamp for a unix-seconds value (tooltip / secondary text). */
export function absoluteTime(unixSeconds: number): string {
  if (!unixSeconds) return "-";
  return new Date(unixSeconds * 1000).toLocaleString();
}
