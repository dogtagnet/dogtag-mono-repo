import { ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "../lib/cn";
import type { LogoState } from "./resolveProfile";

/**
 * A provider's logo, rendered ONLY when the bytes hash to the address the provider published.
 *
 * # The rule
 *
 * **An unverified logo renders nothing.** Not a placeholder, not a broken-image icon, not a generic
 * avatar, not an initials block. A logo is the strongest visual claim of legitimacy in the product,
 * so a stand-in shown for one that could not be verified is precisely how a forged provider comes to
 * look real. Absent is absent.
 *
 * # Why the absence must be STRUCTURAL
 *
 * The way this rule breaks is timing, not logic. An `<img>` whose `src` points at unverified bytes
 * is fetched and may be PAINTED by the browser before any check lands, so a component that mounts
 * the element first and corrects itself afterwards has already shown the thing it exists to
 * withhold. There is therefore no `onError` fallback, no reserved box that later fills, and no
 * CSS-hidden image: while verification is pending or failed, **the element does not exist in the
 * DOM**, and the object URL is minted only from bytes that already verified.
 *
 * `objectUrlFor` is injected so this can be pinned in jsdom, where `URL.createObjectURL` is absent.
 *
 * # Why the two no-image states look different
 *
 * `notPublished` and `unverified` both render no image, so tone is the only thing that separates
 * them — and this repo's standing rule is that facts render neutral while failures warn. A provider
 * that published no logo is ordinary and must read as ordinary; one whose logo does not match its
 * published address is a failure and must read as one, in visible text. A tooltip is not a state:
 * hover survives neither a screenshot nor a touch device.
 */
export interface ProviderLogoProps {
  logo: LogoState | undefined;
  /** The provider's name, for the image's accessible description. Never rendered as a stand-in. */
  providerName: string;
  /** Rendered box size in px. Applied to the image only; the text states are not boxed. */
  size?: number;
  className?: string;
  /**
   * Mint a browser URL for verified bytes. Injected because jsdom has no `URL.createObjectURL`, and
   * because a test must be able to observe that it was NEVER called for unverified bytes.
   */
  objectUrlFor?: (blob: Blob) => string;
  revokeObjectUrl?: (url: string) => void;
}

function defaultObjectUrlFor(blob: Blob): string {
  return URL.createObjectURL(blob);
}

function defaultRevokeObjectUrl(url: string): void {
  URL.revokeObjectURL(url);
}

export function ProviderLogo({
  logo,
  providerName,
  size = 48,
  className,
  objectUrlFor = defaultObjectUrlFor,
  revokeObjectUrl = defaultRevokeObjectUrl,
}: ProviderLogoProps) {
  // `undefined` means the resolution has not finished. It is NOT a fourth rendered state - it is the
  // pending one, and it renders nothing at all, because the only thing worse than a placeholder for
  // an unverified logo is one shown before anybody has looked.
  const verified = logo?.state === "verified" ? logo : null;

  const [url, setUrl] = useState<string | null>(null);

  // The bytes identity, so re-rendering with an equal-but-distinct LogoState does not churn the
  // object URL (which would flicker a correctly-verified logo).
  const bytes = verified?.bytes;
  const mediaType = verified?.mediaType;

  const blob = useMemo(
    () => (bytes && mediaType ? new Blob([bytes as BlobPart], { type: mediaType }) : null),
    [bytes, mediaType],
  );

  useEffect(() => {
    if (!blob) {
      setUrl(null);
      return;
    }
    const next = objectUrlFor(blob);
    setUrl(next);
    return () => {
      revokeObjectUrl(next);
    };
    // `objectUrlFor`/`revokeObjectUrl` are stable in every real caller and injected only by tests.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blob]);

  if (verified && url) {
    return (
      <img
        src={url}
        alt={`${providerName} logo`}
        width={size}
        height={size}
        data-testid="provider-logo"
        data-logo-state="verified"
        className={cn("rounded-md object-contain", className)}
        style={{ width: size, height: size }}
      />
    );
  }

  if (logo?.state === "unverified") {
    return (
      <div
        data-testid="provider-logo-unverified"
        data-logo-state="unverified"
        className={cn(
          "flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-400",
          className,
        )}
      >
        <ShieldAlert className="mt-px h-3.5 w-3.5 shrink-0" aria-hidden />
        <span>
          <span className="font-medium">Logo not shown.</span>{" "}
          <span className="opacity-90">{logo.reason}</span>
        </span>
      </div>
    );
  }

  if (logo?.state === "notPublished") {
    return (
      <div
        data-testid="provider-logo-absent"
        data-logo-state="notPublished"
        className={cn("text-xs text-muted-foreground", className)}
      >
        No logo published
      </div>
    );
  }

  // Pending, or verified-but-the-object-URL-has-not-been-minted-yet. Nothing is claimed either way.
  return null;
}
