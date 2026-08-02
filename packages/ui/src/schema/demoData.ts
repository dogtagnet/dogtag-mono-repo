/**
 * One-click "Fill demo data" presets for the clickable end-to-end demo. These produce VALID inputs
 * for each form so a non-technical operator can populate + submit without typing. The values mirror
 * the schema field paths in `recordTypes.ts` and the wire shapes in `api/types.ts`.
 *
 * NOTE: this is demo scaffolding only — it never bypasses the real backend validation; it just fills
 * the same fields the operator would otherwise type.
 */

import {
  PROVIDER_CONTACT_CHANNELS,
  type ContactChannelRecord,
} from "../directory/channels";

/**
 * Demo credentials for the testnet click-through (scripts/demo-up.sh launches the backends with
 * ADMIN_PASSWORD=admin / OPERATOR_PASSWORD=operator). These prefill the login + admin-login inputs
 * so the operator just clicks Continue. TESTNET DEMO ONLY — never ship real secrets here.
 */
export const DEMO_ADMIN_PASSWORD = "admin";
export const DEMO_OPERATOR_PASSWORD = "operator";

/**
 * Demo custody passphrase — prefilled into the genesis-confirm step AND the /unlock page so the
 * local click-through never asks anyone to remember a seed passphrase. TESTNET DEMO ONLY; a
 * production build (VITE_DEMO_MODE unset) never prefills it and the operator must type the real one.
 */
export const DEMO_CUSTODY_PASSPHRASE = "demo-pass-0000";

/**
 * On-chain VACCINATION DogTagIssuer clone (VACCINATION_ISSUER_ADDR in scripts/demo-up.sh). This is
 * the documentStore the issuer-application must reference so the on-chain whitelist matches.
 */
export const DEMO_VACCINATION_DOCUMENT_STORE = "0x1456f93f7376789c46408CC4616751eB853edD9A";

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

/**
 * Demo register-business presets for the admin register form (vet + groomer + contact-only).
 *
 * Every key of the register form must appear on this type: the form applies a preset by REPLACING
 * its whole state object, so a key missing here silently blanks that field.
 */
export type DemoBusiness = ContactChannelRecord<string> & {
  type: string;
  name: string;
  /** Blank for a provider with no location - see `DEMO_BUSINESS_CONTACT_ONLY`. */
  lat: string;
  lng: string;
  services: string;
  apiBaseUrl: string;
  domain: string;
  documentStores: string;
};

export const DEMO_BUSINESS_VET: DemoBusiness = {
  type: "vet",
  name: "Bayview Veterinary Clinic",
  lat: "37.7749",
  lng: "-122.4194",
  phone: "+1 415 555 0142",
  whatsapp: "",
  telegram: "",
  email: "reception@vet.dogtag.localhost",
  website: "https://vet.dogtag.localhost",
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
  phone: "+1 415 555 0188",
  whatsapp: "+1 415 555 0188",
  telegram: "",
  email: "hello@groomer.dogtag.localhost",
  website: "",
  services: "grooming, boarding, daycare",
  apiBaseUrl: "http://localhost:43618",
  domain: "groomer.dogtag.localhost",
  documentStores: DEMO_VACCINATION_DOCUMENT_STORE,
};

/**
 * A provider reached only by its contact channels - a mobile groomer with no premises.
 *
 * `lat`/`lng` are blank, which registers with NO location rather than the `0, 0` a blank field used
 * to become. It exists as a preset so the location-less path is one click away in the demo, since
 * it is the case most likely to be left untested.
 */
