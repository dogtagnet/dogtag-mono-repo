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
import { PROVIDER_CONTACT_CHANNELS } from "../directory/channels";
import { cn } from "../lib/cn";
import { ProviderLogo, type ProfileResolution } from "../mirror";
import type { SurfaceFault } from "../wallet/walletError";
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
// Progressive disclosure
// -------------------------------------------------------------------------------------------------

/**
 * A short question, and the mechanism behind it one click away.
 *
 * This page has to explain itself to a first-time provider, and the two obvious ways of doing that
 * both fail: a bare field with no hint costs a question, and a paragraph beside every field is a
 * wall nobody reads - which fails the same way, just less visibly. So the inline hint stays one
 * plain sentence and the *why* lives here, closed by default.
 *
 * Native `<details>`, deliberately. It needs no state, so it cannot interact with the no-`act()`
 * rule the mounted suites in this package are written under, and it carries its own keyboard and
 * screen-reader behaviour rather than a re-implementation of it.
 */
export function WhyThisExists({
  question,
  children,
  testId,
}: {
  question: string;
  children: ReactNode;
  testId?: string;
}): ReactNode {
  return (
    <details className="mt-1 text-xs text-muted-foreground" data-testid={testId}>
      <summary className="cursor-pointer select-none underline decoration-dotted underline-offset-2">
        {question}
      </summary>
      <div className="mt-1.5 flex flex-col gap-1.5 border-l-2 border-border pl-3">{children}</div>
    </details>
  );
}

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

/**
 * Why the answers below a verdict no longer describe the present.
 *
 * `edited` - an input this plan was computed from has changed since.
 * `spent`  - a transaction has been sent against it, so the chain has moved under it.
 *
 * Kept apart rather than merged into one "stale", because they send a provider to different places:
 * one means re-read what you typed, the other means re-read the chain.
 */
export type PlanRetirement = "edited" | "spent";

const RETIRED_BADGE: Readonly<Record<PlanRetirement, string>> = {
  edited: "Superseded - inputs changed",
  spent: "Superseded - read before your transaction",
};

/**
 * The verdict, and - when the plan behind it has been retired - the fact that it is superseded.
 *
 * A retired plan is still SHOWN, because "what did I check before I sent this" is exactly the
 * question a provider asks the moment a transaction goes out, and dropping the card answers it with
 * nothing. What makes that safe is the label, so the label is attached HERE, to the verdict itself,
 * rather than only to a sentence above the card: a reader who scans to a green "Ready" and stops is
 * precisely the reader a separate banner misses. The superseded verdict is struck through and an
 * amber qualifier sits beside it - the treatment this repo already uses for a value it cannot stand
 * behind, rather than a third invention for the same idea.
 */
