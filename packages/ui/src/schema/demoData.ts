/**
 * One-click "Fill demo data" presets for the clickable end-to-end demo. These produce VALID inputs
 * for each form so a non-technical operator can populate + submit without typing. The values mirror
 * the schema field paths in `recordTypes.ts` and the wire shapes in `api/types.ts`.
 *
 * NOTE: this is demo scaffolding only — it never bypasses the real backend validation; it just fills
 * the same fields the operator would otherwise type.
 */

/**
 * Demo credentials for the testnet click-through (scripts/demo-up.sh launches the backends with
 * ADMIN_PASSWORD=admin / OPERATOR_PASSWORD=operator). These prefill the login + admin-login inputs
 * so the operator just clicks Continue. TESTNET DEMO ONLY — never ship real secrets here.
 */
export const DEMO_ADMIN_PASSWORD = "admin";
export const DEMO_OPERATOR_PASSWORD = "operator";

/**
 * On-chain VACCINATION DogTagIssuer clone (VACCINATION_ISSUER_ADDR in scripts/demo-up.sh). This is
 * the documentStore the issuer-application must reference so the on-chain whitelist matches.
 */
export const DEMO_VACCINATION_DOCUMENT_STORE = "0x5c703910111f942EE0f47E02214291b5274cDb53";

/**
 * The single record type the vet/groomer backend accepts for issuance + whitelisting. The backend
 * resolves the issuer clone via issuer_addrs["VACCINATION"] and whitelists keccak256("VACCINATION"),
 * so EVERY demo (issue form, whitelist apply, issuer application) must use this literal.
 */
export const DEMO_RECORD_TYPE = "VACCINATION";

/**
 * Record types a demo VET (dog-tag issuer) requests: VACCINATION (issuance) PLUS DOG_PROFILE so that
 * approving the application in the admin portal ALSO grants DogTagSBT.ISSUER_ROLE (mint rights) — the
 * canonical grant now happens in the approve flow, not a separate cast. Groomers (verifiers) stay on
 * `DEMO_RECORD_TYPE` only (no DOG_PROFILE → no mint grant).
 */
export const DEMO_DOG_TAG_ISSUER_RECORD_TYPES = "VACCINATION, DOG_PROFILE";

/**
 * Demo VERIFY:<purpose> whitelist purposes for the groomer onboarding flow. On approval the central
 * backend whitelists VERIFY:<purpose> per address so the groomer can run on-chain verifications.
 * Mirrors the purposes the groomer Verify.tsx surfaces.
 */
export const DEMO_VERIFY_PURPOSES = "grooming_intake, boarding_intake, daycare_access";

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
 * Valid rabies-vaccination preset keyed by the flat dotted field paths used by the issue form (see
 * `RABIES_VACCINATION` in recordTypes.ts). recordType is the backend literal `VACCINATION`.
 * series=primary ⇒ validFrom = vaccinationDate + 21d. Returns `{ dogTagId, recordType, fields }`.
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
    recordType: DEMO_RECORD_TYPE,
    // NOT prefilled: the vaccination's dogTagId must be the operator-entered handle from the DOG_PROFILE
    // SBT issuance. A random/time-based value here had no minted SBT -> ownerOf(field_of_value(dogTagId))
    // reverted on the owner's ZK export (the iOS failure). The operator types the dog tag's handle.
    dogTagId: "",
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
  documentStores: DEMO_VACCINATION_DOCUMENT_STORE,
};

export const DEMO_BUSINESS_GROOMER: DemoBusiness = {
  type: "groomer",
  name: "Pawsh Grooming Studio",
  lat: "37.7849",
  lng: "-122.4094",
  services: "grooming, boarding, daycare",
  apiBaseUrl: "http://localhost:43618",
  domain: "groomer.dogtag.localhost",
  documentStores: DEMO_VACCINATION_DOCUMENT_STORE,
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
  /** comma-separated VERIFY:<purpose> purposes (optional). */
  verifyPurposes: string;
  domain: string;
  documentStore: string;
  usdaNan: string;
}

export const DEMO_ISSUER_APPLICATION_VET: DemoIssuerApplication = {
  issuerEntityId: "bayview-vet",
  addresses: "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96",
  // VACCINATION (keccak256 matches the on-chain whitelist key) + DOG_PROFILE so approving in the admin
  // portal ALSO grants DogTagSBT.ISSUER_ROLE (mint rights) to this signer.
  recordTypes: DEMO_DOG_TAG_ISSUER_RECORD_TYPES,
  // A vet onboards as an issuer; it doesn't need VERIFY purposes by default.
  verifyPurposes: "",
  domain: "vet.dogtag.localhost",
  documentStore: DEMO_VACCINATION_DOCUMENT_STORE,
  usdaNan: "123456",
};

export const DEMO_ISSUER_APPLICATION_GROOMER: DemoIssuerApplication = {
  issuerEntityId: "pawsh-groomer",
  addresses: "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96",
  recordTypes: DEMO_RECORD_TYPE,
  // A groomer onboards as a VERIFIER → whitelist VERIFY:<purpose> per address.
  verifyPurposes: DEMO_VERIFY_PURPOSES,
  domain: "groomer.dogtag.localhost",
  documentStore: DEMO_VACCINATION_DOCUMENT_STORE,
  usdaNan: "",
};

/**
 * Demo preset for the per-issuer Setup "Apply for whitelist" form (vet/groomer Setup.tsx). Shares
 * the issuer-application contract but the form collects a single signer address + flat license
 * fields. The signer address is left blank: the Setup wizard auto-fills it from the genesis-derived
 * signer (see DEMO_CLICKS.md).
 */
export interface DemoWhitelistApply {
  issuerEntityId: string;
  recordTypes: string;
  /** comma-separated VERIFY:<purpose> purposes (optional). */
  verifyPurposes: string;
  domain: string;
  documentStore: string;
  usdaNan: string;
  licenseNumber: string;
  licenseJurisdiction: string;
  licenseExpiry: string;
}

export const DEMO_WHITELIST_APPLY_VET: DemoWhitelistApply = {
  issuerEntityId: "seaport-vet",
  // Vet = dog-tag issuer: VACCINATION + DOG_PROFILE so admin approval grants ISSUER_ROLE (mint rights).
  recordTypes: DEMO_DOG_TAG_ISSUER_RECORD_TYPES,
  // Vets onboard as issuers; no VERIFY purposes by default.
  verifyPurposes: "",
  domain: "vet.local",
  documentStore: DEMO_VACCINATION_DOCUMENT_STORE,
  usdaNan: "123456",
  licenseNumber: "VET-2024-0001",
  licenseJurisdiction: "CA",
  licenseExpiry: "2027-12-31",
};

export const DEMO_WHITELIST_APPLY_GROOMER: DemoWhitelistApply = {
  issuerEntityId: "pampered-paws",
  recordTypes: DEMO_RECORD_TYPE,
  // Groomers onboard as verifiers → whitelist VERIFY:<purpose> per signer.
  verifyPurposes: DEMO_VERIFY_PURPOSES,
  domain: "groomer.local",
  documentStore: DEMO_VACCINATION_DOCUMENT_STORE,
  usdaNan: "",
  licenseNumber: "GRM-2024-0007",
  licenseJurisdiction: "CA",
  licenseExpiry: "2027-12-31",
};
