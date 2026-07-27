import { cn } from "../lib/cn";
import {
  bindingExplanation,
  bindingLine,
  bindingTone,
  type IssuerDomainBinding,
} from "../domain/issuerDomainBinding";

/**
 * The issuer↔domain badge: one compact line that sits BESIDE an issuer, not a panel.
 *
 * # What it says
 *
 * The observation, and nothing more. `verified` is a small green check plus "this address is listed in
 * <domain>'s DNS records", with the description the domain owner published. `notListed` is the
 * symmetric red line: same size, same placement, same register — "this address is not listed in
 * <domain>'s DNS records". No "VERIFICATION FAILED", no "INVALID", no "UNTRUSTED", nothing that reads as
 * a judgement on the credential or the organisation, because that is not what the check establishes.
 *
 * A missing record means the domain owner has not published the binding. The credential's validity is
 * separately and genuinely proven on-chain, and conflating the two would tell someone their perfectly
 * valid credential had failed.
 *
 * `couldNotCheck` stays NEUTRAL — neither green nor red. A resolver timeout says nothing whatsoever
 * about the issuer, so colouring it as a failure would be a lie of emphasis.
 */
export interface DomainBindingBadgeProps {
  binding: IssuerDomainBinding | null | undefined;
  /**
   * Render nothing at all when there is no domain claim on-chain. Off by default: a quiet line is
   * better than silence, because an issuer shown with NO badge must not read as verified.
   */
  hideWhenNoClaim?: boolean;
  className?: string;
  "data-testid"?: string;
}

const TONE_CLASS = {
  positive: "text-success",
  negative: "text-danger",
  neutral: "text-muted",
  pending: "text-muted",
} as const;

/** A small inline check. Sized to the text, so the badge stays a line rather than a chip. */
function CheckMark() {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.25"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M3 8.5l3.5 3.5L13 4.5" />
    </svg>
  );
}

/** A small inline dash — deliberately not a cross or a warning triangle. */
function NotListedMark() {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.25"
      strokeLinecap="round"
    >
      <circle cx="8" cy="8" r="6" strokeWidth="1.75" />
      <path d="M5 8h6" />
    </svg>
  );
}

function NeutralMark() {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
    >
      <circle cx="8" cy="8" r="6" />
      <path d="M8 5.5v.5M8 8v3" />
    </svg>
  );
}

/** A distinct mark for a provenance failure: the observation is about the contract, not a DNS zone, so
 *  it must not look identical to "not listed". */
function NotAnIssuerMark() {
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M2.5 13.5V6l5.5-3.5L13.5 6v7.5" />
      <path d="M5.5 13.5v-4h5v4" />
    </svg>
  );
}

function Mark({ tone, state }: { tone: ReturnType<typeof bindingTone>; state: string }) {
  if (tone === "positive") return <CheckMark />;
  if (state === "notADogTagIssuer") return <NotAnIssuerMark />;
  if (tone === "negative") return <NotListedMark />;
  if (tone === "pending")
    return (
      <span
        aria-hidden="true"
        className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent"
      />
    );
  return <NeutralMark />;
}

export function DomainBindingBadge({
  binding,
  hideWhenNoClaim = false,
  className,
  "data-testid": testId,
}: DomainBindingBadgeProps) {
  if (!binding) return null;
  if (hideWhenNoClaim && binding.state === "noDomainClaimed") return null;

  const tone = bindingTone(binding.state);
  const line = bindingLine(binding);
  const description = binding.state === "verified" ? binding.description?.trim() : undefined;

  return (
    <span
      data-testid={testId ?? "domain-binding-badge"}
      data-state={binding.state}
      className={cn("inline-flex max-w-full flex-col gap-0.5 text-xs leading-snug", className)}
    >
      <span
        className={cn("inline-flex items-center gap-1.5", TONE_CLASS[tone])}
        title={bindingExplanation(binding)}
      >
        <Mark tone={tone} state={binding.state} />
        <span className="min-w-0 break-words">{line}</span>
      </span>
      {description && (
        // The whole reason the TXT value is free-form: show what the domain chose to say about the
        // address. Quoted so it reads as the domain's words, not ours.
        <span data-testid="domain-binding-description" className="min-w-0 break-words pl-5 text-muted">
          Domain says: “{description}”
        </span>
      )}
    </span>
  );
}