export function ProviderVerdictBadge({
  verdict,
  retired,
}: {
  verdict: ProviderVerdict;
  retired?: PlanRetirement | null;
}): ReactNode {
  const v = VERDICT_BADGE[verdict];
  const badge = (
    <Badge
      variant={v.variant}
      className={retired ? "line-through opacity-70" : undefined}
      data-testid={`verdict-${verdict}`}
    >
      {v.text}
    </Badge>
  );
  if (!retired) return badge;
  return (
    <span className="flex flex-wrap items-center justify-end gap-2">
      {badge}
      <Badge variant="warning" data-testid={`verdict-retired-${retired}`}>
        {RETIRED_BADGE[retired]}
      </Badge>
    </span>
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

export function CloneLifecycleCard({
  assessment,
  retired,
}: {
  assessment: CloneAssessment;
  retired?: PlanRetirement | null;
}): ReactNode {
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
        <ProviderVerdictBadge verdict={assessment.verdict} retired={retired} />
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
  retired,
}: {
  plan: DeployPlan;
  attachmentNotice: string;
  retired?: PlanRetirement | null;
}): ReactNode {
  return (
    <section className="rounded-lg border border-border p-4" data-testid="deploy-plan">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Deploy your own contract</h3>
        <ProviderVerdictBadge verdict={plan.verdict} retired={retired} />
      </header>
      {plan.predictedAddress ? (
        // Labelled, because an unlabelled address on a page that has not deployed anything yet
        // reads as something that already exists. This is a PREDICTION, and it is exact: the
        // factory computes the address from the same three inputs whether it is asked to predict
        // or to deploy, so saying so is what turns Check from "did my form validate" into "this is
        // the contract I am about to create".
        <>
          <p className="mt-2 text-xs text-muted-foreground">The address this will deploy to:</p>
          <p className="break-all font-mono text-xs" data-testid="predicted-address">
            {plan.predictedAddress}
          </p>
          <p className="mt-1 text-xs text-muted-foreground" data-testid="predicted-address-caption">
            Worked out before anything is sent, and exact - deploying produces this address, not a
            similar one. Nothing has been created yet.
          </p>
        </>
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
  retired,
  unreadableNotice = "The current domain record could not be read, so what this contract publishes is not known.",
}: {
  assessment: DomainClaimAssessment;
  unreadableNotice?: string;
  retired?: PlanRetirement | null;
}): ReactNode {
  return (
    <section className="rounded-lg border border-border p-4" data-testid="domain-claim">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Your domain</h3>
        <ProviderVerdictBadge verdict={assessment.verdict} retired={retired} />
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
  retired,
}: {
  plan: DirectoryPublicationPlan;
  contactOnlyNotice: string;
  anchoredNotice: string;
  retired?: PlanRetirement | null;
}): ReactNode {
  const pinStep = plan.steps.find((s) => s.kind === "pin");
  return (
    <section className="rounded-lg border border-border p-4" data-testid="directory-publication">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Your listing</h3>
        <ProviderVerdictBadge verdict={plan.verdict} retired={retired} />
      </header>

      {/* Derived from the INPUT first and the steps second, because an absent pin step has three
          different meanings and only one of them is "no location was given". The other two - the
          published location is already exactly this, and what is published could not be read - must
          not be reported as a contact-only publication. */}
      <p className="mt-2 text-sm" data-testid="publication-shape">
        {plan.contactOnly
          ? "Contact details only - no location will be published."
          : pinStep
            ? pinStep.op === "update"
              ? "Contact details, and a replacement for the location you have already published."
              : "Contact details and one location."
            : "The location you gave is not being sent - the checks below say why."}
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

/**
 * What a READER sees of this provider's published listing (registry-plan S-17).
 *
 * A provider needs to know whether what they published actually resolves, and this is the only
 * surface that can tell them: it runs the same resolution any consumer runs - fetch from the content
 * mirror, recompute the digest, compare it to what the chain anchors - and renders the result rather
 * than the intent. A publication that verifies here verifies everywhere; one that does not is broken
 * for everyone, and the provider is the only party who can fix it.
 *
 * The logo obeys the slice's rule, which {@link ProviderLogo} owns: verified renders, unverified
 * renders NOTHING with a visible reason, not published is ordinary and quiet.
 */
export function PublishedListingCard({
  resolution,
  providerName,
  unconfigured,
}: {
  resolution: ProfileResolution | undefined;
  providerName: string;
  /**
   * Set when no content mirror is configured, so the resolution CANNOT start.
   *
   * Its own rendered line, never the pending spinner. `undefined` means "not finished yet", and
   * letting "cannot begin" share that spelling is a spinner that never resolves and an operator who
   * cannot tell whether anything is happening - the exact failure `RECEIPT_TIMEOUT_MS` exists to
   * prevent one flow over. It also makes the two halves of this surface AGREE: the publish path
   * already refuses loudly with a named "no content mirror is configured", so the read path must not
   * fail silently in the same deployment.
   *
   * **It is keyed on the mirror BASE alone, and deliberately not on the mirror token.** The write
   * path needs both and refuses up front without either; reading does not, because
   * `GET /v1/content/:address` is unauthenticated by design - a content address is checked against
   * the bytes it names, so serving them confers nothing and gating the read would buy no integrity.
   * Adding a token term here for symmetry would report a perfectly working read surface as unable
   * to start.
   */
  unconfigured?: boolean;
}): ReactNode {
  return (
    <section className="rounded-lg border border-border p-4" data-testid="published-listing">
      <header className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">What a reader sees</h3>
      </header>

      {unconfigured ? (
        <p className="mt-2 text-sm text-muted-foreground" data-testid="listing-unconfigured">
          No content mirror is configured, so what you have published cannot be read back here. Set{" "}
          <code>VITE_CONTENT_MIRROR_BASE</code> to check it.
        </p>
      ) : resolution === undefined ? (
        <p
          className="mt-2 flex items-center gap-2 text-sm text-muted-foreground"
          data-testid="listing-pending"
        >
          <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> Reading what you have
          published…
        </p>
      ) : resolution.state === "notPublished" ? (
        <p className="mt-2 text-sm text-muted-foreground" data-testid="listing-not-published">
          {resolution.withdrawn
            ? "You have taken your published details down. Your listing is still here; the details are not."
            : "You have not published any details yet."}
        </p>
      ) : resolution.state === "unverified" ? (
        // Loud, because this is the state in which the chain says something IS published and a
        // reader cannot confirm it - which looks, from the outside, exactly like publishing nothing.
        <p
          className="mt-2 flex items-start gap-1.5 text-sm text-amber-700 dark:text-amber-400"
          data-testid="listing-unverified"
        >
          <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" aria-hidden />
          <span>
            <span className="font-medium">Your published details could not be confirmed.</span>{" "}
            {resolution.reason}
          </span>
        </p>
      ) : (
        <div className="mt-3 flex items-start gap-3" data-testid="listing-resolved">
          <ProviderLogo logo={resolution.logo} providerName={providerName} />
          <dl className="min-w-0 flex-1 text-sm">
            {PROVIDER_CONTACT_CHANNELS.map((channel) => {
              const value = resolution.profile.contact[channel];
              if (!value) return null;
              return (
                <div key={channel} className="flex gap-2">
                  <dt className="w-20 shrink-0 text-xs uppercase text-muted-foreground">
                    {channel}
                  </dt>
                  <dd className="min-w-0 break-words">{value}</dd>
                </div>
              );
            })}
          </dl>
        </div>
      )}
    </section>
  );
}

/**
 * A fault in the wallet or in this page, rendered so it can never be read as a verdict.
 *
 * THE DEFECT: a bare wallet string was rendered unlabelled in red inside the card headed "Your
 * provider record", directly beneath the provider id, the record type and the caller address. When
 * a wallet answered `4100 Unauthorized`, that put the words "not been authorized" exactly where an
 * answer about the provider's authorization belongs - and a captain read it that way while the
 * chain said his provider was ACTIVE and approved. The most misleading sentence a wallet could
 * return, in the worst possible position.
 *
 * Three things fix it, and all three are load-bearing:
 *
 *   * **It is labelled**, so the layer at fault is named before the message is read.
 *   * **It states what was NOT established.** Silence about the provider, next to a wallet's
 *     refusal, is read as an answer about the provider; saying so explicitly is the only thing that
 *     stops it. This is the codebase's could-not-run rule - a question nobody could answer is not a
 *     failed check - applied to a thrown error.
 *   * **It sits outside the provider-record card**, so position cannot imply what the words deny.
 *
 * Amber rather than red, deliberately. Red is this page's colour for a failed CHECK, and a fault
 * that establishes nothing must not borrow the styling of an answer.
 *
 * SHARED, so the two surfaces that catch a wallet fault cannot drift on what they deny: the S-15
 * flows and the issuance-list page both render this one.
 */
export function WalletFaultNotice({ fault }: { fault: SurfaceFault | null }): ReactNode {
  if (!fault) return null;
  return (
    <div
      className="rounded-lg border border-amber-500/50 bg-amber-500/10 p-4 text-sm text-amber-800 dark:text-amber-300"
      data-testid="wallet-fault"
      data-fault={fault.kind}
    >
      <p className="font-semibold">
        {fault.kind === "walletRejected"
          ? "You cancelled this in your wallet"
          : fault.kind === "surfaceFault"
            ? "This page hit a problem"
            : "Your wallet could not complete this"}
      </p>
      {/* The denial comes BEFORE the wallet's own words, so the qualification is read first rather
          than as an afterthought to a sentence that has already landed. */}
      <p className="mt-1" data-testid="wallet-fault-established">
        {fault.established}
      </p>
      <p className="mt-1" data-testid="wallet-fault-next">
        {fault.nextStep}
      </p>
      <p className="mt-2 break-words text-xs opacity-80">
        Your wallet said: <span className="font-mono">{fault.detail}</span>
      </p>
    </div>
  );
}