export const DEMO_BUSINESS_CONTACT_ONLY: DemoBusiness = {
  type: "groomer",
  name: "Roving Paws Mobile Grooming",
  lat: "",
  lng: "",
  phone: "+1 415 555 0170",
  whatsapp: "+1 415 555 0170",
  telegram: "@rovingpaws",
  email: "book@rovingpaws.dogtag.localhost",
  website: "https://rovingpaws.dogtag.localhost",
  services: "mobile grooming",
  apiBaseUrl: "http://localhost:43618",
  domain: "rovingpaws.dogtag.localhost",
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

/**
 * The admin signer from `contracts/deployments/roax.json`. Used wherever a demo preset must name an
 * address that really holds authority on the testnet, so the resulting action exercises the real
 * on-chain path rather than proposing against a stranger.
 */
export const DEMO_ADMIN_SIGNER = "0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2";

/**
 * Deploy-an-issuer-clone preset (admin Issuers/Factory).
 *
 * `recordType` is DELIBERATELY not `DEMO_RECORD_TYPE`. A clone's CREATE2 salt is
 * `keccak256(recordType, business)`, so re-using VACCINATION with the same business would collide
 * with the clone already deployed and the submit would revert - a demo button that fills a form
 * which then fails is worse than no button. A demo-only record type keeps the salt fresh.
 *
 * `business` is left blank on purpose: the backend defaults it to the hosted signer, which is the
 * single-authority topology every deployed clone already uses.
 */
export interface DemoIssuerDeploy {
  name: string;
  recordType: string;
  business: string;
}

export const DEMO_ISSUER_DEPLOY: DemoIssuerDeploy = {
  name: "Demo Issuer (testnet)",
  recordType: "DEMO_VACCINATION",
  business: "",
};

/**
 * Grant-a-capability preset (admin Whitelist). Grants BOTH axes at once - the record-type issuance
 * key and the `VERIFY:<purpose>` keys - because that pair is what the onboarding flow grants and a
 * preset covering one axis would leave the other looking broken.
 */
export interface DemoWhitelistGrant {
  signer: string;
  recordType: string;
  verifyPurposes: string;
}

export const DEMO_WHITELIST_GRANT: DemoWhitelistGrant = {
  signer: DEMO_ADMIN_SIGNER,
  recordType: DEMO_RECORD_TYPE,
  verifyPurposes: DEMO_VERIFY_PURPOSES,
};

/**
 * Register-a-provider preset (admin registrar surface, the generation-2 `ProviderRegistry`).
 *
 * `providerId` is left blank: the screen mints a CSPRNG id, and a fixed one would collide with
 * `AlreadyRegistered` the second time the captain walks the demo. `controller` is the well-known
 * anvil account 0 - the same key the first live provider was registered with, chosen so anyone can
 * act as it on a disposable testnet.
 */
export interface DemoProviderRegistration {
  controller: string;
  legalName: string;
  jurisdiction: string;
  registrationNumber: string;
  verifiedOn: string;
  notes: string;
}

export const DEMO_PROVIDER_REGISTRATION: DemoProviderRegistration = {
  controller: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
  legalName: "Demo Provider Pte Ltd (testnet)",
  jurisdiction: "SG",
  registrationNumber: "DEMO-KYC-0001",
  verifiedOn: isoDate(0),
  notes: "Demo registration - no KYC was performed.",
};

/**
 * Groomer CRM presets. Deliberately unmistakable as demo data: a real receptionist must never be
 * able to mistake a filled row for a real client, so every name says so and the contact details are
 * RFC-2606 reserved (`example.com`) rather than plausible-looking personal data.
 */
export interface DemoCrmPet {
  name: string;
  species: string;
  breed: string;
  sex: string;
  dateOfBirth: string;
  notes: string;
  microchipCode: string;
}

export const DEMO_CRM_PET: DemoCrmPet = {
  name: "Demo Dog (sample)",
  species: "Dog",
  breed: "Golden Retriever",
  sex: "female",
  dateOfBirth: isoDate(-1400),
  notes: "Demo record - not a real animal.",
  microchipCode: "900000000000001",
};

export interface DemoCrmClient {
  name: string;
  email: string;
  phone: string;
  address: string;
  notes: string;
  pet: DemoCrmPet;
}

export const DEMO_CRM_CLIENT: DemoCrmClient = {
  name: "Demo Client (sample)",
  email: "demo.client@example.com",
  phone: "+65 6000 0000",
  address: "1 Demo Street, #01-01, Singapore 000000",
  notes: "Demo record - not a real person.",
  pet: DEMO_CRM_PET,
};

/**
 * Book-an-appointment preset. `service`/`groomer` are the free-text fields the form collects; the
 * client, pet and times are chosen in the form itself (the client picker is a live search, so a
 * preset cannot name a client that exists on this shop's own database).
 */
export interface DemoCrmAppointment {
  service: string;
  groomer: string;
  notes: string;
}

export const DEMO_CRM_APPOINTMENT: DemoCrmAppointment = {
  service: "Demo full groom (sample)",
  groomer: "Demo Groomer",
  notes: "Demo booking - not a real appointment.",
};

/**
 * Provider self-service preset (vet + groomer `ProviderSelfServiceFlows`).
 *
 * `providerId` is deliberately ABSENT: it is registrar-assigned and opaque, so any value here would
 * name a provider that does not exist. The operator pastes the id the admin registrar screen minted.
 *
 * Contacts derive from the shared channel list rather than restating it, and are typed
 * `ContactChannelRecord` so a channel added to `PROVIDER_CONTACT_CHANNELS` FAILS TO COMPILE here
 * until it is given a demo value. A widened `Record<string, string>` with a `?? ""` fallback would
 * accept the addition silently and fill the new field with a blank - which is the exact silence
 * `channels.ts` exists to make impossible.
 */
export interface DemoProviderListing {
  recordType: string;
  domain: string;
  lat: string;
  lng: string;
  contacts: ContactChannelRecord<string>;
}

const DEMO_LISTING_CONTACTS: ContactChannelRecord<string> = {
  phone: "+65 6000 0000",
  whatsapp: "+65 6000 0000",
  telegram: "demoprovider",
  email: "demo.provider@example.com",
  website: "https://demo-provider.example.com",
};

export const DEMO_PROVIDER_LISTING: DemoProviderListing = {
  recordType: DEMO_RECORD_TYPE,
  domain: "demo-provider.example.com",
  // Singapore, to a sane precision. A real coordinate rather than 0,0 - which is a REAL place and
  // must never be used as a stand-in for "no location".
  lat: "1.290270",
  lng: "103.851959",
  contacts: Object.fromEntries(
    PROVIDER_CONTACT_CHANNELS.map((c) => [c, DEMO_LISTING_CONTACTS[c]]),
  ) as ContactChannelRecord<string>,
};
