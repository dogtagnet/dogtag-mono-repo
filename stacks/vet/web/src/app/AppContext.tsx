import {
  createApiClient,
  custodyStateFromSigners,
  useToast,
  type ApiClient,
  type CustodyState,
  type SigningMode,
} from "@dogtag/ui";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { env } from "../lib/env";

const OP_KEY = "vet.opToken";
const ADMIN_KEY = "vet.adminToken";
const SIGNER_KEY = "vet.signerAddress";

interface AppContextValue {
  api: ApiClient;
  opToken: string | null;
  adminToken: string | null;
  setOpToken: (t: string | null) => void;
  setAdminToken: (t: string | null) => void;
  /** genesis-derived active signer address — auto-carried into the whitelist apply form */
  signerAddress: string | null;
  setSignerAddress: (a: string | null) => void;
  signingMode: SigningMode;
  setSigningMode: (m: SigningMode) => void;
  /**
   * What we currently believe about the backend's custody seal. `unknown` until the probe (or a
   * failing call) tells us otherwise; nothing may announce a lock on `unknown`, since a backend that
   * is merely down is not a locked one.
   */
  custodyState: CustodyState;
  setCustodyState: (s: CustodyState) => void;
  /** True while the point-of-need unlock prompt is raised. */
  unlockPromptOpen: boolean;
  /** Resolve the pending unlock prompt: true once the seal is open, false if it was dismissed. */
  resolveUnlockPrompt: (unlocked: boolean) => void;
  /** Raise the unlock prompt by hand (the locked banner) rather than via a refused request. */
  openUnlockPrompt: () => void;
  logout: () => void;
}

const AppContext = createContext<AppContextValue | null>(null);

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function AppProvider({ children }: { children: ReactNode }) {
  const { toast } = useToast();
  const [opToken, setOpTokenState] = useState<string | null>(() => read(OP_KEY));
  const [adminToken, setAdminTokenState] = useState<string | null>(() => read(ADMIN_KEY));
  const [signerAddress, setSignerAddressState] = useState<string | null>(() => read(SIGNER_KEY));
  const [signingMode, setSigningMode] = useState<SigningMode>("backend");
  const [custodyState, setCustodyState] = useState<CustodyState>("unknown");
  const [unlockPromptOpen, setUnlockPromptOpen] = useState(false);
  // A single in-flight prompt shared by every request that trips the lock at once, so two concurrent
  // 409s raise ONE dialog and both replay on the same answer.
  const pendingUnlock = useRef<{ promise: Promise<boolean>; resolve: (v: boolean) => void } | null>(null);

  const requestUnlock = useCallback((): Promise<boolean> => {
    setCustodyState("locked");
    if (pendingUnlock.current) return pendingUnlock.current.promise;
    let resolve!: (v: boolean) => void;
    const promise = new Promise<boolean>((r) => {
      resolve = r;
    });
    pendingUnlock.current = { promise, resolve };
    setUnlockPromptOpen(true);
    return promise;
  }, []);

  const resolveUnlockPrompt = useCallback((didUnlock: boolean) => {
    setUnlockPromptOpen(false);
    if (didUnlock) setCustodyState("unlocked");
    const pending = pendingUnlock.current;
    pendingUnlock.current = null;
    pending?.resolve(didUnlock);
  }, []);

  const openUnlockPrompt = useCallback(() => {
    void requestUnlock();
  }, [requestUnlock]);

  const persist = (key: string, t: string | null) => {
    try {
      if (t) window.localStorage.setItem(key, t);
      else window.localStorage.removeItem(key);
    } catch {
      /* ignore */
    }
  };

  const setOpToken = useCallback((t: string | null) => {
    setOpTokenState(t);
    persist(OP_KEY, t);
  }, []);

  const setAdminToken = useCallback((t: string | null) => {
    setAdminTokenState(t);
    persist(ADMIN_KEY, t);
  }, []);

  const setSignerAddress = useCallback((a: string | null) => {
    setSignerAddressState(a);
    persist(SIGNER_KEY, a);
  }, []);

  // The api client is created once; route its 401 callback through a ref so it always sees the
  // latest token setters + toast without recreating the client.
  const onUnauthorizedRef = useRef<(kind: "operator" | "admin") => void>(() => {});
  onUnauthorizedRef.current = (kind) => {
    if (kind === "operator") setOpToken(null);
    setAdminToken(null);
    // The session died, but that says nothing about the seal — go back to "unknown" so the probe
    // re-runs after the next login instead of asserting a lock we did not observe.
    setCustodyState("unknown");
    toast({
      title: "Session expired",
      description: "Your session is no longer valid — please log in again.",
      variant: "danger",
    });
  };

  // Routed through a ref so the once-created client always sees the latest resolver.
  const onCustodyLockedRef = useRef<() => Promise<boolean>>(() => Promise.resolve(false));
  onCustodyLockedRef.current = () => requestUnlock();

  const api = useMemo(
    () =>
      createApiClient({
        baseUrl: env.vetApiBase,
        centralBaseUrl: env.centralApiBase,
        getOperatorToken: () => read(OP_KEY),
        getAdminToken: () => read(ADMIN_KEY),
        onUnauthorized: (kind) => onUnauthorizedRef.current(kind),
        // Any "not unlocked" refusal raises the in-place prompt and, once the seal opens, the client
        // replays the refused request. The page never unmounts, so a half-filled form is preserved
        // and the operator's action simply continues.
        onCustodyLocked: () => onCustodyLockedRef.current(),
      }),
    [],
  );

  const logout = useCallback(() => {
    setOpToken(null);
    setAdminToken(null);
    setCustodyState("unknown");
  }, [setOpToken, setAdminToken]);

  // Custody probe: once an operator session exists and we have no opinion yet, ask the existing
  // read-only GET /issuer/signers whether the seal is unlocked. A restart re-locks custody silently,
  // so this is what turns "the portal loads" into "the portal lands you on /unlock". Any failure
  // leaves the state `unknown` — a backend that is down must not look like a locked one.
  useEffect(() => {
    if (!opToken || custodyState !== "unknown") return;
    let cancelled = false;
    api
      .issuerSigners()
      .then((r) => {
        if (!cancelled) setCustodyState(custodyStateFromSigners(r));
      })
      .catch(() => {
        /* unauthenticated or backend down — stay `unknown` and do not redirect */
      });
    return () => {
      cancelled = true;
    };
  }, [opToken, custodyState, api]);

  // best-effort: load persisted signing mode once an operator session exists.
  useEffect(() => {
    if (!opToken) return;
    let cancelled = false;
    api
      .getSigningMode()
      .then((r) => {
        if (!cancelled) setSigningMode(r.signingMode);
      })
      .catch(() => {
        /* unauthenticated or backend down; keep default */
      });
    return () => {
      cancelled = true;
    };
  }, [opToken, api]);

  const value = useMemo<AppContextValue>(
    () => ({
      api,
      opToken,
      adminToken,
      setOpToken,
      setAdminToken,
      signerAddress,
      setSignerAddress,
      signingMode,
      setSigningMode,
      custodyState,
      setCustodyState,
      unlockPromptOpen,
      resolveUnlockPrompt,
      openUnlockPrompt,
      logout,
    }),
    [
      api,
      opToken,
      adminToken,
      setOpToken,
      setAdminToken,
      signerAddress,
      setSignerAddress,
      signingMode,
      custodyState,
      unlockPromptOpen,
      resolveUnlockPrompt,
      openUnlockPrompt,
      logout,
    ],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within <AppProvider>");
  return ctx;
}
