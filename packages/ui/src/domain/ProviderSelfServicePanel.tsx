/**
 * The provider self-service surface (registry-plan S-15).
 *
 * Renders the four flows against the generation-2 registry set: deploy your own clone, select which
 * of your contracts is current, claim a domain, and publish your pin, contacts and profile.
 *
 * The whole component is a RENDERER. Every verdict comes from `../provider`, which is pure and
 * reader-injected and separately tested; nothing here re-derives an answer. That split is why the
 * hard claims - a forged address is refused, a blank location publishes no pin - are pinned by
 * hermetic tests rather than resting on a browser someone remembered to open.
 *
 * Two rendering rules are load-bearing:
 *
 *   * `could-not-run` renders as its own state, never as a softened failure and never as a pass. A
 *     check that did not run is not a check that passed, and it is not an accusation either.
 *   * every `could-not-run` prints its reason inline. A reason reachable only by hovering is not
 *     reported - this repo's standing rule, and the reason the bench prints findings inline too.
 */

import { AlertTriangle, CircleHelp, CircleSlash, Loader2 } from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "../components/Badge";
import { cn } from "../lib/cn";
import type {
  CheckOutcome,
  CloneAssessment,
  CloneLifecycle,
  DeployPlan,
  DirectoryPublicationPlan,
  DomainClaimAssessment,
  ProviderCheck,
  ProviderVerdict,
} from "../provider";

// -------------------------------------------------------------------------------------------------
// Check rows
// -------------------------------------------------------------------------------------------------

const OUTCOME_STYLE: Readonly<Record<CheckOutcome, { label: string; className: string }>> = {
  pass: { label: "passed", className: "text-emerald-700 dark:text-emerald-400" },
  fail: { label: "failed", className: "text-red-700 dark:text-red-400" },
  // Amber and its own word. Deliberately NOT a muted grey: a question nobody could answer is a gap
  // in what is known, and rendering it as neutral invites reading it as fine.
  "could-not-run": { label: "could not run", className: "text-amber-700 dark:text-amber-400" },
};

export function ProviderCheckRow({ check }: { check: ProviderCheck }): ReactNode {
  const style = OUTCOME_STYLE[check.outcome];
  return (
    <li className="border-b border-border/60 py-2 last:border-0" data-testid={`check-${check.id}`}>
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm">{check.question}</span>
        <span className={cn("shrink-0 text-xs font-medium uppercase tracking-wide", style.className)}>
          {style.label}
        </span>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{check.finding}</p>
      {check.couldNotRunReason ? (
        // Printed, not hovered. This is the sentence that distinguishes "we could not ask" from
        // "the answer was no", and it is useless if a reader has to discover it.
        <p className="mt-1 text-xs text-amber-700 dark:text-amber-400" data-testid="could-not-run-reason">
          Why it could not run: {check.couldNotRunReason}
        </p>
      ) : null}
    </li>
  );
}

export function ProviderCheckList({ checks }: { checks: readonly ProviderCheck[] }): ReactNode {
  if (checks.length === 0) return null;
  return (
    <ul className="mt-3" data-testid="provider-checks">
      {checks.map((c) => (
        <ProviderCheckRow key={c.id} check={c} />
      ))}
    </ul>
  );
}

const VERDICT_BADGE: Readonly<
  Record<ProviderVerdict, { text: string; variant: "success" | "danger" | "warning" }>
> = {
  ready: { text: "Ready", variant: "success" },
  refused: { text: "Not allowed", variant: "danger" },
  // "Unknown" in WARNING colour, never "Not allowed" and never neutral. The repo's standing rule:
  // an unresolved state is a failure to establish the claim, not a skipped optional step - the same
  // reason the verification bench renders an indeterminate pillar amber rather than grey.
  indeterminate: { text: "Unknown", variant: "warning" },
};

export function ProviderVerdictBadge({ verdict }: { verdict: ProviderVerdict }): ReactNode {
  const v = VERDICT_BADGE[verdict];
  return (
    <Badge variant={v.variant} data-testid={`verdict-${verdict}`}>
      {v.text}
    </Badge>
  );
}

// -------------------------------------------------------------------------------------------------
// The clone lifecycle
// -------------------------------------------------------------------------------------------------

/**
 * Each lifecycle state gets its own sentence AND its own icon.
 *
 * `deployed` is the one people model wrong, so it says out loud whose move is next: `attachService`
 * is `onlyOwner` on the core, so a contract the provider owns is not part of their listing until
 * DogTag attaches it. Presenting that as an error, or as something the provider can retry, sends
 * them looking for a button that does not and should not exist.
 */
const LIFECYCLE_COPY: Readonly<Record<CloneLifecycle, { title: string; body: string }>> = {
  unknown: {
    title: "Not established",
    body: "Nothing about this address has been established yet.",
  },
  notAClone: {
    title: "Not a DogTag contract",
    body: "This address was not deployed by the DogTag issuer factory, so it cannot be entered here.",
  },
  deployed: {
    title: "Deployed, waiting for DogTag",
    body: "You own this contract. It becomes part of your listing once DogTag attaches it to your provider record - that step is DogTag's, not yours.",
  },
  attached: {
    title: "Attached",
    body: "Attached to your provider record, and available to select.",
  },
  current: {
    title: "Selected",
    body: "New credentials of this record type are anchored here.",
  },
  foreign: {
    title: "Another provider's contract",
    body: "This contract is genuine, but it belongs to a different provider.",
  },
};

