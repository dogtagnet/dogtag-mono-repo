/**
 * Why a control on the provider page cannot be used right now (registry-plan S-15).
 *
 * THE DEFECT THIS CLOSES, found by a captain on a live stack and worth stating exactly, because it
 * is the sharpest instance of the whole page's failure mode. **Deploy** is disabled until a plan
 * exists (`!deploy?.canDeploy`), and before the first check there is no plan - so the FIRST-RUN
 * state, the one every new provider is in, was a dead button with nothing on screen saying why,
 * directly under a heading reading "You deploy it and you own it". The explanation path could not
 * cover it either: `retiredBecause` answers `null` when no plan is held, so the notice that
 * explains a stale plan rendered nothing at all for a plan that had never been run. Nowhere in the
 * page did the words "run the check first" appear.
 *
 * Two rules follow, and they are the same rule the check rows already obey one level down.
 *
 *   1. **A DISABLED CONTROL ALWAYS SAYS WHY.** Not sometimes, and not only for the states somebody
 *      remembered. A control that refuses in silence is indistinguishable from one that is broken,
 *      and the user's next move is to hunt for a setting that does not exist.
 *   2. **EACH REASON IS ITS OWN REASON.** "No wallet", "wrong chain", "something is in flight",
 *      "you have not run the check", "you changed the form after checking" and "the check said no"
 *      are six different situations with six different remedies, and they rendered identically -
 *      as nothing. Collapsing them into one "unavailable" would fix the silence and keep the part
 *      that actually costs time.
 *
 * Pure, so every branch is reachable from a test rather than from a browser somebody remembered to
 * open in exactly the right state - which for `neverChecked` means the state that is hardest to
 * stumble into deliberately and easiest to ship broken.
 */

import type { ProviderVerdict } from "./types";

/**
 * A plan-gated control's view of the plan behind it.
 *
 * `never` is deliberately its own member rather than being folded in with `edited`. They are both
 * "there is no usable plan", and they send a user to opposite places: one has never pressed Check,
 * the other has pressed it and then typed something. Telling the first "your answers are out of
 * date" describes answers they have never seen.
 */
export type PlanGateState =
  | "never"
  | "edited"
  | "spent"
  | "refused"
  | "indeterminate"
  | "ready";

export function planGateState(input: {
  /** A plan is held at all. */
  present: boolean;
  /** A transaction has been submitted against it. */
  spent: boolean;
  /** The inputs it was computed from still match the form. */
  keyMatches: boolean;
  /** The plan's own permission flag (`canDeploy`, `canRepoint`, …). */
  canAct: boolean;
  verdict?: ProviderVerdict;
}): PlanGateState {
  if (!input.present) return "never";
  // Spent before edited, matching `retiredBecause`: a submitted transaction falsifies a plan whose
  // form inputs are untouched, so it must not be reported as an edit the user did not make.
  if (input.spent) return "spent";
  if (!input.keyMatches) return "edited";
  if (input.canAct) return "ready";
  return input.verdict === "indeterminate" ? "indeterminate" : "refused";
}

export type ActionBlock =
  | { kind: "busy" }
  | { kind: "notConnected" }
  | { kind: "readerUnavailable" }
  | { kind: "wrongChain"; expected: number; actual: number }
  /** `needs` is the complete sentence to show, not a field name. */
  | { kind: "missingInput"; needs: string }
  | { kind: "neverChecked"; check: string }
  | { kind: "planEdited"; check: string }
  | { kind: "planSpent"; check: string }
  | { kind: "planRefused" }
  | { kind: "planIndeterminate" }
  /** A flow-specific refusal that already has its own sentence (a bad domain, no content mirror). */
  | { kind: "otherwiseBlocked"; why: string };

interface Preconditions {
  /** An action is already in flight. */
  busy: boolean;
  connected: boolean;
  /** The chain reader has been constructed. */
  hasReader: boolean;
}

/**
 * The terms every control shares, in the order a user would fix them.
 *
 * `busy` outranks the rest because it is transient: telling somebody to connect a wallet they have
 * already connected, because a request happens to be in flight, is worse than saying nothing.
 */
function preconditionBlock(p: Preconditions): ActionBlock | null {
  if (p.busy) return { kind: "busy" };
  if (!p.connected) return { kind: "notConnected" };
  if (!p.hasReader) return { kind: "readerUnavailable" };
  return null;
}

