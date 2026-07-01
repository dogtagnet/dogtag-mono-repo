// Live ROAX (chainId 135) deployment the owner wallet reads/writes against. Source of truth:
// contracts/deployments/roax.json. These are used for the on-chain validity check (DogTagIssuer.
// isValid) and the consent-key bind (ConsentKeyRegistry). The Groth16 verifier below is the
// CURRENTLY-LIVE one — a v2 swap is scheduled separately and is NOT this app's concern.
export const ROAX_CHAIN_ID = 135;
export const ROAX_RPC_URL = "https://devrpc.roax.net";
export const ROAX_EXPLORER = "https://explorer.roax.net";

export const CONTRACTS = {
  ConsentKeyRegistry: "0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671",
  Groth16Verifier: "0x138b433071Ad806E841B5AD53623290a9bf21761",
  VerificationRegistry: "0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1",
  IssuerRegistry: "0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c",
  DogTagSBT: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
} as const;

/**
 * The owner's TRUSTED prover-service base URL (`POST /prove-verification`). A 64-bit phone proves
 * on-device; a browser wallet delegates the heavy Groth16 proving to a prover the owner trusts (their
 * own, or a service they run) — the same "owner's prover" model as the native 32-bit fallback. The
 * verifier NEVER sees the witness, only the resulting proof. Configurable via `VITE_OWNER_PROVER_URL`.
 */
export const PROVER_URL: string =
  (import.meta.env.VITE_OWNER_PROVER_URL as string | undefined)?.replace(/\/+$/, "") ||
  "http://localhost:41875";

export function explorerTxUrl(txHash: string): string {
  return `${ROAX_EXPLORER}/tx/${txHash}`;
}