const LIFECYCLE_ICON: Readonly<Record<CloneLifecycle, typeof CircleHelp>> = {
  unknown: CircleHelp,
  notAClone: CircleSlash,
  deployed: Loader2,
  attached: CircleHelp,
  current: CircleHelp,
  foreign: AlertTriangle,
};

export function CloneLifecycleCard({ assessment }: { assessment: CloneAssessment }): ReactNode {
  const copy = LIFECYCLE_COPY[assessment.lifecycle];
  const Icon = LIFECYCLE_ICON[assessment.lifecycle];
  return (
    <section className="rounded-lg border border-border p-4" data-testid="clone-lifecycle">
      <header className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <h3 className="text-sm font-semibold" data-testid={`lifecycle-${assessment.lifecycle}`}>
            {copy.title}
          </h3>
        </div>
        <ProviderVerdictBadge verdict={assessment.verdict} />
      </header>
      <p className="mt-2 text-sm text-muted-foreground">{copy.body}</p>
      <p className="mt-2 text-sm" data-testid="clone-next-step">
        {assessment.nextStep}
      </p>
      <ProviderCheckList checks={assessment.checks} />
    </section>
  );
}

// -------------------------------------------------------------------------------------------------
// The three plan panels
// -------------------------------------------------------------------------------------------------

export function DeployPlanCard({
  plan,
  attachmentNotice,
}: {
  plan: DeployPlan;
  attachmentNotice: string;
}): ReactNode {
  return (
    <section className="rounded-lg border border-border p-4" data-testid="deploy-plan">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Deploy your own contract</h3>
        <ProviderVerdictBadge verdict={plan.verdict} />
      </header>
      {plan.predictedAddress ? (
        <p className="mt-2 break-all font-mono text-xs" data-testid="predicted-address">
          {plan.predictedAddress}
        </p>
      ) : (
        // NOT a blank. A blank where a provider expects an address reads as "no address yet", which
        // is a different and wrong fact from "we could not compute it".
        <p className="mt-2 text-xs text-amber-700 dark:text-amber-400" data-testid="predicted-unavailable">
          The address this would deploy to could not be computed.
        </p>
      )}
      <p className="mt-2 text-sm">{plan.nextStep}</p>
      <p className="mt-2 text-xs text-muted-foreground">{attachmentNotice}</p>
      <ProviderCheckList checks={plan.checks} />
    </section>
  );
}

export function DomainClaimCard({
  assessment,
  unreadableNotice = "The current domain record could not be read, so what this contract publishes is not known.",
}: {
  assessment: DomainClaimAssessment;
  unreadableNotice?: string;
}): ReactNode {
  return (
    <section className="rounded-lg border border-border p-4" data-testid="domain-claim">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Your domain</h3>
        <ProviderVerdictBadge verdict={assessment.verdict} />
      </header>
      {/* The typed disposition's own sentence, or an explicit statement that it is unknown. There is
          deliberately no fallback to "no domain published": that is a FACT the chain states, and
          claiming it from a read that failed is the conflation the typed enum exists to end. */}
      <p className="mt-2 text-sm" data-testid="domain-description">
        {assessment.description ?? unreadableNotice}
      </p>
      <p className="mt-2 text-sm">{assessment.nextStep}</p>
      <ProviderCheckList checks={assessment.checks} />
    </section>
  );
}

export function DirectoryPublicationCard({
  plan,
  contactOnlyNotice,
  anchoredNotice,
}: {
  plan: DirectoryPublicationPlan;
  contactOnlyNotice: string;
  anchoredNotice: string;
}): ReactNode {
  const pinSteps = plan.steps.filter((s) => s.kind === "pin");
  return (
    <section className="rounded-lg border border-border p-4" data-testid="directory-publication">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Your listing</h3>
        <ProviderVerdictBadge verdict={plan.verdict} />
      </header>

      <p className="mt-2 text-sm" data-testid="publication-shape">
        {pinSteps.length === 0
          ? "Contact details only - no location will be published."
          : "Contact details and one location."}
      </p>

      {/* Rendered whenever no location is being published, including when the location was REFUSED.
          A provider who has just been told their coordinates are unusable is exactly the provider
          about to type something plausible to get past it. */}
      {plan.contactOnly ? (
        <p className="mt-2 text-xs text-muted-foreground" data-testid="contact-only-notice">
          {contactOnlyNotice}
        </p>
      ) : null}

      <p className="mt-2 text-xs text-muted-foreground">{anchoredNotice}</p>
      <p className="mt-2 text-sm">{plan.nextStep}</p>
      <ProviderCheckList checks={plan.checks} />
    </section>
  );
}
