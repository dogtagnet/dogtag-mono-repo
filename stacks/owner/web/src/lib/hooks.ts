import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { loadConsentReceipts, type ConsentReceipt } from "./consents";
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

export interface ConsentHistory {
  state: "loading" | "ready" | "error";
  receipts: ConsentReceipt[];
  reload: () => void;
}

/** The owner's consent history for the held credentials' tags, read live from ROAX. Re-reads when
 *  the held-credential set changes; `reload` re-reads on demand (e.g. after an RPC failure). */
export function useConsentReceipts(credentials: StoredCredential[]): ConsentHistory {
  const [state, setState] = useState<ConsentHistory["state"]>("loading");
  const [receipts, setReceipts] = useState<ConsentReceipt[]>([]);
  const [generation, setGeneration] = useState(0);
  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  useEffect(() => {
    let live = true;
    setState("loading");
    loadConsentReceipts(credentials)
      .then((r) => {
        if (!live) return;
        setReceipts(r);
        setState("ready");
      })
      .catch(() => live && setState("error"));
    return () => {
      live = false;
    };
  }, [credentials, generation]);

  return { state, receipts, reload };
}
