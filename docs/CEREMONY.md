# DogTag - ZK Trusted-Setup Ceremony (production)

> **Live circuit: `circuits/consent.circom` (`DogTagConsent(6)`) - the owner-hidden consent circuit.**
> This is the concise ceremony guide for the ONE circuit the protocol runs on.
> The testnet key securing it today came from the documented single-operator self-run `circuits/scripts/ceremony-consent.sh` (transcript: [`CEREMONY_TRANSCRIPT.consent.md`](./CEREMONY_TRANSCRIPT.consent.md)); production requires the multi-party run described here.
>
> **History:** an earlier ceremony was run for the since-retired owner-revealing `verification.circom` (transcript: [`CEREMONY_TRANSCRIPT.md`](./CEREMONY_TRANSCRIPT.md), kept frozen as provenance).
> That circuit, its ceremony scripts (`scripts/ceremony.sh`, `scripts/setup.sh`) and the `npm run compile-circuit` / `npm run build-circuit` package scripts were all removed with the owner-revealing layer; commands referencing them no longer resolve anywhere in this repo.

> **See also** [`CEREMONY_RUNBOOK.md`](./CEREMONY_RUNBOOK.md) - the expanded, captain-fill-in production runbook (participant slot table, attestation/transcript format, air-gapped step detail).
> This file is the concise version.

The Groth16 owner-hidden consent path needs a circuit-specific **proving/verifying key** produced by a **multi-party ceremony**.
Two non-production setups exist and must never secure a real deployment:

- `circuits/scripts/setup-consent.sh` (`npm run build-consent`) is the throwaway **DEV** setup - a self-generated ptau with a single contributor, forgeable by construction, used only by the circuit test.
  Running it **overwrites the committed ceremony zkey/VK** - avoid.
- `circuits/scripts/ceremony-consent.sh` as committed performs a **single-operator testnet self-run** (public Hermez ptau + ONE contribution + a public drand beacon).
  It is real enough for the disposable ROAX testnet, but a sole contributor could retain the toxic waste and forge consent attestations.

Production needs **>=3 independent contributors + a public random beacon**, then wires the resulting verifier into the live `VerificationRegistryConsent` via its 2-day timelock.

> The Merkle-proof side of the protocol (issuance anchoring, integrity, selective disclosure - the three-pillar trust model) does **not** depend on this ceremony at all.
> The ceremony gates the **ZK consent** path (`recordVerificationZK`), which is the only on-chain verification path, so the key that secures it is the key that secures verification.

Circuit: `DogTagConsent(6)`, 38,501 non-linear constraints -> **2^16** needed; the ceremony uses the public Hermez **2^17** powers of tau (sha256-pinned in `ceremony-consent.sh`).

## Roles
- **Coordinator** (you): compiles the circuit, runs the setup, collects contributions in order, applies the beacon, finalizes, publishes the transcript, deploys + wires the verifier.
- **Contributors** (>=3, independent): each adds secret entropy and **destroys it**.
  The more independent contributors, the stronger the guarantee (1-of-N honesty suffices).

## Steps

The phases below are the phases `ceremony-consent.sh` automates for the testnet self-run; the production run inserts one `snarkjs zkey contribute` per independent contributor between setup and beacon (the runbook §3 has the full per-step detail).

### 1. Coordinator - initialize
```bash
cd circuits
npm ci
circom consent.circom --r1cs --wasm --sym -l node_modules/circomlib/circuits -l . -o build
# fetch + sha256-verify the public Hermez ptau exactly as ceremony-consent.sh does (power 17,
# sha256 6b662a32…83ae0), then make contribution #0:
npx snarkjs groth16 setup build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_0000.zkey
```
> Compile only - never `npm run build-consent`.
> That is the DEV single-contributor setup: it generates a local, insecure ptau and **overwrites** the committed `consent_final.zkey` / VK with a forgeable dev key.
> The ceremony needs only the r1cs from compilation.

