import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  CredentialVerifyPanel,
  VerifyFlow,
  useRoaxRpcPreference,
} from "@dogtag/ui";
import { ShieldCheck } from "lucide-react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";
import { VERIFY_PURPOSES } from "./verifyPurposes";

/**
 * AD-HOC verification — a walk-in with no booking.
 *
 * The appointment-linked flow on the appointment page is the primary one; this is the SAME
 * `VerifyFlow` machinery without the business context, so a verification means the same thing
 * however it was started — it just lands in "All verifications" as an unlinked row. Use it when
 * there is genuinely no appointment to attach it to.
 *
 * The groomer's verification HISTORY lives on its own "All verifications" page (searchable, joined
 * to appointment and client), so this page deliberately carries no second, page-local history list.
 */
export function Verify() {
  const { api } = useApp();
  const rpc = useRoaxRpcPreference(env.roaxRpc);
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" /> Ad-hoc verification
          </CardTitle>
          <CardDescription>
            For a walk-in with no booking. If this pet has an appointment, start the check from{" "}
            <Link to="/appointments" className="text-primary hover:underline">
              the appointment
            </Link>{" "}
            instead — the result is then filed against that visit and its client.
          </CardDescription>
        </CardHeader>
        <CardContent className="text-sm text-muted">
          You can record an on-chain proof-of-verification of a vet-issued vaccination{" "}
          <strong>without being a credential issuer</strong>. The owner <strong>approves</strong> a
          proof from their app; your authority to verify lives in the{" "}
          <code>VERIFY:&lt;purpose&gt;</code> whitelist namespace, which is separate from the issuer
          roles used to mint records. No owner identity or credential data is written on chain.
        </CardContent>
      </Card>
      {/* Permissionless, direct-to-RPC check of a credential the owner simply pasted in. */}
      <CredentialVerifyPanel rpcUrl={rpc.rpcUrl} defaultRpcUrl={rpc.defaultRpcUrl} />
      <VerifyFlow
        client={api}
        purposes={VERIFY_PURPOSES}
        showDemo={env.demoMode}
        pollSession={async (id) => {
          const s = await api.verifySessionStatus(id);
          return { status: s.status, txHash: s.txHash ?? undefined };
        }}
      />
    </div>
  );
}
