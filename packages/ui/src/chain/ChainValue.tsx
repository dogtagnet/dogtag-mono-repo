import { Check, Copy, ExternalLink, X } from "lucide-react";
import { useCallback, useState } from "react";
import { Badge } from "../components/Badge";
import { cn } from "../lib/cn";
import {
  chainProvenance,
  formatChainTime,
  formatRelativeTime,
  shortHex,
  type ChainProvenance,
} from "./provenance";

/**
 * Shared audit-surface primitives for the on-chain activity tables (government Oversight, vet and
 * groomer Traceability). One implementation so the portals cannot drift into separate dialects of
 * "here is the transaction" - see `./provenance.ts` for why chain-addressability is gated.
 */

/**
 * The pre-`navigator.clipboard` copy path: a hidden textarea plus `document.execCommand("copy")`.
 *
 * Not legacy-browser nostalgia - `navigator.clipboard` is undefined in any NON-SECURE context, and
 * these portals are routinely served over plain `http://` on a LAN address. Without this the button
 * is dead in exactly the deployment topology the demo stacks use.
 */
function execCommandCopy(value: string): boolean {
  if (typeof document === "undefined" || !document.body) return false;
  const ta = document.createElement("textarea");
  ta.value = value;
  ta.setAttribute("readonly", "");
  ta.style.position = "fixed";
  ta.style.top = "0";
  ta.style.left = "0";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  try {
    ta.focus();
    ta.select();
    ta.setSelectionRange(0, value.length);
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    document.body.removeChild(ta);
  }
}

/** Copy `value` to the clipboard, reporting success so the button can confirm - or admit - it. */
async function writeClipboard(value: string): Promise<boolean> {
  try {
    if (navigator?.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Permission denied or a non-secure context: fall through to the execCommand path.
  }
  return execCommandCopy(value);
}

export interface CopyButtonProps {
  value: string;
  /** What is being copied, for the accessible label ("root", "transaction hash"). */
  label?: string;
  className?: string;
}

/**
 * The copy affordance that makes a truncated identifier usable. Truncation without this is data loss:
 * the reader can see there is a root but can never quote it.
 *
 * A failure is therefore SHOWN, never swallowed. `shortHex` truncates the string itself, so the elided
 * characters are not in the DOM at all - a silently inert button would leave the full value
 * unreachable while looking perfectly functional, which on an audit surface is the worst of both.
 */
export function CopyButton({ value, label = "value", className }: CopyButtonProps) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const onCopy = useCallback(() => {
    void writeClipboard(value).then((ok) => {
      setState(ok ? "copied" : "failed");
      setTimeout(() => setState("idle"), ok ? 1200 : 3000);
    });
  }, [value]);

  // The failure message may only name a fallback that exists on EVERY consumer: the row expansion is
  // government-only, whereas the value's own hover text carries the full string on both portals.
  const title =
    state === "copied"
      ? "Copied"
      : state === "failed"
        ? `Could not copy — this browser blocks clipboard access here (usually a non-secure http:// origin). Hover the value to read the full ${label}.`
        : `Copy full ${label}`;

  return (
    <button
      type="button"
      onClick={onCopy}
      data-testid="copy-button"
      data-copied={state === "copied" ? "true" : "false"}
      data-copy-state={state}
      aria-label={
        state === "copied"
          ? `${label} copied`
          : state === "failed"
            ? `Could not copy ${label}`
            : `Copy full ${label}`
      }
      title={title}
      className={cn(
        "inline-flex shrink-0 items-center rounded p-0.5 text-muted transition-colors hover:text-onSurface focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary",
        className,
      )}
    >
      {state === "copied" ? (
        <Check className="h-3 w-3 text-success" aria-hidden />
      ) : state === "failed" ? (
        <X className="h-3 w-3 text-danger" aria-hidden />
      ) : (
        <Copy className="h-3 w-3" aria-hidden />
      )}
    </button>
  );
}

export interface ChainValueProps {
  /**
   * What this value IS ("root", "record type key", "nullifier"). Required, and deliberately so: a
   * bare 32-byte hex in an audit table is unreadable, and every one of them looks alike.
   */
  label: string;
  value?: string | null;
  /** Explorer link, when the value is chain-addressable. `null`/absent renders plain text. */
  href?: string | null;
  /** Prefix truncation length; the tail is fixed at 6. Ignored when `full` is set. */
  head?: number;
  /**
   * Put the label on its own line above the value. Long labels ("verification registry") cannot
   * shrink inline without being cut, so in a dense column they would widen the whole table; stacking
   * keeps the column narrow while the label stays fully readable.
   */
  stacked?: boolean;
  /**
   * Render the value COMPLETE, wrapping across lines rather than middle-truncating. For the row
   * expansion, whose whole promise is the untruncated value: dropping the truncation alone would not
   * do it, because the `truncate` class would still clip the value in CSS, so this swaps in
   * `break-all` too.
   */
  full?: boolean;
  /**
   * Keep the label for the copy button's accessible name but do not render it, for a caller that
   * already prints the label itself (the expansion panel's row label). Never a way to ship an
   * unlabelled hex - the label prop stays required.
   */
  labelHidden?: boolean;
  className?: string;
  testId?: string;
}

/**
 * One labelled, truncated, copyable on-chain identifier - optionally linked to the explorer.
 *
 * The label is a required prop rather than an option, because the failure this component exists to
 * fix is precisely an unlabelled hex string.
 */
