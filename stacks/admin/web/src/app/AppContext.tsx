import { createCentralClient, type CentralClient } from "@dogtag/ui";
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
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

  const central = useMemo(
    () =>
      createCentralClient({
        baseUrl: env.centralApiBase,
        getAdminToken: () => read(ADMIN_KEY),
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
