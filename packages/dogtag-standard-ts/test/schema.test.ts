import {describe, it, expect} from "vitest";
import {validateSchema, DOGTAG_CONTEXT_URI, SchemaError} from "../src/schema.js";

// deep clone helper so each case mutates a fresh fixture
const clone = <T>(o: T): T => JSON.parse(JSON.stringify(o));

/** Fully-valid rabies vaccination credential fixture (primary series, valid titer). */
function validRabies(): Record<string, unknown> {
  return {
    "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
    type: ["VerifiableCredential", "RabiesVaccinationCertificate"],
    id: "urn:uuid:rabies-1",
    issuer: "did:web:vet.example",
    validFrom: "2024-02-01", // vaccinationDate + 21d
    validUntil: "2027-01-11",
    nextDueDate: "2027-01-11",
    credentialSchema: {id: "https://dogtag.io/schemas/rabies", type: "JsonSchema"},
    credentialStatus: {id: "https://dogtag.io/status/1", type: "DogTagStatus2025"},
    attestationType: "vaccination",
    signatureTrustTier: "licensed_vet",
    legalEffect: "evidentiary",
    legalBasisVersion: "EU-2013-576-v1",
    jurisdiction: "EU",
    recordType: "VACCINATION",
    vaccineProductCode: "USDA-PCN-12345",
    vaccineProductName: "Rabvac 3",
    vaccineManufacturer: "Boehringer Ingelheim",
    batchLotNumber: "LOT-998",
    vaccinationDate: "2024-01-11",
    authorizedVet: "did:web:vet.example#vet1",
    series: "primary",
    titer: {
      labId: "LAB-7",
      sampledAt: "2024-02-11", // vaccinationDate + 31d (>= +30d)
      resultIUml: "0.7",
    },
    credentialSubject: {
      dogTagId: "dogtag:0xabc",
      dateOfBirth: "2023-09-01", // > 12 weeks before vaccination
      microchip: {
        code: "985141006580319",
        standard: "ISO_11784_11785",
        implantDate: "2023-10-01", // <= vaccinationDate
      },
    },
  };
}

function validServiceAttestation(): Record<string, unknown> {
  return {
    "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
    type: ["VerifiableCredential", "ServiceAttestation"],
    id: "urn:uuid:svc-1",
    issuer: "did:web:trainer.example",
    validFrom: "2024-01-01",
    credentialSchema: {id: "https://dogtag.io/schemas/svc", type: "JsonSchema"},
    credentialStatus: {id: "https://dogtag.io/status/2", type: "DogTagStatus2025"},
    attestationType: "service",
    signatureTrustTier: "self_attested",
    legalEffect: "evidentiary",
    legalBasisVersion: "ADA-v1",
    jurisdiction: "US",
    recordType: "SERVICE_ATTESTATION",
    assistanceType: "service_dog",
    issuerTrustTier: "adi_accredited",
    taskDescription: "mobility assistance",
    legalContext: ["ADA", "ACAA"],
    storage: "off_chain",
    credentialSubject: {dogTagId: "dogtag:0xdef"},
  };
}

function expectViolation(c: Record<string, unknown>, needle: string) {
  try {
    validateSchema(c);
    throw new Error("expected validateSchema to throw");
  } catch (e) {
    expect(e).toBeInstanceOf(SchemaError);
    const v = (e as SchemaError).violations.join("\n");
    expect(v).toContain(needle);
  }
}

describe("validateSchema", () => {
  it("accepts a fully-valid rabies vaccination credential", () => {
    expect(() => validateSchema(validRabies())).not.toThrow();
  });

  it("fails when vaccineManufacturer is missing", () => {
    const c = clone(validRabies());
    delete (c as Record<string, unknown>).vaccineManufacturer;
    expectViolation(c, "vaccineManufacturer is required");
  });

  it("fails when microchip.code is 14 digits", () => {
    const c = clone(validRabies());
    (c.credentialSubject as Record<string, any>).microchip.code = "98514100658031"; // 14 digits
    expectViolation(c, "microchip.code must match");
  });

  it("fails when microchip.code is a number (non-string)", () => {
    const c = clone(validRabies());
    (c.credentialSubject as Record<string, any>).microchip.code = 985141006580319;
    expectViolation(c, "microchip.code must be a string");
  });

  it("fails when signatureTrustTier is bogus", () => {
    const c = clone(validRabies());
    c.signatureTrustTier = "bogus";
    expectViolation(c, "signatureTrustTier must be one of");
  });

  it("fails when primary series validFrom != vaccinationDate + 21d", () => {
    const c = clone(validRabies());
    c.validFrom = "2024-02-02"; // +22d
    expectViolation(c, "validFrom must == vaccinationDate + 21 days");
  });

  it("fails when titer resultIUml is 0.4", () => {
    const c = clone(validRabies());
    (c.titer as Record<string, unknown>).resultIUml = "0.4";
    expectViolation(c, 'titer.resultIUml must be >= "0.5"');
  });

  it("passes when titer resultIUml is exactly 0.5", () => {
    const c = clone(validRabies());
    (c.titer as Record<string, unknown>).resultIUml = "0.5";
    expect(() => validateSchema(c)).not.toThrow();
  });

  it("accepts a valid SERVICE_ATTESTATION", () => {
    expect(() => validateSchema(validServiceAttestation())).not.toThrow();
  });

  it("fails SERVICE_ATTESTATION when storage != off_chain", () => {
    const c = clone(validServiceAttestation());
    c.storage = "on_chain";
    expectViolation(c, 'storage must == "off_chain"');
  });
});
