import type { WrappedDoc } from "@dogtag/standard";

/**
 * A real, integrity-VALID DogTag credential (a rabies VACCINATION for "Rex", dog tag #424242),
 * produced by the SDK's `wrapDocument` with deterministic salts. Used by the "Fill sample" demo
 * button and the e2e test so the receive → hold → share flow can be exercised without a live
 * issuer. Its Merkle root is genuine — `checkIntegrity` passes on it.
 */
export const SAMPLE_WRAPPED_DOC: WrappedDoc = {
  version: "dogtag/1.0",
  data: {
    recordType: "030a11181f262d343b424950575e656c:2:VACCINATION",
    credentialSubject: {
      dogTagId: "222930373e454c535a61686f767d848b:3:424242",
      name: "41484f565d646b727980878e959ca3aa:2:Rex",
      species: "60676e757c838a91989fa6adb4bbc2c9:2:dog",
      vaccine: "7f868d949ba2a9b0b7bec5ccd3dae1e8:2:Rabies",
      manufacturer: "9ea5acb3bac1c8cfd6dde4ebf2f90007:2:Zoetis",
      lotNumber: "bdc4cbd2d9e0e7eef5fc030a11181f26:2:RB-2291",
      administeredOn: "dce3eaf1f8ff060d141b222930373e45:2:2026-05-14",
      validUntil: "fb020910171e252c333a41484f565d64:2:2029-05-13",
      veterinarian: "1a21282f363d444b525960676e757c83:2:Dr. A. Meyer, DVM",
    },
  },
  signature: {
    type: "DogTagMerkleProof",
    targetHash: "0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67",
    proof: [],
    merkleRoot: "0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67",
  },
  privacy: { obfuscated: [] },
  issuer: {
    name: "Seaport Vet",
    domain: "vet.local",
    documentStore: "0xe4aC139eB257C309Ec448C116A6F657Dab5590BA",
    recordType: "VACCINATION",
  },
};

export const SAMPLE_WRAPPED_DOC_JSON = JSON.stringify(SAMPLE_WRAPPED_DOC, null, 2);

/**
 * A real, integrity-VALID CDC-modeled TRAVEL_CLEARANCE credential ("Blaze", dog tag #7, receipt
 * 9RVBXK8AFQ2C), produced by the SDK's `wrapDocument` with deterministic salts. It carries the exact
 * nested `credentialSubject` the government backend issues (`build_gov_vc`, app.rs): Section A
 * (importer PII), Section B (animal), Section C (travel), a validity block and a public `receiptId`
 * leaf. Used by the "Fill travel-clearance sample" demo button + the receipt e2e so the CDC-grade
 * receipt renderer can be exercised without a live government issuer. Its Merkle root is genuine —
 * `checkIntegrity` passes on it.
 */
