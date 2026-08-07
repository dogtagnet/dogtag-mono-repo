/**
 * The remembered provider id (registry-plan S-15).
 *
 * WHY A PAGE STATE SURVIVES A REFRESH HERE AND NOWHERE ELSE ON THIS PAGE. The provider id is a
 * stable identity assigned by DogTag when the provider is approved - it is typed once and then
 * true for the life of the record. Holding it in component state alone meant a refresh emptied it,
 * three flows went dead at once, and the reasons under their buttons pointed at an empty field in a
 * card the reader had scrolled past. A captain hit exactly that and named the general truth:
 * refreshes are inevitable. So the id is remembered the way this codebase already remembers
 * per-browser operator choices - `localStorage` under a versioned key, the `dogtag.roax-rpc-url.v1`
 * precedent - and restored on the next visit.
 *
 * SCOPED BY WALLET, because the wallet is the provider's identity anchor on this page (every plan
 * key already starts `${caller}|${providerId}`). The key carries the caller address, so two
 * providers sharing one browser profile each get their own remembered id back: connecting wallet B
 * never restores the id an operator typed while wallet A was connected. Without a connected wallet
 * there is no scope to read or write, and nothing is remembered - which costs nothing, because
 * every action on the page needs the wallet connected anyway.
 *
 * WHAT IS DELIBERATELY NOT REMEMBERED: flow 2's contract address. That field is an ACTION
 * parameter - the subject of a pending "make this current" decision, not a durable identity - and
 * it is recoverable from the page itself: the deployed-contracts card lists this wallet's clones
 * straight from the factory's own creation log. Restoring a weeks-old candidate into a field that
 * names the target of a transaction would pre-fill a choice nobody made this session, which is the
 * exact prefill hazard the page's checked-values discipline exists to refuse.
 *
 * Remembering is a convenience, so every storage failure is swallowed: a browser that blocks
 * `localStorage` gets today's behaviour, never a broken page.
 */

import type { StorageLike } from "../chain/rpcEndpoint";

export const PROVIDER_ID_STORAGE_PREFIX = "dogtag.provider-id.v1.";

/** One key per wallet, per origin. Lowercased so a checksummed and a lowercase caller share it. */
export function providerIdStorageKey(caller: string): string {
  return `${PROVIDER_ID_STORAGE_PREFIX}${caller.toLowerCase()}`;
}

function browserStorage(): StorageLike | undefined {
  if (typeof window === "undefined") return undefined;
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/**
 * The provider id this wallet last typed, or `null` when none is remembered.
 *
 * What was typed is restored verbatim - an id mid-correction survives a refresh exactly as the
 * operator left it, and the field's own validation line says so if it is not yet a whole id.
 */
export function recallProviderId(
  caller: string | undefined,
  storage: StorageLike | undefined = browserStorage(),
): string | null {
  if (!caller) return null;
  try {
    const stored = storage?.getItem(providerIdStorageKey(caller)) ?? null;
    return stored === null || stored.trim() === "" ? null : stored;
  } catch {
    return null;
  }
}

/**
 * Remember what this wallet typed. Clearing the field forgets it - an emptied field is a statement,
 * and restoring a value the operator deliberately removed would overrule them.
 */
export function rememberProviderId(
  caller: string | undefined,
  providerId: string,
  storage: StorageLike | undefined = browserStorage(),
): void {
  if (!caller) return;
  try {
    if (providerId.trim() === "") {
      storage?.removeItem(providerIdStorageKey(caller));
    } else {
      storage?.setItem(providerIdStorageKey(caller), providerId);
    }
  } catch {
    // Remembering is a convenience; failing to must never break the page.
  }
}
