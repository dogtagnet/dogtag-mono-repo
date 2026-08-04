/**
 * The vet portal's issuance-list page — LAYER 2 of the two-layer issuance requirement.
 *
 * Thin by design. The read comes from this deployment's own backend (operator-gated), and every
 * WRITE is a wallet transaction signed by the contract's owner, because `setIssuanceAllowed` admits
 * only from `owner()` and this backend is not it. All of that lives in the shared panel.
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
