import { isOpaqueIdentifier } from "@dogtag/ui";

/**
 * Middle-truncate an OPAQUE identifier (an address, a hash, a decimal field element) for a dense cell.
 *
 * The `isOpaqueIdentifier` guard is the same rule `shortValue` enforces in `@dogtag/ui`, and it is here
 * because a middle-truncator is correct only for a value whose middle is noise. Applied to human text
 * it reads as corruption rather than elision - in PR #88 the shared truncator rendered
 * `TRAVEL_CLEARANCE` as `TRAVEL_C…ARANCE` on the very cell an operator uses to tell what a row is
 * about. Admin never fed this one a label, so nothing changes for the six pages that call it; the guard
 * exists so that stays true, and so a future caller gets clipped-but-whole text instead of a mangled
 * word. Addresses keep their 6/4 form.
 */
export function shortAddr(addr: string): string {
  if (!addr || addr.length < 10) return addr;
  if (!isOpaqueIdentifier(addr)) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}
