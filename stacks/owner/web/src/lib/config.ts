// Bundled fallback for the guarded ROAX (chainId 135) reads; Settings may choose a same-chain peer.
export const ROAX_CHAIN_ID = 135;
export const ROAX_RPC_URL = "https://devrpc.roax.net";
export const ROAX_EXPLORER_URL = "https://explorer.roax.net";

// The owner-hidden VerificationRegistryConsent - the sole emitter of the owner-blind `Verified`
// events the consent-history surface reads. READ FROM CONFIGURATION; the value lives in
// contracts/deployments/roax.json and reaches this build through the environment.
//
// It was a literal, and carried the SUPERSEDED M5 instance for long enough that the scan in chain.ts
// read a dead contract and the owner saw "no consent history" for absent EVIDENCE rather than for an
// absent history. That is the failure worth naming, because it is silent in both directions: a
// retired instance still answers `eth_getLogs` perfectly well, so a wrong address here is
// indistinguishable from an owner who has consented to nothing, forever.
//
// Unset is the empty string, which `chain.ts` must treat as "cannot scan" - never as an address to
// dial, and never as an empty history. `make check-addresses` keeps the literal from coming back.
export const VERIFICATION_REGISTRY_CONSENT_ADDR = (
  import.meta.env.VITE_VERIFICATION_REGISTRY_CONSENT_ADDR ?? ""
).trim();