/** Why a read-only **Check** button is unavailable. */
export function checkBlock(
  input: Preconditions & { missingInput?: string | null },
): ActionBlock | null {
  const pre = preconditionBlock(input);
  if (pre) return pre;
  if (input.missingInput) return { kind: "missingInput", needs: input.missingInput };
  return null;
}

/**
 * One control's reason, and whether it has already been said in full further up the page.
 *
 * `brief` exists because the two obvious answers to a repeated sentence are both wrong. Printing it
 * again is what a captain saw - the identical four-line sentence twice in a row, and once per flow
 * down the page. But SUPPRESSING it breaks the older and more important rule this module exists for:
 * a disabled control always says why, and a control whose reason was dropped because some other
 * control said it first is silent again, which is the defect the whole file was written to remove.
 * `never leaves a disabled send button silent, in any of the states this page has` fails on exactly
 * that, and it is right to.
 */
export interface RenderedReason {
  block: ActionBlock;
  /** `full` for the first control that hits this obstacle; `brief` for every later one. */
  style: "full" | "brief";
}

/**
 * Say each obstacle in full once, and in one line under every later control it also blocks.
 *
 * Ordered, and the order is the source order of the controls: the first control a reader reaches is
 * the one that should carry the explanation, and a full sentence appearing only under the last
 * button is attached to the wrong control.
 *
 * Compared on the RENDERED TEXT rather than on the block kind, because two blocks of the same kind
 * can carry genuinely different sentences - `missingInput` and `otherwiseBlocked` are whole
 * sentences supplied by the call site, and each flow's is about a different field.
 *
 * The list is passed whole rather than accumulated during render, so the ordering is a property of
 * one array rather than an assumption about when JSX children are evaluated.
 */
export function sequenceReasons(
  blocks: readonly (ActionBlock | null)[],
): (RenderedReason | null)[] {
  const seen = new Set<string>();
  return blocks.map((block) => {
    if (!block) return null;
    const text = describeActionBlock(block);
    if (seen.has(text)) return { block, style: "brief" };
    seen.add(text);
    return { block, style: "full" };
  });
}

/** The sentence to print for one control, at whichever length it earned. */
export function renderReason(reason: RenderedReason): string {
  return reason.style === "full"
    ? describeActionBlock(reason.block)
    : // Still a complete sentence naming the obstacle, so this control is explained on its own terms
      // rather than by a pointer to something further up.
      `Unavailable while ${briefActionBlock(reason.block)}.`;
}

/**
 * The obstacle named in a few words, for a sentence that has to MENTION it rather than BE it.
 *
 * The superseded banner needs to say what is standing in the way of re-running the check, and it
 * sits directly under a control whose own reason has already said it in full. Restating four lines
 * there would be the duplication above in a new costume; "see the reason above" makes the reader do
 * the joining. So each obstacle gets a short clause, and the banner reads as one sentence.
 */
export function briefActionBlock(block: ActionBlock): string {
  switch (block.kind) {
    case "busy":
      return "something is already in flight";
    case "notConnected":
      return "your wallet is not connected";
    case "readerUnavailable":
      return "this page cannot reach the chain";
    case "wrongChain":
      return `your wallet is on chain ${block.actual} and this deployment is on chain ${block.expected}`;
    case "missingInput":
      return "a field it needs is still empty";
    case "neverChecked":
    case "planEdited":
    case "planSpent":
    case "planRefused":
    case "planIndeterminate":
      // Unreachable for a CHECK button, which is the only thing this describes: those five states
      // are about a plan, and a plan is what a check produces. Answered rather than thrown so a
      // future caller cannot crash the page on a state this simply has no short form for.
      return "the check is unavailable";
    case "otherwiseBlocked":
      return block.why;
  }
}

/**
 * The wallet's chain, when it can be established.
 *
 * `actual: undefined` means the connector did not report one, and that is NOT the wrong chain - it
 * is not knowing. Reporting it as a mismatch would accuse a wallet that may be perfectly correct,
 * which is this repo's standing rule about a read that did not resolve, applied to a control.
 */
export interface ChainCheck {
  expected: number;
  actual: number | undefined;
}

/**
 * Why a **send** button is unavailable.
 *
 * The chain term is here and deliberately NOT on {@link checkBlock}: the checks read through the
 * page's own configured endpoint, so they are correct whatever the wallet is pointed at, while a
 * transaction can only go to the wallet's chain. Gating the checks on it would refuse a preflight
 * that would have worked and answered usefully.
 */
