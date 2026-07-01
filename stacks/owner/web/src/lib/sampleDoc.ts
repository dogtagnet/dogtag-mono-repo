import type { WrappedDoc } from "@dogtag/standard";

/**
 * A real, integrity-VALID DogTag credential (a rabies VACCINATION for "Rex", dog tag #424242),
 * produced by the SDK's `wrapDocument` with deterministic salts. Used by the "Fill sample" demo
 * button and the e2e test so the receive → hold → present flow can be exercised without a live
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
    documentStore: "0x16671686a5926606aB05f5e167fC65B0f8825B85",
    recordType: "VACCINATION",
  },
};

export const SAMPLE_WRAPPED_DOC_JSON = JSON.stringify(SAMPLE_WRAPPED_DOC, null, 2);