export function ChainValue({
  label,
  value,
  href,
  head = 10,
  stacked = false,
  full = false,
  labelHidden = false,
  className,
  testId,
}: ChainValueProps) {
  if (!value) {
    return (
      <span className={cn("text-[11px] text-muted", className)} data-testid={testId}>
        {labelHidden ? "—" : `${label} —`}
      </span>
    );
  }
  const shown = full ? value : shortHex(value, head);
  // `truncate` clips in CSS, so the full form has to opt out of it as well as out of `shortHex`.
  const valueClass = full ? "break-all" : "truncate";
  return (
    <span
      className={cn(
        "min-w-0 max-w-full gap-1 text-[11px]",
        stacked ? "flex flex-col" : "inline-flex items-center",
        className,
      )}
      data-testid={testId}
      data-value={value}
    >
      {!labelHidden && (
        <span className={cn("text-muted", stacked ? "truncate" : "shrink-0")} title={label}>
          {label}
        </span>
      )}
      <span className="inline-flex min-w-0 max-w-full items-center gap-1">
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noreferrer"
            className={cn("font-mono text-primary hover:underline", valueClass)}
            title={value}
            data-testid={testId ? `${testId}-link` : undefined}
          >
            {shown}
          </a>
        ) : (
          <span className={cn("font-mono text-onSurface", valueClass)} title={value}>
            {shown}
          </span>
        )}
        <CopyButton value={value} label={label} />
      </span>
    </span>
  );
}

export interface ProvenanceBadgeProps {
  provenance: ChainProvenance;
  className?: string;
}

/**
 * The per-ROW provenance verdict. A header badge cannot do this job: a feed can mix a real event with
 * a seeded one, and in an audit view the two must never look alike.
 *
 * BOTH labels state only what was actually checked - the SHAPE of the transaction hash. No chain read
 * happens anywhere on this path, so the positive side says the hash is chain-ADDRESSABLE (it can be
 * looked up), never that the transaction exists: asserting existence from arithmetic would be the same
 * over-claiming this whole module exists to remove. The two sides are deliberately symmetric.
 */
export function ProvenanceBadge({ provenance, className }: ProvenanceBadgeProps) {
  if (provenance === "onchain") {
    return (
      <Badge
        variant="neutral"
        className={className}
        data-testid="provenance-onchain"
        title="This event's transaction hash is a well-formed 32-byte value, so it can be looked up on the block explorer. Checked here: the shape of the hash only — this page does not read the chain, so it is not a confirmation that the transaction exists."
      >
        chain-addressable
      </Badge>
    );
  }
  return (
    <Badge
      variant="warning"
      className={className}
      data-testid="provenance-synthetic"
      title="This event's transaction hash is not a well-formed 32-byte value, so it addresses no transaction on any chain and there is no explorer page to link to. The usual cause is a scripted/demo indexer feed."
    >
      not chain-addressable
    </Badge>
  );
}

export interface ChainTimeProps {
  /** The on-chain block timestamp, in unix SECONDS. */
  seconds?: number | null;
  className?: string;
  testId?: string;
}

/**
 * When an event happened, absolute-first. The relative form is an affordance for scanning; the
 * absolute form with a timezone is the one an auditor can actually cite, so it is the primary line.
 */
export function ChainTime({ seconds, className, testId }: ChainTimeProps) {
  if (!seconds) {
    return (
      <span className={cn("text-xs text-muted", className)} data-testid={testId}>
        —
      </span>
    );
  }
  const relative = formatRelativeTime(seconds);
  return (
    <span className={cn("flex flex-col", className)} data-testid={testId}>
      <span className="whitespace-nowrap text-[11px] text-onSurface">{formatChainTime(seconds)}</span>
      {relative && <span className="text-[10px] text-muted">{relative}</span>}
    </span>
  );
}

export interface TxRefProps {
  event: { txHash?: string | null; txUrl?: string | null };
  /** The resolved explorer href, or `null` when the event is not chain-addressable. */
  href: string | null;
  /** Render the hash complete, wrapping rather than middle-truncating (the row expansion). */
  full?: boolean;
  className?: string;
  testId?: string;
}

/**
 * The transaction reference. When the event is chain-addressable this is an explorer link; when it is
 * not, it is deliberately INERT - a synthetic row must not offer a link that pretends to resolve.
 */
export function TxRef({ event, href, full = false, className, testId = "chain-tx" }: TxRefProps) {
  const hash = event.txHash;
  if (!hash) {
    return (
      <span className={cn("text-xs text-muted", className)} data-testid={`${testId}-none`}>
        no tx
      </span>
    );
  }
  const synthetic = chainProvenance(event) !== "onchain";
  const shown = full ? hash : shortHex(hash);
  const valueClass = full ? "break-all" : "truncate";
  return (
    <span className={cn("inline-flex min-w-0 max-w-full items-center gap-1", className)}>
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className={cn(
            "inline-flex items-center gap-1 font-mono text-xs text-primary hover:underline",
            valueClass,
          )}
          title={hash}
          data-testid={testId}
        >
          {shown}
          <ExternalLink className="h-3 w-3 shrink-0" aria-hidden />
        </a>
      ) : (
        <span
          className={cn(
            "font-mono text-xs text-muted line-through decoration-warning/60",
            valueClass,
          )}
          title={
            synthetic
              ? `${hash} — not a well-formed 32-byte transaction hash, so it addresses no transaction on any chain`
              : hash
          }
          data-testid={`${testId}-inert`}
        >
          {shown}
        </span>
      )}
      <CopyButton value={hash} label="transaction hash" />
    </span>
  );
}
