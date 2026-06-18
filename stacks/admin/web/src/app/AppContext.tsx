import { createCentralClient, useToast, type CentralClient } from "@dogtag/ui";
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { env } from "../lib/env";

const ADMIN_KEY = "admin.token";

interface AppContextValue {
  central: CentralClient;
  adminToken: string | null;
  setAdminToken: (t: string | null) => void;
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
  const [adminToken, setAdminTokenState] = useState<string | null>(() => read(ADMIN_KEY));

  const setAdminToken = useCallback((t: string | null) => {
    setAdminTokenState(t);
    try {
      if (t) window.localStorage.setItem(ADMIN_KEY, t);
      else window.localStorage.removeItem(ADMIN_KEY);
    } catch {
      /* ignore */
    }
  }, []);

  // Route the 401 callback through a ref so the once-created client always clears the latest token.
  const onUnauthorizedRef = useRef<() => void>(() => {});
  onUnauthorizedRef.current = () => {
    setAdminToken(null);
    toast({
      title: "Session expired",
      description: "Your admin session is no longer valid — please log in again.",
      variant: "danger",
    });
  };

  const central = useMemo(
    () =>
      createCentralClient({
        baseUrl: env.centralApiBase,
        getAdminToken: () => read(ADMIN_KEY),
        onUnauthorized: () => onUnauthorizedRef.current(),
      }),
    [],
  );

  const logout = useCallback(() => setAdminToken(null), [setAdminToken]);

  const value = useMemo<AppContextValue>(
    () => ({ central, adminToken, setAdminToken, logout }),
    [central, adminToken, setAdminToken, logout],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within <AppProvider>");
  return ctx;
}
