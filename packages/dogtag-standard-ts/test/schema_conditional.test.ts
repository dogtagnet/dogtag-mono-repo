import {describe, it, expect} from "vitest";
import {validateSchema, DOGTAG_CONTEXT_URI, SchemaError} from "../src/schema.js";

// These tests pin the conditional / jurisdiction-specific branches of validateSchema
// (impl §11.5) that schema.test.ts does not reach: the VC-envelope guards, the
// EU_HEALTH_CERT / CDC_IMPORT_FORM / DOG_PROFILE record types, the DOT trustLevel
// mutation, and the rabies date-math sub-rules. The EU onwardValid path in particular
// is the only caller of the private civil-date helpers (addMonths/civilFromDays/
// daysInMonth), so exercising it here closes the parity gap with the Rust schema.rs
// date-helper coverage (dogtag-standard-rs iteration 8).

const clone = <T>(o: T): T => JSON.parse(JSON.stringify(o));

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

/** Minimal credential carrying only the always-required envelope + legal meta. */
function validMinimal(): Record<string, unknown> {
  return {
    "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
    type: ["VerifiableCredential"],
    id: "urn:uuid:min-1",
    issuer: "did:web:issuer.example",
    validFrom: "2024-01-01",
    credentialSchema: {id: "https://dogtag.io/schemas/min", type: "JsonSchema"},
    credentialStatus: {id: "https://dogtag.io/status/1", type: "DogTagStatus2025"},
    attestationType: "generic",
    signatureTrustTier: "self_attested",
    legalEffect: "evidentiary",
    legalBasisVersion: "v1",
    jurisdiction: "EU",
    credentialSubject: {dogTagId: "dogtag:0x1"},
  };
}

/** Valid EU_HEALTH_CERT: requires a microchip object and the entry/onward date rules. */
function validEuHealthCert(): Record<string, unknown> {
  return {
    ...validMinimal(),
    type: ["VerifiableCredential"],
    recordType: "EU_HEALTH_CERT",
    validFrom: "2024-03-01",
    validUntilEntry: "2024-03-11", // validFrom + 10 days
    onwardValid: "2024-07-11", // entry (2024-03-11) + 4 months exactly
    echinococcusRequired: true,
    treatmentBeforeEntry: 48, // within [24h, 120h]
    credentialSubject: {
      dogTagId: "dogtag:0xeu",
      microchip: {
        code: "985141006580319",
        standard: "ISO_11784_11785",
        implantDate: "2023-10-01",
      },
    },
  };
}

function validDogProfile(): Record<string, unknown> {
  return {
    ...validMinimal(),
    recordType: "DOG_PROFILE",
    credentialSubject: {
      dogTagId: "dogtag:0xprof",
      species: "dog",
      breedVbo: "VBO-123",
      breedLabel: "Labrador",
      sex: "female",
      neuterStatus: "spayed",
      dateOfBirth: "2022-05-01",
      ownerIdentity: {countryOfIdentification: "DE", identification: "ID-1", name: "Jane"},
      weightHistory: [{unit: "kg", value: "12.5", measuredOn: "2024-01-01"}],
    },
  };
}

describe("validateSchema VC envelope guards", () => {
  it("accepts a minimal credential with no record-specific fields", () => {
    expect(() => validateSchema(validMinimal())).not.toThrow();
  });

  it("rejects a non-array @context", () => {
    const c = validMinimal();
    c["@context"] = "https://www.w3.org/ns/credentials/v2";
    expectViolation(c, "@context must be an array");
  });

  it("rejects @context missing the DogTag context URI", () => {
    const c = validMinimal();
    c["@context"] = ["https://www.w3.org/ns/credentials/v2"];
    expectViolation(c, "must include DogTag context URI");
  });

  it("rejects type missing VerifiableCredential", () => {
    const c = validMinimal();
    c.type = ["SomethingElse"];
    expectViolation(c, 'type must include "VerifiableCredential"');
  });

  it("rejects a non-evidentiary legalEffect", () => {
    const c = validMinimal();
    c.legalEffect = "binding";
    expectViolation(c, 'legalEffect must == "evidentiary"');
  });

  it("collects multiple violations at once", () => {
    const c = validMinimal();
    delete (c as Record<string, unknown>).id;
    delete (c as Record<string, unknown>).issuer;
    try {
      validateSchema(c);
      throw new Error("expected throw");
    } catch (e) {
      expect(e).toBeInstanceOf(SchemaError);
      expect((e as SchemaError).violations.length).toBeGreaterThanOrEqual(2);
    }
  });
});

