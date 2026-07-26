import { buildUnlockPath, isLockExemptPath } from "@dogtag/ui";
import type { ReactNode } from "react";
import { Navigate, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";

/**
 * Sends the operator to the dedicated /unlock page whenever the backend's custody seal is locked,
 * remembering where they were headed so the unlock can put them back.
 *
 * Deliberately narrow: it acts ONLY on an observed `locked` state (the on-load probe or a "not
 * unlocked" refusal). `unknown` — the initial state, and where a down backend leaves us — renders
 * the app untouched, because a passphrase cannot fix a backend that is not answering.
 *
 * /setup stays reachable while locked: it owns genesis, the only cure for a seal-less instance.
 */
export function CustodyGate({ children }: { children: ReactNode }) {
  const { custodyState } = useApp();
  const location = useLocation();

  if (custodyState !== "locked" || isLockExemptPath(location.pathname)) return <>{children}</>;
  return <Navigate to={buildUnlockPath(`${location.pathname}${location.search}`)} replace />;
}
