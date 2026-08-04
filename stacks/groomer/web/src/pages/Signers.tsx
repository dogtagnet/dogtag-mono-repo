/**
 * The groomer portal's issuance-list page — LAYER 2 of the two-layer issuance requirement.
 *
 * Thin by design. The read comes from this deployment's own backend (operator-gated), and every
 * WRITE is a wallet transaction signed by the contract's owner, because `setIssuanceAllowed` admits
 * only from `owner()` and this backend is not it. All of that lives in the shared panel.
 *
 * MOUNTED ON THE GROOMER TOO, deliberately, even though a groomer does not issue. `demo-up.sh`
 * configures every vet-api instance with the same clones, so a groomer's backend signer really can
 * appear on one of these lists - and REMOVAL is the safety direction, so the role that cannot issue
 * must still be able to see who may sign in its name and withdraw a key it no longer trusts. That is
 * why this read route is mounted in the PUBLIC router rather than the issuance one.
 */

import { IssuerSignersPanel } from "@dogtag/ui";
import { useCallback } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

export function Signers() {
  const { api } = useApp();
  const load = useCallback(() => api.issuanceAllowed(), [api]);
  return <IssuerSignersPanel load={load} rpcUrl={env.roaxRpc} />;
}

export default Signers;
