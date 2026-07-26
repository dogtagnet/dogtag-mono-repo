import {
  Button,
  CustodyUnlockPanel,
  NEXT_PARAM,
  sanitizeNextPath,
  useToast,
  DEMO_ADMIN_PASSWORD,
  DEMO_CUSTODY_PASSPHRASE,
} from "@dogtag/ui";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/** Where an unlock lands when there is no (or no safe) `?next=` destination to restore. */
const HOME = "/issue-dog-tag";

/**
 * The dedicated unlock route (§11.4). CustodyGate sends the operator here the moment the backend
 * reports a locked seal — after a restart, or on the first action that trips the lock — and this
 * page returns them to exactly where they were headed. Genesis stays in Setup; this page only ever
 * points at Setup when the instance has no seal at all.
 */
export function Unlock() {
  const { api, setAdminToken, setCustodyState, setSignerAddress } = useApp();
  const { toast } = useToast();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const next = sanitizeNextPath(params.get(NEXT_PARAM), HOME);

  function resume() {
    // Clear the lock BEFORE navigating, or CustodyGate bounces the destination straight back here.
    setCustodyState("unlocked");
    navigate(next, { replace: true });
  }

  return (
    <CustodyUnlockPanel
      portalLabel="Vet Portal"
      demoMode={env.demoMode}
      demoAdminPassword={DEMO_ADMIN_PASSWORD}
      demoPassphrase={DEMO_CUSTODY_PASSPHRASE}
      adminLogin={api.adminLogin}
      unlock={(passphrase) => api.unlock({ passphrase })}
      onAdminToken={setAdminToken}
      onAlreadyUnlocked={resume}
      onUnlocked={(accounts) => {
        if (accounts[0]?.address) setSignerAddress(accounts[0].address);
        toast({ title: "Custody unlocked", variant: "success" });
        resume();
      }}
      setupLink={
        <Button asChild className="w-full">
          <Link to="/setup">Go to Setup</Link>
        </Button>
      }
    />
  );
}
