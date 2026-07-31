// Bundled fallback for the guarded ROAX (chainId 135) reads; Settings may choose a same-chain peer.
export const ROAX_CHAIN_ID = 135;
export const ROAX_RPC_URL = "https://devrpc.roax.net";
export const ROAX_EXPLORER_URL = "https://explorer.roax.net";

// The live owner-hidden VerificationRegistryConsent (contracts/deployments/roax.json) - the sole
// emitter of the owner-blind `Verified` events the consent-history surface reads.
//
// This carried the SUPERSEDED M5 instance (0xb9B313C1...) until S-13, so the scan in chain.ts read a
// dead contract and the owner saw "no consent history" for absent evidence rather than for an absent
// history. Take the value from the ledger's canonical `VerificationRegistryConsent` key - a retired
// instance still answers eth_getLogs, so a wrong one here fails silently and looks like an empty
// history forever. `make check-cutover-consumers` now fails on any undeclared carrier of a retired
// address; this constant moves again at cutover step C-9.
export const VERIFICATION_REGISTRY_CONSENT_ADDR =
  "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B" as const;
