# DogTag — ZK Trusted-Setup Ceremony (production)

The Groth16 proof-of-verification path needs a circuit-specific **proving/verifying key** produced by a
**multi-party ceremony**. The dev key shipped in `circuits/build` (from `scripts/setup.sh`) is a
single-contributor setup for TESTS ONLY and must NOT secure a real deployment — a sole contributor
could forge attestations. This runbook produces a production-grade key with **≥3 independent
contributors + a public random beacon**, then wires the resulting verifier into the live
`VerificationRegistry` via its 2-day timelock.

> The core three-pillar trust model (integrity + on-chain status + DNS) does **not** depend on this
> ceremony at all, and the **normal/ECDSA** proof-of-verification path already works on-chain today
> (`recordVerification`). The ceremony only gates the **ZK** path (`recordVerificationZK`). Until it
> completes, leave `ZK_VERIFIER = 0x0` on the registry.

Circuit: `DogTagVerification(24,5)`, ~94,459 constraints → **2^17** powers of tau.

## Roles
- **Coordinator** (you): runs `init`, collects contributions in order, applies the beacon, finalizes,
  publishes the transcript, deploys + wires the verifier.
- **Contributors** (≥3, independent): each adds secret entropy and **destroys it**. The more
  independent contributors, the stronger the guarantee (1-of-N honesty suffices).

## Steps

### 1. Coordinator — initialize
```bash
cd circuits
npm run build-circuit          # ensure build/verification.r1cs exists (or just the r1cs)
bash scripts/ceremony.sh init  # downloads Hermez ptau (2^17) + makes contribution #0
```
Publish `build/ceremony_0000.zkey` and send it to contributor #1.

### 2. Each contributor (in sequence, ≥3)
On their OWN machine, having received `ceremony_{prev}.zkey`:
```bash
bash scripts/ceremony.sh contribute ceremony_prev.zkey ceremony_mine.zkey "Alice @ OrgA"
# adds unpredictable entropy when prompted; verifies the chain first
```
They publish `ceremony_mine.zkey` and pass it to the next contributor, then **destroy their entropy**
(close the terminal, wipe shell history). Each contribution is independently verifiable by anyone:
`snarkjs zkey verify build/verification.r1cs ptau/...17.ptau ceremony_mine.zkey`.

### 3. Coordinator — public random beacon
After the LAST contribution, choose a value that was **unpredictable at contribution time** — e.g. a
specific **future Bitcoin block hash** or a **drand** round — and apply it:
```bash
bash scripts/ceremony.sh beacon ceremony_lastN.zkey 0x<beaconHash> "final beacon: BTC block 9xxxxx"
```

### 4. Coordinator — finalize
```bash
bash scripts/ceremony.sh finalize build/ceremony_final.zkey
# -> verifies (ZKey Ok!), exports circuits/Groth16Verifier.sol, copies to build/verification_final.zkey,
#    prints the final zkey sha256 to PIN in CI + the prover image (§11.8(f)).
```
Publish the full transcript (every `ceremony_*.zkey`, contributor names, the beacon value + source) so
anyone can reproduce `snarkjs zkey verify`. Pin the sha256.

## Deploy & wire the verifier (on-chain)

The dev `Groth16Verifier.sol` is already deployed only as part of the local/dev set; for production the
ceremony output replaces it. Swapping the registry's ZK verifier is behind a **2-day timelock**:
`proposeZkVerifier(addr)` → wait ≥ 2 days → `executeZkVerifier()` (there is no single `setZkVerifier`).

```bash
cp circuits/Groth16Verifier.sol contracts/src/Groth16Verifier.sol
cd contracts && forge build

# 1. deploy the new verifier (deployer = registry DEFAULT_ADMIN)
VERIFIER=$(forge create src/Groth16Verifier.sol:Groth16Verifier \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --json | jq -r .deployedTo)

# 2. propose it (starts the 2-day timer)
cast send <VerificationRegistry> "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy

# 3. AFTER >= 2 days, execute
cast send <VerificationRegistry> "executeZkVerifier()" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy
```
The `VerificationRegistry` address is in `contracts/deployments/roax.json`. After `executeZkVerifier`,
`recordVerificationZK` accepts proofs from the ceremony key. Re-run the prover's vectors against the new
zkey hash, and update CI to assert the pinned hash.

## Re-run on circuit changes
Any change to `verification.circom` (constraints) invalidates the key — a NEW ceremony is required.