Publish `build/consent_0000.zkey` and send it to contributor #1.

### 2. Each contributor (in sequence, >=3)
On their OWN machine, having received `consent_{prev}.zkey`:
```bash
npx snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau consent_prev.zkey
npx snarkjs zkey contribute consent_prev.zkey consent_mine.zkey --name="Alice @ OrgA" -v
# adds unpredictable entropy when prompted
```
They publish `consent_mine.zkey` and pass it to the next contributor, then **destroy their entropy** (close the terminal, wipe shell history).
Each contribution is independently verifiable by anyone via the same `snarkjs zkey verify` invocation.

### 3. Coordinator - public random beacon
After the LAST contribution, apply a value that was **unpredictable at contribution time** - e.g. a pre-announced **future Bitcoin block hash** or a **drand** round (the testnet self-run used the drand League-of-Entropy mainnet chain):
```bash
npx snarkjs zkey beacon consent_lastN.zkey build/consent_final.zkey <beaconHex> 10 -n="final beacon: <source>"
```

### 4. Coordinator - finalize
```bash
npx snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_final.zkey
# must print "ZKey Ok!"
npx snarkjs zkey export verificationkey build/consent_final.zkey build/consent_verification_key.json
npx snarkjs zkey export solidityverifier build/consent_final.zkey Groth16Verifier.consent.sol
# rename the emitted contract to `Groth16VerifierConsent` (ceremony-consent.sh step [7/8] does this
# with sed) so it cannot collide with the retired verifier contract name.
shasum -a 256 build/consent_final.zkey     # PIN this sha256
```
Publish the full transcript (every `consent_*.zkey`, contributor names, the beacon value + source) so anyone can reproduce `snarkjs zkey verify`.
**Pin the sha256** - the prover **enforces** it at load (fail-closed on mismatch, audit M4).
The pin lives in the protocol version's `ArtifactDescriptor` (`crates/dogtag-prover-rs/src/artifact.rs`, keyed by the internal protocol version key `dogtag-levelb/1` - an internal identifier, not a product label).
A production key therefore means updating that descriptor's zkey pin (and the VK-json hash it carries) - otherwise the prover FATALs on a hash mismatch.

## Deploy & wire the verifier (on-chain)

The testnet self-run `Groth16VerifierConsent` is the currently wired verifier; the production ceremony output replaces it.
Swapping the registry's ZK verifier is behind a **2-day timelock**: `proposeZkVerifier(addr)` -> wait >= 2 days -> `executeZkVerifier()` (there is no single `setZkVerifier`).

```bash
cp circuits/Groth16Verifier.consent.sol contracts/src/Groth16VerifierConsent.sol
cd contracts && forge build

# 1. deploy the new verifier (deployer = registry DEFAULT_ADMIN)
VERIFIER=$(forge create src/Groth16VerifierConsent.sol:Groth16VerifierConsent \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --json | jq -r .deployedTo)

# 2. propose it (starts the 2-day timer)
cast send <VerificationRegistryConsent> "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy

# 3. AFTER >= 2 days, execute
cast send <VerificationRegistryConsent> "executeZkVerifier()" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy
```
The `VerificationRegistryConsent` address is in `contracts/deployments/roax.json` / `contracts/.env`.
After `executeZkVerifier`, `recordVerificationZK` accepts proofs from the ceremony key.
Re-run the circuit test-suite and fixture generation against the new zkey, update the pinned hashes in `crates/dogtag-prover-rs/src/artifact.rs`, and ship the new `consent_final.zkey` (+ witness graph) to every prover surface - the app bundles and the `/prove-consent` server-prove fallback - so nothing fails closed on the old pin.

## Re-run on circuit changes
Any change to `consent.circom` (constraints) invalidates the key - a NEW ceremony is required, and the new VK is a new protocol version (the frozen VK is what the internal version key pins).
