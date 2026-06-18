import { VerifyFlow, type VerifyPurpose } from "@dogtag/ui";
import { useApp } from "../app/AppContext";

const PURPOSES: VerifyPurpose[] = [
  { value: "boarding_intake", label: "Boarding intake — rabies status", recordType: "RabiesVaccinationCertificate", sensitive: false },
  { value: "travel_check", label: "Travel check — vaccination", recordType: "RabiesVaccinationCertificate", sensitive: true },
  { value: "service_dog_access", label: "Service-dog access", recordType: "SERVICE_ATTESTATION", sensitive: true },
];

export function Verify() {
  const { api } = useApp();
  // routes.rs exposes no GET session-status endpoint; VerifyFlow polls only if a poller is given.
  // Omitted here → the QR + awaiting-consent state is shown; status advances when a poller is wired.
  return (
    <div className="space-y-4">
      <VerifyFlow client={api} purposes={PURPOSES} />
    </div>
  );
}
