// The pet owner's self-custodial identity — the holder's key material, held ENTIRELY on the device
// (localStorage). Two keys, exactly as the mobile wallet derives them (apps/*/wallet):
//
//   1. a secp256k1 wallet  -> the on-chain address (the credential SUBJECT / SBT owner). It ECDSA-
//      signs the EIP-712 BindConsentKey digest so a relayer can gaslessly bind the consent key.
//   2. a BabyJubjub consent key -> the in-circuit EdDSA key. keyHash = Poseidon(Ax, Ay) is bound
//      on-chain (ConsentKeyRegistry) and proven inside the circuit; the owner EdDSA-signs the §1.10
//      consent message with it. This is the sensitive "phone ZK" key and it never leaves the device.
//
// The BabyJubjub seed is derived deterministically from the wallet key (a distinct domain tag) so a
// single stored secret reconstitutes both keys. This mirrors the native apps' single-mnemonic model.
import { deriveBabyjubConsentKey, keyHash as consentKeyHash } from "@dogtag/standard";
import { keccak256, concatBytes, toBytes } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import type { Hex, LocalAccount } from "viem";

const STORAGE_KEY = "dogtag.owner.wallet.v1";
const CONSENT_SEED_DOMAIN = toBytes("dogtag/consent-key/v1");

export interface OwnerWallet {
  /** secp256k1 private key (0x + 64 hex) — the only persisted secret. */
  privateKey: Hex;
  /** the on-chain wallet address (the credential subject / SBT owner). */
  address: `0x${string}`;
  /** viem local account for ECDSA / EIP-712 signing. */
  account: LocalAccount;
  /** BabyJubjub consent private-key bytes (in-circuit EdDSA signer). */
  consentPrv: Uint8Array;
  /** BabyJubjub public point A = (Ax, Ay) as decimal-safe bigints. */
  Ax: bigint;
  Ay: bigint;
  /** keyHash = Poseidon(Ax, Ay), 0x + 64 hex — bound on-chain, proven in-circuit. */
  keyHash: string;
}

async function hydrate(privateKey: Hex): Promise<OwnerWallet> {
  const account = privateKeyToAccount(privateKey);
  // Distinct-domain seed: keccak256(privKey ‖ "dogtag/consent-key/v1") -> 32 bytes.
  const seed = toBytes(keccak256(concatBytes([toBytes(privateKey), CONSENT_SEED_DOMAIN])));
  const { prv, Ax, Ay } = await deriveBabyjubConsentKey(seed);
  return {
    privateKey,
    address: account.address,
    account,
    consentPrv: prv,
    Ax,
    Ay,
    keyHash: consentKeyHash(Ax, Ay),
  };
}

/** Load the stored owner wallet, creating (and persisting) a fresh one on first run. */
export async function loadOrCreateWallet(): Promise<OwnerWallet> {
  let pk = localStorage.getItem(STORAGE_KEY) as Hex | null;
  if (!pk) {
    pk = generatePrivateKey();
    localStorage.setItem(STORAGE_KEY, pk);
  }
  return hydrate(pk);
}

/** Wipe the wallet (and, by caller convention, the held credentials) — a hard reset for the demo. */
export function resetWallet(): void {
  localStorage.removeItem(STORAGE_KEY);
}
