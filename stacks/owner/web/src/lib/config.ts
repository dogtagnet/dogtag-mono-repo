// Live ROAX (chainId 135) network the owner wallet reads for credential validity.
export const ROAX_CHAIN_ID = 135;
export const ROAX_RPC_URL = "https://devrpc.roax.net";
export const ROAX_EXPLORER_URL = "https://explorer.roax.net";

// The live owner-hidden VerificationRegistryConsent (contracts/deployments/roax.json) - the sole
// emitter of the owner-blind `Verified` events the consent-history surface reads.
export const VERIFICATION_REGISTRY_CONSENT_ADDR =
  "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87" as const;
