# DogTag - Pet-Owner (holder) wallet

The consumer front of DogTag: the app the **pet owner** uses to **hold** their pet's credentials and
**present** zero-knowledge proofs of them. Issuers (vet, groomer, government) issue; verifiers verify;
this is the missing middle - the holder.

It mirrors, on the web, what the native Android/iOS apps do on a phone: a self-custodial wallet that
receives credentials, displays them, and generates the client-side "phone ZK" proof-of-verification.

## The holder loop

1. **Receive + store** - paste the wrapped credential document an issuer portal hands you (its
   "Copy wrapped document" button). Its integrity is checked offline (the Merkle root is recomputed)
   before it is held. Storage is local to the device (localStorage), idempotent by root.
2. **Display** - the wallet lists every held credential (pet, type, issuer, validity, integrity). The
   detail view decodes exactly the fields hashed into the on-chain root and reads on-chain validity
   (`DogTagIssuer.isValid`).
3. **Generate a ZK proof** - for a chosen credential + a verifier's request, the wallet builds the
   §1.10 consent and **EdDSA-BabyJubjub signs it in the browser** (the genuine client-side crypto, via
   `@dogtag/standard`), then asks the owner's **trusted prover-service** (`POST /prove-verification`)
   for the Groth16 proof. The verifier never sees the credential - only the resulting proof.
4. **Present + verify** - the wallet submits the consent + proof to the verifier
   (`POST /verify/consent/submit`), which relays it on-chain (the owner stays gasless) against the
   live `Groth16Verifier` + `VerificationRegistry`, then reports `recorded` with a tx hash + nullifier.

The wallet has **no backend of its own**. It talks directly to two hosts it is given at runtime: the
verifier it scanned (a `…/x/<token>` verify-session link) and the prover-service it trusts.

## Run

```bash
pnpm --filter @dogtag/owner-web dev        # http://localhost:45931
```

Point it at a trusted prover with `VITE_OWNER_PROVER_URL` (see `.env.example`); `scripts/demo-up.sh`
runs one at `:41875`. In a full local demo: issue a credential in the vet/government portal, copy the
wrapped doc, **Receive** it here, start a verify session in the groomer/verifier portal, paste its
`/x/<token>` link into **Present**, and watch it verify on ROAX.

## E2E

```bash
pnpm --filter @dogtag/owner-web test:e2e   # holder loop, mocked prover+verifier, real client crypto
OWNER_URL=https://<tunnel> pnpm --filter @dogtag/owner-web test:e2e   # against a live wallet
```

The default run starts its own dev server and **mocks the prover + verifier + ROAX RPC** at the
network layer so the loop is deterministic - but the client-side crypto (consent assembly, the
EdDSA-BabyJubjub signature, the EIP-712 bind signature) is 100% real. The funded on-chain present is
additionally exercised by the native mobile e2e and `scripts/e2e-zk.sh`.