export function sendBlock(
  input: Preconditions & {
    chain: ChainCheck;
    /** The label on the Check button that authorizes this send. */
    check: string;
    plan: PlanGateState;
    /** A flow-specific refusal, applied last so a plan problem is reported before a form one. */
    otherwiseBlocked?: string | null;
  },
): ActionBlock | null {
  const pre = preconditionBlock(input);
  if (pre) return pre;
  const { expected, actual } = input.chain;
  if (actual !== undefined && actual !== expected) {
    return { kind: "wrongChain", expected, actual };
  }
  switch (input.plan) {
    case "never":
      return { kind: "neverChecked", check: input.check };
    case "edited":
      return { kind: "planEdited", check: input.check };
    case "spent":
      return { kind: "planSpent", check: input.check };
    case "refused":
      return { kind: "planRefused" };
    case "indeterminate":
      return { kind: "planIndeterminate" };
    case "ready":
      break;
  }
  if (input.otherwiseBlocked) return { kind: "otherwiseBlocked", why: input.otherwiseBlocked };
  return null;
}

/** Why a held plan's answers no longer describe what would be sent. */
export type PlanRetirementReason = "edited" | "spent";

/**
 * What to tell someone whose plan has been superseded - given whether they can actually re-run it.
 *
 * **A SCREEN MUST NOT INSTRUCT AN ACTION IT HAS DISABLED.** The superseded banner said "Check again
 * before sending" unconditionally, so a captain whose wallet had disconnected read it directly above
 * a Check button the page itself had greyed out. That is a dead end of the worst kind: it names one
 * remedy, that remedy is unavailable, and nothing on screen connects the two - so the reader
 * concludes the page is broken, which from where they are sitting it is.
 *
 * The obstacle's own sentence is what replaces the instruction, because that sentence already names
 * the thing that has to happen first. Nothing here duplicates it or paraphrases it.
 */
export function describePlanRetirement(
  reason: PlanRetirementReason,
  checkBlocked: ActionBlock | null,
): string {
  const situation =
    reason === "edited"
      ? "You have changed something since this was checked, so what is shown below describes what you typed before."
      : "A transaction has already been sent against this, so the chain may have moved since these answers were read. They are kept below so you can see what you checked.";
  return checkBlocked
    ? `${situation} Checking again is what clears this, and that is not available while ${briefActionBlock(
        checkBlocked,
      )}.`
    : `${situation} Check again before sending${reason === "spent" ? " another" : ""}.`;
}

/**
 * The sentence shown under the control.
 *
 * Every one names the next action rather than only the obstacle. "No plan has been run" is a
 * description of our state; "Run *Check what this would deploy* first" is something the reader can
 * do, and it is the difference between an explanation and a label.
 */
export function describeActionBlock(block: ActionBlock): string {
  switch (block.kind) {
    case "busy":
      return "Something is already in flight. Wait for it to finish.";
    case "notConnected":
      return "Connect your wallet first, using the button in the top right. Every action on this page is signed by you - including the checks, because there is no backend here to run them for you.";
    case "readerUnavailable":
      return "This page cannot reach the chain yet, so nothing can be checked.";
    case "wrongChain":
      return `Your wallet is on chain ${block.actual} and this deployment is on chain ${block.expected}. Switch it in your wallet. The checks above are unaffected - they read through this page's own connection - but a transaction can only go to the chain your wallet is on.`;
    case "missingInput":
      // Passed whole by the call site rather than slotted into a template. Flow 3's reason has to
      // explain that the field it wants lives in ANOTHER step, which no "<field> is required"
      // shape can carry - and that is the one this page most needed.
      return block.needs;
    case "neverChecked":
      // The first-run sentence, and the one the page never had. It says what to press AND why the
      // button is gated at all - without the second half, a user who checks once and then edits the
      // form has no way to predict that it goes dead again.
      return `Run "${block.check}" first. This button only ever sends something a check has approved, and that check has to match the values in the form right now - so it goes back to unavailable whenever you change one.`;
    case "planEdited":
      return `You have changed something since "${block.check}" was run, so those answers no longer describe what would be sent. Run it again.`;
    case "planSpent":
      return `A transaction has already been sent against this check, so the chain may have moved under it. Run "${block.check}" again before sending another.`;
    case "planRefused":
      return "The last check refused this. The failed rows above say why.";
    case "planIndeterminate":
      return "The last check could not resolve, so whether this would succeed is not known. Run it again once the connection is back.";
    case "otherwiseBlocked":
      return block.why;
  }
}
