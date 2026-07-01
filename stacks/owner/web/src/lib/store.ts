// The owner's credential wallet — held credentials persisted locally (localStorage), keyed by their
// on-chain Merkle root so re-importing the same credential is idempotent. This mirrors the native
// apps' `credentials.json` local store: no cloud, no server, the owner holds their own credentials.
import type { WrappedDoc } from "@dogtag/standard";

const STORAGE_KEY = "dogtag.owner.credentials.v1";

export interface StoredCredential {
  /** the credential's on-chain Merkle root (0x + 64 hex) — the stable identity of a held credential. */
  id: string;
  /** ISO timestamp the credential was added to the wallet. */
  addedAt: string;
  /** the full wrapped document (the witness) — re-verifiable + presentable. */
  wrappedDoc: WrappedDoc;
}

type Listener = () => void;
const listeners = new Set<Listener>();

function read(): StoredCredential[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as StoredCredential[]) : [];
  } catch {
    return [];
  }
}

// A stable snapshot reference so `useSyncExternalStore` doesn't loop: only re-materialize on write.
let snapshot: StoredCredential[] = read();

function write(next: StoredCredential[]): void {
  snapshot = next;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  for (const l of listeners) l();
}

export const credentialStore = {
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  getSnapshot(): StoredCredential[] {
    return snapshot;
  },
  get(id: string): StoredCredential | undefined {
    return snapshot.find((c) => c.id === id);
  },
  /** Add (or replace) a credential by its root. Returns `true` if it was newly added. */
  add(wrappedDoc: WrappedDoc, now: string): boolean {
    const id = wrappedDoc.signature.merkleRoot;
    const existing = snapshot.findIndex((c) => c.id === id);
    const entry: StoredCredential = { id, addedAt: now, wrappedDoc };
    if (existing >= 0) {
      const next = snapshot.slice();
      next[existing] = { ...entry, addedAt: snapshot[existing]!.addedAt };
      write(next);
      return false;
    }
    write([entry, ...snapshot]);
    return true;
  },
  remove(id: string): void {
    write(snapshot.filter((c) => c.id !== id));
  },
  clear(): void {
    write([]);
  },
};
