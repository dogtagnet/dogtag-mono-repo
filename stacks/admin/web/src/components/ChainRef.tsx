import { CopyButton, addressChainRef, cn } from "@dogtag/ui";
import { shortAddr } from "../lib/format";

/**
 * The admin portal's ONE on-chain address reference.
 *
 * Every page here used to open-code `href={explorerAddressUrl(addr)}` and render the anchor
 * unconditionally, which tells the operator *this exists on chain, click to confirm it* about a value
 * that may address nothing at all. A dead link is worse than no link: its presence reads as evidence,
 * and an operator will not click every one to find out. This is the same rule PR #88 established for
 * the government and vet audit tables and that `Activity.tsx` already followed - the reason it lives in
 * one component now is that six pages each having their own idea of how chain state looks is its own
 * defect.
 *
 * Three states, and none of them is a link to nowhere:
 *
 *  - **on chain** - a well-formed 20-byte address, so it has an explorer page: linked.
 *  - **not recorded** - no address at all. Nothing was claimed, so nothing is explained; the caller's
 *    own fallback (`—`, "hosted operator key") says what it means.
 *  - **cannot be looked up** - an address IS present but is not well-formed, so no explorer page can
 *    exist for it. The value is still shown (it is what the backend reported, and hiding it would cost
 *    the operator a fact), inert, carrying the reason and an unconditional copy affordance. Never
 *    silently collapsed into either neighbour: "we have nothing" and "we have something unusable" are
 *    different situations with different remedies.
 *
 * The positive side claims only that the address is well-formed enough to LOOK UP - exactly what
 * `addressChainRef` checks. No chain read happens here, so nothing on this path may be read as
 * confirmation that the address holds anything.
 */
export function AddressRef({
  address,
  full = false,
  className,
  testId = "chain-address",
}: {
  address?: string | null;
  /** Show the address complete rather than middle-truncated (a detail row rather than a dense cell). */
  full?: boolean;
  className?: string;
  testId?: string;
}) {
  if (!address) {
    return (
      <span className={cn("text-xs text-muted", className)} data-testid={`${testId}-none`}>
        —
      </span>
    );
  }
  const { href, reason } = addressChainRef(address);
  const shown = full ? address : shortAddr(address);
  if (href) {
    return (
      <a
        href={href}
        target="_blank"
        rel="noreferrer"
        className={cn("font-mono text-xs text-primary hover:underline", className)}
        title={address}
        data-testid={testId}
      >
        {shown}
      </a>
    );
  }
  // Struck through, exactly as the shared `TxRef` renders an unusable transaction hash. The two are
  // the same claim - "this cannot be looked up" - so they get the same visual language, and it has to
  // be a VISIBLE one: colour alone leaves the state legible only to someone who already knows the
  // linked form is blue, and this repo's standing rule is that a finding reachable only by hovering is
  // not reported.
  //
  // The COPY affordance is on this branch for the same hovering rule, applied to the value rather than
  // to its state. `shortAddr` middle-truncates, so the elided characters are not in the DOM at all,
  // and the linked branch's escape hatch is gone here: there is no `href` to copy the full address out
  // of. That would leave the tooltip as the only route to the one fact this row exists to give the
  // operator - and a tooltip survives neither a screenshot, nor a touch device, nor a paste into an
  // incident report, which is exactly what an unusable address has to be pasted into. `provenance.ts`
  // states that rule for every middle-truncated value; `TxRef` and `ChainValue` already keep it.
  //
  // It is UNCONDITIONAL, and there is deliberately no prop to decline it. An opt-out existed for one
  // round, so that a caller already rendering its own control did not show two - and it turned the
  // guarantee into a per-caller convention: both callers that took it supplied a
  // `navigator.clipboard`-only button, which is silently dead on the plain `http://` LAN origins these
  // portals are routinely served from, so the state whose whole point is recoverability had no working
  // route to the value at all. An escape hatch a caller can decline is not an escape hatch. Where a
  // caller still renders its own copy control, the inert state now shows two; that duplication is
  // deliberate and is the cheaper of the two failures.
  return (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1">
      <span
        className={cn(
          "font-mono text-xs text-muted line-through decoration-warning/60",
          className,
        )}
        title={`${address} — ${reason}`}
        data-testid={`${testId}-inert`}
        data-reason={reason ?? undefined}
      >
        {shown}
      </span>
      <CopyButton value={address} label="address" />
    </span>
  );
}
