/**
 * One-click "Fill demo data" presets for the clickable end-to-end demo. These produce VALID inputs
 * for each form so a non-technical operator can populate + submit without typing. The values mirror
 * the schema field paths in `recordTypes.ts` and the wire shapes in `api/types.ts`.
 *
 * NOTE: this is demo scaffolding only — it never bypasses the real backend validation; it just fills
 * the same fields the operator would otherwise type.
 */

/** YYYY-MM-DD `daysFromToday` days from now (UTC). */
export function isoDate(daysFromToday = 0): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() + daysFromToday);
  return d.toISOString().slice(0, 10);
}

/** A pseudo-random-ish but deterministic-per-call microchip code (15 digits). */
function demoMicrochip(): string {
  const tail = String(Date.now()).slice(-9).padStart(9, "0");
  return `985112${tail}`; // 6 + 9 = 15 digits, ISO 11784/11785 manufacturer-ish prefix
}

/**
 * Valid RabiesVaccinationCertificate preset keyed by the flat dotted field paths used by the
 * issue form (see `RABIES_VACCINATION` in recordTypes.ts). series=primary ⇒ validFrom =
 * vaccinationDate + 21d. Returns `{ dogTagId, recordType, fields }` for the issue form.
 */
export function demoRabiesIssue(): {
  recordType: string;
  dogTagId: string;
  fields: Record<string, string>;
} {
  const vaccinationDate = isoDate(-7); // a week ago
  const validFrom = isoDate(-7 + 21); // primary ⇒ +21d
  const validUntil = isoDate(-7 + 365); // 1y validity
  const nextDueDate = isoDate(-7 + 365);
  const implantDate = isoDate(-120); // chipped well before vaccination
  return {
    recordType: "RabiesVaccinationCertificate",
    dogTagId: `dtag:demo:${String(Date.now()).slice(-6)}`,
    fields: {
      "microchip.code": demoMicrochip(),
      "microchip.standard": "ISO_11784_11785",
      "microchip.implantDate": implantDate,
      vaccineProductCode: "1351.20",
      vaccineProductName: "RABVAC 3 TF",
      vaccineManufacturer: "Boehringer Ingelheim",
      batchLotNumber: "LOT-2026-A17",
      series: "primary",
      vaccinationDate,
      validFrom,
      validUntil,
      nextDueDate,
      authorizedVet: "Dr. Casey Rivera, DVM",
    },
  };
}

/** Demo register-business presets for the admin register form (vet + groomer). */
export interface DemoBusiness {
  type: string;
  name: string;
  lat: string;
  lng: string;
  services: string;
  apiBaseUrl: string;
  domain: string;
  documentStores: string;
}

export const DEMO_BUSINESS_VET: DemoBusiness = {
  type: "vet",
  name: "Bayview Veterinary Clinic",
  lat: "37.7749",
  lng: "-122.4194",
  services: "vaccination, microchip, wellness",
  apiBaseUrl: "http://localhost:41874",
  domain: "vet.dogtag.localhost",
  documentStores: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
};

export const DEMO_BUSINESS_GROOMER: DemoBusiness = {
  type: "groomer",
  name: "Pawsh Grooming Studio",
  lat: "37.7849",
  lng: "-122.4094",
  services: "grooming, boarding, daycare",
  apiBaseUrl: "http://localhost:43618",
  domain: "groomer.dogtag.localhost",
  documentStores: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
};

/**
 * Demo issuer-application preset. The default address is the deployed `admin` signer from
 * contracts/deployments/roax.json so an approve actually exercises whitelistFor on-chain in a
 * local/dev deployment. Callers may override the address.
 */
export interface DemoIssuerApplication {
  issuerEntityId: string;
  addresses: string;
  recordTypes: string;
  domain: string;
  documentStore: string;
  usdaNan: string;
}

export const DEMO_ISSUER_APPLICATION_VET: DemoIssuerApplication = {
  issuerEntityId: "bayview-vet",
  addresses: "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96",
  recordTypes: "RabiesVaccinationCertificate",
  domain: "vet.dogtag.localhost",
  documentStore: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
  usdaNan: "123456",
};

export const DEMO_ISSUER_APPLICATION_GROOMER: DemoIssuerApplication = {
  issuerEntityId: "pawsh-groomer",
  addresses: "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96",
  recordTypes: "HealthAttestation",
  domain: "groomer.dogtag.localhost",
  documentStore: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
  usdaNan: "",
};
