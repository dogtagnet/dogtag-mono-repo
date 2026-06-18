import { createApiClient, useToast, type ApiClient, type SigningMode } from "@dogtag/ui";
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
  /** in-memory unlock flag — the backend keeps the real state; this gates optimistic UI */
  unlocked: boolean;
  setUnlocked: (v: boolean) => void;
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
  const [unlocked, setUnlocked] = useState(false);

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
    setUnlocked(false);
    toast({
      title: "Session expired",
      description: "Your session is no longer valid — please log in again.",
      variant: "danger",
    });
  };

  const api = useMemo(
    () =>
      createApiClient({
        baseUrl: env.vetApiBase,
        centralBaseUrl: env.centralApiBase,
        getOperatorToken: () => read(OP_KEY),
        getAdminToken: () => read(ADMIN_KEY),
        onUnauthorized: (kind) => onUnauthorizedRef.current(kind),
      }),
    [],
  );

  const logout = useCallback(() => {
    setOpToken(null);
    setAdminToken(null);
    setUnlocked(false);
  }, [setOpToken, setAdminToken]);

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
      unlocked,
      setUnlocked,
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
      unlocked,
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
