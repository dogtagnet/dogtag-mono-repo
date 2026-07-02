import { useEffect, useState, useSyncExternalStore } from "react";
import { credentialStore, type StoredCredential } from "./store";
import { loadOrCreateWallet, type OwnerWallet } from "./wallet";

/** Reactive view of the held credentials (localStorage-backed). */
export function useCredentials(): StoredCredential[] {
  return useSyncExternalStore(credentialStore.subscribe, credentialStore.getSnapshot);
}

/** Load (creating on first run) the owner's self-custodial wallet. */
export function useWallet(): OwnerWallet | null {
  const [wallet, setWallet] = useState<OwnerWallet | null>(null);
  useEffect(() => {
    let live = true;
    loadOrCreateWallet()
      .then((w) => {
        if (live) setWallet(w);
      })
      .catch(() => {
        /* surfaced as a null wallet -> "preparing" state in the UI */
      });
    return () => {
      live = false;
    };
  }, []);
  return wallet;
}

/** Short 0x… address label. */
export function shortAddr(a: string): string {
  return a.length > 12 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a;
}
