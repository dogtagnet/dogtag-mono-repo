import { Check, Copy, ExternalLink } from "lucide-react";
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

/** Copy `value` to the clipboard, reporting success so the button can confirm it. */
async function writeClipboard(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
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
 */
export function CopyButton({ value, label = "value", className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const onCopy = useCallback(() => {
    void writeClipboard(value).then((ok) => {
      if (!ok) return;
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  }, [value]);

  return (
    <button
      type="button"
      onClick={onCopy}
      data-testid="copy-button"
      data-copied={copied ? "true" : "false"}
      aria-label={copied ? `${label} copied` : `Copy full ${label}`}
      title={copied ? "Copied" : `Copy full ${label}`}
      className={cn(
        "inline-flex shrink-0 items-center rounded p-0.5 text-muted transition-colors hover:text-onSurface focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary",
        className,
      )}
    >
      {copied ? (
        <Check className="h-3 w-3 text-success" aria-hidden />
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
  /** Prefix truncation length; the tail is fixed at 6. */
  head?: number;
  /**
   * Put the label on its own line above the value. Long labels ("verification registry") cannot
   * shrink inline without being cut, so in a dense column they would widen the whole table; stacking
   * keeps the column narrow while the label stays fully readable.
   */
  stacked?: boolean;
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
  className,
  testId,
}: ChainValueProps) {
  if (!value) {
    return (
      <span className={cn("text-[11px] text-muted", className)} data-testid={testId}>
        {label} —
      </span>
    );
  }
  const short = shortHex(value, head);
  return (
    <span
      className={cn(
        "max-w-full gap-1 text-[11px]",
        stacked ? "flex flex-col" : "inline-flex items-center",
        className,
      )}
      data-testid={testId}
      data-value={value}
    >
      <span className={cn("text-muted", stacked ? "truncate" : "shrink-0")} title={label}>
        {label}
      </span>
      <span className="inline-flex max-w-full items-center gap-1">
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noreferrer"
            className="truncate font-mono text-primary hover:underline"
            title={value}
            data-testid={testId ? `${testId}-link` : undefined}
          >
            {short}
          </a>
        ) : (
          <span className="truncate font-mono text-onSurface" title={value}>
            {short}
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
 */
export function ProvenanceBadge({ provenance, className }: ProvenanceBadgeProps) {
  if (provenance === "onchain") {
    return (
      <Badge
        variant="neutral"
        className={className}
        data-testid="provenance-onchain"
        title="This event carries a well-formed 32-byte transaction hash and is addressable on the block explorer."
      >
        on-chain
      </Badge>
    );
  }
  return (
    <Badge
      variant="warning"
      className={className}
      data-testid="provenance-synthetic"
      title="This event's transaction hash is not a well-formed 32-byte value, so no such transaction exists on any chain and there is no explorer page to link to. The usual cause is a scripted/demo indexer feed."
    >
      not on chain
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
  className?: string;
  testId?: string;
}

/**
 * The transaction reference. When the event is chain-addressable this is an explorer link; when it is
 * not, it is deliberately INERT - a synthetic row must not offer a link that pretends to resolve.
 */
export function TxRef({ event, href, className, testId = "chain-tx" }: TxRefProps) {
  const hash = event.txHash;
  if (!hash) {
    return (
      <span className={cn("text-xs text-muted", className)} data-testid={`${testId}-none`}>
        no tx
      </span>
    );
  }
  const synthetic = chainProvenance(event) !== "onchain";
  return (
    <span className={cn("inline-flex max-w-full items-center gap-1", className)}>
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 truncate font-mono text-xs text-primary hover:underline"
          title={hash}
          data-testid={testId}
        >
          {shortHex(hash)}
          <ExternalLink className="h-3 w-3 shrink-0" aria-hidden />
        </a>
      ) : (
        <span
          className="truncate font-mono text-xs text-muted line-through decoration-warning/60"
          title={
            synthetic
              ? `${hash} — not a well-formed 32-byte transaction hash, so it addresses no transaction on any chain`
              : hash
          }
          data-testid={`${testId}-inert`}
        >
          {shortHex(hash)}
        </span>
      )}
      <CopyButton value={hash} label="transaction hash" />
    </span>
  );
}
