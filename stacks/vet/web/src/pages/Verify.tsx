import { VerifyFlow, type VerifyPurpose } from "@dogtag/ui";
import { useApp } from "../app/AppContext";

const PURPOSES: VerifyPurpose[] = [
  { value: "boarding_intake", label: "Boarding intake — rabies status", recordType: "RabiesVaccinationCertificate", sensitive: false },
  { value: "travel_check", label: "Travel check — vaccination", recordType: "RabiesVaccinationCertificate", sensitive: true },
  { value: "service_dog_access", label: "Service-dog access", recordType: "SERVICE_ATTESTATION", sensitive: true },
];

export function Verify() {
  const { api } = useApp();
  // Poll GET /verify/session/{id}; status flips pending → recorded once the owner consent is on chain.
  return (
    <div className="space-y-4">
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
