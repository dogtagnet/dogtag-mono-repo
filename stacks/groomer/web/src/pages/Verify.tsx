import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  VerifyFlow,
  type VerifyPurpose,
} from "@dogtag/ui";
import { ShieldCheck } from "lucide-react";
import { useApp } from "../app/AppContext";

const PURPOSES: VerifyPurpose[] = [
  {
    value: "grooming_intake",
    label: "Grooming intake — rabies status",
    recordType: "RabiesVaccinationCertificate",
    sensitive: false,
  },
  {
    value: "boarding_intake",
    label: "Boarding intake — vaccination",
    recordType: "RabiesVaccinationCertificate",
    sensitive: true,
  },
  {
    value: "daycare_access",
    label: "Daycare access — health attestation",
    recordType: "HealthAttestation",
    sensitive: true,
  },
];

export function Verify() {
  const { api } = useApp();
  // Poll GET /verify/session/{id}; status flips pending → recorded once the owner consent is on chain.
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" /> Verify without being an issuer
          </CardTitle>
          <CardDescription>
            A groomer can record an on-chain proof-of-verification of a vet-issued vaccination{" "}
            <strong>without being a credential issuer</strong>. Verification authority lives in the{" "}
            <code>VERIFY:&lt;purpose&gt;</code> whitelist namespace, which is distinct from the issuer
            roles used to mint records.
          </CardDescription>
        </CardHeader>
        <CardContent className="text-sm text-muted">
          Pick a purpose and Normal/ZK mode below, start a session, and let the owner scan + approve
          consent. ZK is the default for sensitive purposes — no credential data is written on chain.
        </CardContent>
      </Card>
      <VerifyFlow
        client={api}
        purposes={PURPOSES}
        pollSession={async (id) => {
          const s = await api.verifySessionStatus(id);
          return { status: s.status, txHash: s.txHash ?? undefined };
        }}
      />
    </div>
  );
}