export const SAMPLE_TRAVEL_CLEARANCE_DOC: WrappedDoc = {
  version: "dogtag/1.0",
  data: {
    "@context": "030a11181f262d343b424950575e656c:2:v2",
    type: "20272e353c434a51585f666d747b8289:2:PetTravelClearance",
    id: "3d444b525960676e757c838a91989fa6:2:urn:dogtag:travel_clearance:7",
    issuer: "5a61686f767d848b9299a0a7aeb5bcc3:2:did:web:gov.example",
    recordType: "777e858c939aa1a8afb6bdc4cbd2d9e0:2:TRAVEL_CLEARANCE",
    attestationType: "949ba2a9b0b7bec5ccd3dae1e8eff6fd:2:authority_endorsement",
    signatureTrustTier: "b1b8bfc6cdd4dbe2e9f0f7fe050c131a:2:accredited_authority",
    legalEffect: "ced5dce3eaf1f8ff060d141b22293037:2:evidentiary",
    legalBasisVersion: "ebf2f900070e151c232a31383f464d54:2:EU-2013-576-v1",
    jurisdiction: "080f161d242b323940474e555c636a71:2:EU",
    credentialSubject: {
      dogTagId: "252c333a41484f565d646b727980878e:3:7",
      receiptId: "424950575e656c737a81888f969da4ab:2:9RVBXK8AFQ2C",
      validity: {
        validFrom: "5f666d747b828990979ea5acb3bac1c8:2:2026-07-01",
        validUntil: "7c838a91989fa6adb4bbc2c9d0d7dee5:2:2027-01-01",
        multipleEntries: "99a0a7aeb5bcc3cad1d8dfe6edf4fb02:1:true",
        countryOfDepartureBinding: "b6bdc4cbd2d9e0e7eef5fc030a11181f:2:CA",
      },
      importer: {
        firstName: "d3dae1e8eff6fd040b121920272e353c:2:Dominic",
        middleName: "f0f7fe050c131a21282f363d444b5259:2:",
        lastName: "0d141b222930373e454c535a61686f76:2:Zagara",
        role: "2a31383f464d545b626970777e858c93:2:owner",
        idType: "474e555c636a71787f868d949ba2a9b0:2:drivers_license",
        idJurisdiction: "646b727980878e959ca3aab1b8bfc6cd:2:US-NY",
        idNumber: "81888f969da4abb2b9c0c7ced5dce3ea:2:887524355",
        dateOfBirth: "9ea5acb3bac1c8cfd6dde4ebf2f90007:2:1997-02-13",
        email: "bbc2c9d0d7dee5ecf3fa01080f161d24:2:dom.zagara@example.com",
        phone: "d8dfe6edf4fb020910171e252c333a41:2:216-533-5925",
      },
      consignee: {
        fullName: "f5fc030a11181f262d343b424950575e:2:",
        email: "121920272e353c434a51585f666d747b:2:",
        idType: "2f363d444b525960676e757c838a9198:2:",
      },
      animal: {
        name: "4c535a61686f767d848b9299a0a7aeb5:2:Blaze",
        ageYears: "6970777e858c939aa1a8afb6bdc4cbd2:3:3",
        ageMonths: "868d949ba2a9b0b7bec5ccd3dae1e8ef:3:1",
        sex: "a3aab1b8bfc6cdd4dbe2e9f0f7fe050c:2:male",
        neutered: "c0c7ced5dce3eaf1f8ff060d141b2229:1:true",
        breed: "dde4ebf2f900070e151c232a31383f46:2:Poodle - Standard",
        colorMarkings: "fa01080f161d242b323940474e555c63:2:Grey",
        microchipNumber: "171e252c333a41484f565d646b727980:2:985112345678903",
        importationPurpose: "343b424950575e656c737a81888f969d:2:service_animal",
      },
      travel: {
        travelType: "51585f666d747b828990979ea5acb3ba:2:air",
        countryOfDeparture: "6e757c838a91989fa6adb4bbc2c9d0d7:2:CA",
        dateOfArrival: "8b9299a0a7aeb5bcc3cad1d8dfe6edf4:2:2026-07-08",
        portOfEntry: "a8afb6bdc4cbd2d9e0e7eef5fc030a11:2:JFK",
        carrierOrFlight: "c5ccd3dae1e8eff6fd040b121920272e:2:AC 8552",
      },
      endorsingAuthority: "e2e9f0f7fe050c131a21282f363d444b:2:Example Competent Authority",
    },
  },
  signature: {
    type: "DogTagMerkleProof",
    targetHash: "0x010a607eb1f94fd672622331ae1272c5e08afba9b6d094b52b5b5e3a2bec4a45",
    proof: [],
    merkleRoot: "0x010a607eb1f94fd672622331ae1272c5e08afba9b6d094b52b5b5e3a2bec4a45",
  },
  privacy: { obfuscated: [] },
  issuer: {
    name: "Example Competent Authority",
    domain: "gov.example",
    documentStore: "0x1111111111111111111111111111111111111111",
    recordType: "TRAVEL_CLEARANCE",
  },
  // The provenance block an issuing stack stamps. `statusBaseUrl` is the REACHABLE origin the receipt
  // QR points at - note it is deliberately NOT `issuer.domain` (`gov.example`), which is a `did:web`
  // identity and RFC-2606 reserved. This block sits outside the Merkle root, so its presence does not
  // move `signature.merkleRoot` above.
  protocol: {
    chainId: 135,
    version: "dogtag-levelb/1",
    verificationRegistry: "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B",
    issuerClone: "0x1111111111111111111111111111111111111111",
    issuerSigner: "0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2",
    statusBaseUrl: "https://travel.authority.example-demo.net",
  },
};

export const SAMPLE_TRAVEL_CLEARANCE_DOC_JSON = JSON.stringify(SAMPLE_TRAVEL_CLEARANCE_DOC, null, 2);
