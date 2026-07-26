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
 * The dedicated unlock route: the FALLBACK surface, for arriving at an already-locked backend or
 * following a direct link. The primary path is the point-of-need dialog raised over whatever page
 * the operator is on (see Layout's CustodyPrompt), which preserves their work; this page exists so
 * "unlock" is also a place you can simply go. It restores `?next=` on success. Genesis stays in
 * Setup; this page points there only when the instance has no seal at all.
 */
export function Unlock() {
  const { api, adminToken, setAdminToken, setCustodyState, setSignerAddress } = useApp();
  const { toast } = useToast();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const next = sanitizeNextPath(params.get(NEXT_PARAM), HOME);

  function resume() {
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
      adminToken={adminToken}
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
