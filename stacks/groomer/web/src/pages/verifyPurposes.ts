import type { VerifyPurpose } from "@dogtag/ui";

/**
 * The `VERIFY:<purpose>` namespaces this shop is whitelisted for.
 *
 * Shared by the appointment-linked flow (AppointmentDetail) and the ad-hoc one (Verify) so both
 * offer exactly the same purposes — a verification must mean the same thing however it was started.
 * `sensitive` purposes default to ZK, where the owner discloses nothing at all.
 */
export const VERIFY_PURPOSES: VerifyPurpose[] = [
  {
    value: "grooming_intake",
    label: "Grooming intake — rabies status",
    recordType: "VACCINATION",
    sensitive: false,
  },
  {
    value: "boarding_intake",
    label: "Boarding intake — vaccination",
    recordType: "VACCINATION",
    sensitive: true,
  },
  {
    value: "daycare_access",
    label: "Daycare access — health attestation",
    recordType: "HealthAttestation",
    sensitive: true,
  },
];