describe("validateSchema EU_HEALTH_CERT", () => {
  it("accepts a valid EU_HEALTH_CERT", () => {
    expect(() => validateSchema(validEuHealthCert())).not.toThrow();
  });

  it("requires a microchip object (needsChip)", () => {
    const c = clone(validEuHealthCert());
    delete (c.credentialSubject as Record<string, unknown>).microchip;
    expectViolation(c, "credentialSubject.microchip must be an object");
  });

  it("rejects validUntilEntry != validFrom + 10 days", () => {
    const c = clone(validEuHealthCert());
    c.validUntilEntry = "2024-03-12"; // +11 days
    expectViolation(c, "validUntilEntry must == validFrom + 10 days");
  });

  it("rejects onwardValid beyond entry + 4 months (exercises addMonths)", () => {
    const c = clone(validEuHealthCert());
    c.onwardValid = "2024-07-12"; // one day past entry + 4 months
    expectViolation(c, "onwardValid must be <= entry + 4 months");
  });

  it("rejects echinococcus treatment outside [24h, 120h]", () => {
    const c = clone(validEuHealthCert());
    c.treatmentBeforeEntry = 12;
    expectViolation(c, "echinococcus treatmentBeforeEntry must be within");
  });
});

describe("validateSchema CDC_IMPORT_FORM", () => {
  it("accepts ageMonthsAtEntry >= 6", () => {
    const c = validMinimal();
    c.recordType = "CDC_IMPORT_FORM";
    c.ageMonthsAtEntry = 6;
    expect(() => validateSchema(c)).not.toThrow();
  });

  it("rejects ageMonthsAtEntry < 6", () => {
    const c = validMinimal();
    c.recordType = "CDC_IMPORT_FORM";
    c.ageMonthsAtEntry = 5;
    expectViolation(c, "ageMonthsAtEntry must be >= 6");
  });

  it("rejects a non-numeric ageMonthsAtEntry", () => {
    const c = validMinimal();
    c.recordType = "CDC_IMPORT_FORM";
    c.ageMonthsAtEntry = "6";
    expectViolation(c, "ageMonthsAtEntry must be >= 6");
  });
});

describe("validateSchema DOG_PROFILE", () => {
  it("accepts a valid DOG_PROFILE", () => {
    expect(() => validateSchema(validDogProfile())).not.toThrow();
  });

  it("rejects an invalid sex enum", () => {
    const c = clone(validDogProfile());
    (c.credentialSubject as Record<string, unknown>).sex = "unknown";
    expectViolation(c, "sex must be one of");
  });

  it("rejects ownerIdentity with a non-string sub-field", () => {
    const c = clone(validDogProfile());
    (c.credentialSubject as Record<string, any>).ownerIdentity.name = 42;
    expectViolation(c, "ownerIdentity.name must be a string");
  });

  it("rejects a weightHistory entry with a non-decimal value", () => {
    const c = clone(validDogProfile());
    (c.credentialSubject as Record<string, any>).weightHistory[0].value = "heavy";
    expectViolation(c, "weightHistory[0].value must be a decimal string");
  });
});

describe("validateSchema DOT trustLevel mutation", () => {
  it("sets trustLevel = SELF_ATTESTED for a DOT-typed credential", () => {
    const c = validMinimal();
    c.type = ["VerifiableCredential", "DOT"];
    const out = validateSchema(c);
    expect(out.trustLevel).toBe("SELF_ATTESTED");
  });
});

describe("validateSchema rabies date-math sub-rules", () => {
  // Minimal rabies fixture isolating each date rule (primary series validFrom rule
  // is already covered in schema.test.ts; these target the other isoDate-driven checks).
  function rabies(): Record<string, unknown> {
    return {
      ...validMinimal(),
      type: ["VerifiableCredential", "RabiesVaccinationCertificate"],
      signatureTrustTier: "licensed_vet",
      recordType: "VACCINATION",
      vaccineProductCode: "USDA-PCN-12345",
      vaccineProductName: "Rabvac 3",
      vaccineManufacturer: "Boehringer Ingelheim",
      batchLotNumber: "LOT-998",
      vaccinationDate: "2024-01-11",
      validFrom: "2024-02-01", // vaccinationDate + 21d
      validUntil: "2027-01-11",
      nextDueDate: "2027-01-11",
      authorizedVet: "did:web:vet.example#vet1",
      series: "primary",
      credentialSubject: {
        dogTagId: "dogtag:0xrab",
        dateOfBirth: "2023-09-01", // > 12 weeks before vaccination
        microchip: {
          code: "985141006580319",
          standard: "ISO_11784_11785",
          implantDate: "2023-10-01", // <= vaccinationDate
        },
      },
    };
  }

  it("accepts the isolated rabies fixture", () => {
    expect(() => validateSchema(rabies())).not.toThrow();
  });

  it("rejects microchip.implantDate after vaccinationDate", () => {
    const c = clone(rabies());
    (c.credentialSubject as Record<string, any>).microchip.implantDate = "2024-02-01";
    expectViolation(c, "implantDate must be <= vaccinationDate");
  });

  it("rejects animal age < 12 weeks at vaccination", () => {
    const c = clone(rabies());
    (c.credentialSubject as Record<string, any>).dateOfBirth = "2024-01-01"; // ~10 days old
    expectViolation(c, "age at vaccination must be >= 12 weeks");
  });
});
