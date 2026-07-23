# DogTag - Production Trusted-Setup Ceremony Runbook

> **Live circuit: `circuits/consent.circom` (`DogTagConsent(6)`) - the owner-hidden consent circuit.**
> This runbook produces the production proving/verifying key for the ONE circuit the protocol runs on.
>
> **History:** the first ceremony in this repo was run for the since-retired owner-revealing `verification.circom`; its transcript ([`CEREMONY_TRANSCRIPT.md`](./CEREMONY_TRANSCRIPT.md)) stays frozen as provenance, and its tooling (`circuits/scripts/ceremony.sh`, `circuits/scripts/setup.sh`, `npm run compile-circuit` / `npm run build-circuit`) was removed with the owner-revealing layer.

> **Status: go-live blocker (audit Finding H3, unchanged in substance).**
> The key currently securing the on-chain ZK consent path was produced by a **single-operator testnet self-run** (`circuits/scripts/ceremony-consent.sh`, transcript [`CEREMONY_TRANSCRIPT.consent.md`](./CEREMONY_TRANSCRIPT.consent.md)): one party performed the only contribution and applied the beacon, so that party could hold the toxic waste and **forge consent attestations**.
> H3 was originally raised against the identical self-run pattern on the retired circuit; it applies to the consent key without modification.
> This runbook is the procedure that replaces the self-run key with a real **>=3-independent-contributor** ceremony.
>
> **This document is the runbook the ceremony coordinator (the "captain") fills in and executes.**
> It does **not** generate production keys by itself, and reading or merging it changes nothing on-chain.
> Every fill-in field is marked `<FILL-IN>`.

This runbook drives the phases automated by `circuits/scripts/ceremony-consent.sh` (public pinned ptau + phase-2 contributions + public beacon), **not** `circuits/scripts/setup-consent.sh` (the single-contributor DEV setup, which is for tests only, must never secure production, and **overwrites the committed ceremony key if run**).

---

## 0. Scope and what stays unaffected

The ceremony **only** gates the Groth16 verifier behind the **ZK consent** path (`VerificationRegistryConsent.recordVerificationZK`) - which is the sole on-chain verification path, so this key is the key that secures verification.
The Merkle-proof side of the protocol - issuance anchoring (`issue(R)`), integrity, selective disclosure, revocation - does **not** depend on this ceremony and keeps working throughout.

You can still run this ceremony without freezing the product: the only on-chain change it produces is a verifier swap behind a 2-day timelock, and the incumbent key keeps verifying until `executeZkVerifier()` lands.

---

## 1. What it produces + the security model

### 1.1 The two phases and their outputs

A Groth16 trusted setup has two phases. Phase 1 is circuit-independent; phase 2 binds to **this** circuit.

| Phase | What it is | Artifact | Source in this repo |
|---|---|---|---|
| **Phase 1 - Powers of Tau** | Universal, circuit-independent setup over the BN254 curve. | `powersOfTau28_hez_final_17.ptau` | Downloaded + sha256-pinned by `ceremony-consent.sh` (the public **Hermez / Perpetual Powers of Tau** file, served from the Polygon zkEVM mirror - the URL is not the trust anchor, the pinned sha256 + `zkey verify` are). We do **not** generate our own phase-1 (a self-generated ptau means the operator knows `tau` -> can forge). |
| **Phase 2 - circuit-specific** | The multi-party part *you* run, binding the ptau to `DogTagConsent(6)`. | `consent_final.zkey` (**proving key**) + the **verification key** baked into `Groth16VerifierConsent.sol` + `consent_verification_key.json` | Produced by the setup -> contribute (xN) -> beacon -> finalize sequence in §3. |

Concretely, a completed ceremony yields these pinned artifacts:

- `circuits/build/consent_final.zkey` - the **proving key** every prover loads: the mobile app bundles (with the witness graph `consent.graph`) and the `/prove-consent` server-prove fallback. `crates/dogtag-prover-rs` pins its SHA-256 in the version's `ArtifactDescriptor` (`src/artifact.rs`) and fails closed on a mismatch at load.
- `circuits/Groth16Verifier.consent.sol` - the Solidity **verifier** (contract `Groth16VerifierConsent`), with the verification key compiled in. Copied to `contracts/src/Groth16VerifierConsent.sol`, deployed, and wired into `VerificationRegistryConsent`.
- `circuits/build/consent_verification_key.json` - the JSON verification key so anyone can run `snarkjs groth16 verify` independently.
- The **final zkey SHA-256** - pinned in the prover's `ArtifactDescriptor` and CI so a swapped key is detected.

The circuit is `DogTagConsent(6)`, **38,501** non-linear constraints -> needs **2^16 = 65,536** powers of tau; the ceremony uses the public **2^17** Hermez ptau (`POWER=17` in `ceremony-consent.sh`), which covers it.

### 1.2 Toxic waste - what each contributor MUST destroy

Every contribution injects fresh secret randomness (snarkjs takes it via the interactive entropy prompt and/or `/dev/urandom`). That secret is the **toxic waste**.

- Each `snarkjs zkey contribute` step mixes in a contributor's secret scalar.
- If **any single party** ever learns the *product* of all contributors' secrets (plus the beacon), that party can forge proofs that the verifier accepts - i.e. forge consent attestations for owners who never consented.
- Therefore each contributor MUST destroy their entropy the moment their contribution is written: close the terminal, wipe shell history, and ideally power off / wipe the ephemeral machine. The entropy is **never** recorded in the transcript - only the resulting contribution hashes are.

### 1.3 The trust model: 1-of-N honest (NOT a majority / multisig)

> **This is the single most important and most misunderstood point. Read it carefully.**

The ceremony is **secure if AT LEAST ONE contributor honestly destroys their toxic waste.**
It is broken **only if EVERY contributor colludes** (or is compromised) and pools their secrets.

This is the opposite of a multisig / majority-honest model:

| Misconception (WRONG) | Reality (CORRECT) |
|---|---|
| "We need a majority of honest contributors." | We need **one**. The setup is sound as long as a *single* link in the chain destroyed its secret. |
| "More contributors = more ways to break it." | More contributors = **more independent chances** that at least one was honest. Adding contributors can only *help*. |
| "It's like a 3-of-5 multisig / threshold." | It is **not** a threshold scheme. There is no quorum, no signing, no on-chain weights. It's a sequential hash-chain where every honest link protects everyone. |
| "We should pick a few highly-trusted parties." | Trust concentration is the risk. Prefer **many, diverse, mutually-independent** parties who are unlikely to all collude. |

**Design goal that follows directly:** maximize the number of **diverse, independent** contributors - different organizations, jurisdictions, hardware, and incentives - so that "all of them colluded" is implausible.
Diversity and independence beat reputation.
There is no downside to adding more contributors.

---

## 2. Participant structure

### 2.1 Recommended anchor set

Pick contributors who would not plausibly all collude. The recommended **diverse anchor set** spans four independent categories:

1. **DogTag (the protocol team).** Establishes the chain and finalizes; one ordinary contribution like any other.
2. **A government / national pet-registry authority.** A different jurisdiction and incentive structure from the protocol team.
3. **A veterinary association.** A domain stakeholder independent of both the protocol and the state.
4. **An independent security firm or university.** A neutral technical party whose reputation depends on doing this correctly.

These four already give a strong 1-of-N guarantee because their incentives and infrastructure are uncorrelated.

### 2.2 Optional open public contribution round

After (or interleaved with) the anchors, you MAY open a **public contribution round** so anyone can add entropy.
Each public participant runs the same `contribute` step on the latest zkey and passes it on.
This further strengthens the 1-of-N guarantee at near-zero marginal cost.

The public round is **closed and made unforgeable by a public verifiable beacon**: a value that was *unpredictable at contribution time*, chosen **after** the last contribution - e.g. a specific **future Bitcoin/Ethereum block hash** or a **drand round** (the testnet self-run used the drand League-of-Entropy mainnet chain).
Because the beacon is fixed in advance by reference (e.g. "the hash of BTC block N, which has not yet been mined") but unknowable until it occurs, no contributor (or coordinator) can grind their contribution against it.

### 2.3 Participant slot table (captain fills in)

> The captain replaces every `<FILL-IN>`. Keep ordering explicit - contributions are sequential and the `zkey` is handed off in this exact order. Append rows for any public-round participants.

| # | Role | Name / Org | Category | Jurisdiction | Contact | Machine (air-gapped?) | Scheduled date |
|---|---|---|---|---|---|---|---|
| 0 | Coordinator (setup, zero contribution) | `<FILL-IN: DogTag coordinator>` | Protocol | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| 1 | Contributor | `<FILL-IN>` | Protocol (DogTag) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 2 | Contributor | `<FILL-IN>` | Government / registry authority | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 3 | Contributor | `<FILL-IN>` | Veterinary association | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 4 | Contributor | `<FILL-IN>` | Security firm / university | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 5…n | Public round (optional) | `<FILL-IN>` | Public | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| beacon | Coordinator applies public beacon | n/a | Public randomness | n/a | n/a | n/a | `<FILL-IN>` |

**Minimum:** >=3 genuinely independent contributors (the anchor set gives 4). The beacon is **not** a contributor - it is a separate finalization step that no party controls.

---

## 3. Step-by-step procedure

> **Golden rules.**
> Every contributor works on a **clean, ideally air-gapped machine** dedicated to this ceremony.
> Every contributor **verifies the zkey they receive** before adding to it.
> Every contributor **publishes** their output zkey + attestation and then **destroys their entropy**.
> Nothing here generates a production key just by being read - the captain runs these commands with real participants.
>
> **Tooling note.** The committed `ceremony-consent.sh` automates the single-contributor testnet self-run end-to-end.
> The production run uses the same phases with the contribution step repeated per contributor, so the commands below are the underlying `circom`/`snarkjs` invocations (identical to what the script runs), not a new tool.

### 3.0 Prerequisites (each machine)

- Node.js + the repo's `snarkjs` (`cd circuits && npm ci`; the script resolves `node_modules/.bin/snarkjs`).
- The committed circuit, compiled so `build/consent.r1cs` exists (deterministic from `consent.circom`):
  ```bash
  cd circuits
  npm ci
  circom consent.circom --r1cs --wasm --sym -l node_modules/circomlib/circuits -l . -o build
  ```
  > **Do NOT run `npm run build-consent` here.** That script (`scripts/setup-consent.sh`) is the DEV single-contributor setup: it generates a *local, insecure* ptau and **overwrites** the committed `consent_final.zkey` / `consent_verification_key.json` with a forgeable dev key. The ceremony needs only the r1cs from compilation.
- The **same pinned phase-1 ptau** every other machine uses, at `circuits/ptau/powersOfTau28_hez_final_17.ptau`. The `zkey verify` steps read it, so the ptau must be present on **every** contributor/verifier machine, not just the coordinator. Obtain it (download or receive it alongside the zkey) and confirm it matches the pin before contributing:
  ```bash
  cd circuits && mkdir -p ptau
  curl -L --fail -o ptau/powersOfTau28_hez_final_17.ptau \
    https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_17.ptau
  shasum -a 256 ptau/powersOfTau28_hez_final_17.ptau
  # must print 6b662a324867139fb1a20a324d90b6ff61856dfb23f59326909f14b0e2483ae0
  # (the sha256 pinned in ceremony-consent.sh; the original Hermez S3 bucket denies access, the
  #  Polygon zkEVM mirror serves the byte-identical file - the pin, not the URL, is the trust anchor)
  ```
- A secure file-transfer channel for handing the `zkey` from one contributor to the next (the zkey is public; the channel just needs integrity, not secrecy).

### 3.1 Coordinator - initialize (contribution #0)

```bash
cd circuits
npx snarkjs powersoftau verify ptau/powersOfTau28_hez_final_17.ptau   # slow; independent phase-1 audit
npx snarkjs groth16 setup build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_0000.zkey
```

This creates `build/consent_0000.zkey` (the zero contribution).

Publish `build/consent_0000.zkey` and its SHA-256, then send it to **contributor #1**.

### 3.2 Each contributor - contribute (in sequence, >=3)

On their **own clean machine**, having received the previous contributor's zkey (e.g. `consent_0001.zkey`):

```bash
cd circuits
npx snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau consent_<prev>.zkey
npx snarkjs zkey contribute consent_<prev>.zkey consent_<mine>.zkey --name="<Name @ Org>" -v
```

The two commands:
1. `zkey verify` confirms the chain is intact before you add to it.
2. `zkey contribute` prompts for **unpredictable entropy** - type a long unpredictable string; snarkjs also mixes OS randomness - and writes `consent_<mine>.zkey` with your contribution.

Then the contributor:
- **Destroys the toxic waste:** close the terminal, wipe shell history (`history -c` / delete the shell history file), and wipe/poweroff the ephemeral machine if possible.
- **Publishes** `consent_<mine>.zkey`, its SHA-256, and their contribution hash (printed by snarkjs) - see §4.
- **Hands off** `consent_<mine>.zkey` to the next contributor (or back to the coordinator if last).

Use the participant slot table (§2.3) for the exact ordering and hand-off.

### 3.3 Coordinator - apply the public beacon

After the **last** contribution, choose the beacon value that was unpredictable at contribution time (see §2.2), and only once it is known:

```bash
cd circuits
npx snarkjs zkey beacon consent_<lastN>.zkey build/consent_final.zkey <beaconHex> 10 \
  -n="final beacon: <source, e.g. drand round NNNN / BTC block 9xxxxxx>"
```

This writes `build/consent_final.zkey` (`zkey beacon`, 2^10 hash iterations). Record the beacon value **and its public source** so anyone can confirm it later.

### 3.4 Coordinator - finalize

```bash
cd circuits
npx snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_final.zkey
# 1. must print "ZKey Ok!" (replays and validates the whole contribution chain)
npx snarkjs zkey export verificationkey build/consent_final.zkey build/consent_verification_key.json
# 2. the JSON VK for independent `snarkjs groth16 verify`
npx snarkjs zkey export solidityverifier build/consent_final.zkey Groth16Verifier.consent.sol
# 3. then rename the emitted contract to `Groth16VerifierConsent` (ceremony-consent.sh step [7/8]
#    does this with sed) so it cannot collide with the retired verifier's contract name
shasum -a 256 build/consent_final.zkey
# 4. the final zkey SHA-256 - PIN THIS in the prover ArtifactDescriptor + CI
```

Then publish the full transcript (§4) and proceed to deployment (§5).

---

## 4. Attestation + transcript

The ceremony's auditability rests on a **public hash chain**: each contribution's hash commits to the previous one, so anyone can replay the whole chain and confirm no step was tampered with or skipped.

### 4.1 What each contributor publishes (attestation)

Each contributor publishes a short signed attestation. Recommended format:

```
DogTag ZK Ceremony - Contribution Attestation
  Contributor:        <Name @ Org>
  Position in chain:  #<k>
  Input zkey sha256:  <sha256 of the zkey I received>
  Output zkey sha256: <sha256 of the zkey I produced>     # shasum -a 256 consent_<mine>.zkey
  Contribution hash:  <the "Contribution Hash" snarkjs printed during contribute>
  Machine:            <clean/air-gapped machine description>
  Entropy:            fresh, unpredictable, and DESTROYED (not recorded)
  Date (UTC):         <FILL-IN>
  Signature:          <PGP / signed message over the above>
```

The contribution hash and the input/output SHA-256 are the load-bearing fields: they pin the contributor's exact position in the chain.

### 4.2 What the coordinator publishes (transcript)

Append the production ceremony to a transcript table (mirroring `docs/CEREMONY_TRANSCRIPT.consent.md`, but marked **PRODUCTION**), with one row per step:

| # | Name | zkey sha256 | contribution hash (first words) | attestation link |
|---|---|---|---|---|
| 0 | groth16 setup (zero) | `<FILL-IN>` | - | n/a |
| 1 | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| … | … | … | … | … |
| beacon | final beacon | `<FILL-IN>` | `<FILL-IN>` | beacon source + value |

Also record: circuit hash, ptau file (Hermez `powersOfTau28_hez_final_17.ptau` + its sha256), beacon generator hex + iterations + **public source**, and the final pinned artifacts (zkey sha256, VK-json sha256, verifier bytes).

### 4.3 How anyone verifies the full transcript

```bash
cd circuits
# 1. Reproduce the r1cs deterministically from the committed circuit (COMPILE ONLY - never
#    `npm run build-consent`, which would overwrite the verifier/zkey with a forgeable dev key):
npm ci
circom consent.circom --r1cs --wasm --sym -l node_modules/circomlib/circuits -l . -o build

# 2. Verify the final zkey against circuit + ptau (replays the whole contribution chain):
node_modules/.bin/snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_final.zkey
#    expect: "ZKey Ok!" - and the printed contribution hashes must match the transcript, in order.

# 3. Confirm the pinned hash:
shasum -a 256 build/consent_final.zkey      # must equal the transcript's final zkey sha256

# 4. (optional) Verify each intermediate contribution independently:
node_modules/.bin/snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau consent_<k>.zkey

# 5. Verify a proof with the exported vkey (proves the verifier matches the key):
node_modules/.bin/snarkjs groth16 verify build/consent_verification_key.json <public.json> <proof.json>
```

`snarkjs zkey verify` printing **`ZKey Ok!`** and listing the contribution hashes **in the same order** as the transcript is the end-to-end proof that the chain is intact. Confirm the beacon line shows your chosen generator + iterations.

---

## 5. Deployment hand-off (swap the on-chain verifier)

The testnet self-run verifier is currently live. Swapping in the production verifier is behind a **2-day timelock** on `VerificationRegistryConsent`: there is **no** single-call `setZkVerifier` - you `propose`, wait, then `execute`.

> **Where things stand now:** the live `VerificationRegistryConsent` points at the `Groth16VerifierConsent` produced by the single-operator testnet self-run (`docs/CEREMONY_TRANSCRIPT.consent.md`).
> The ROAX testnet is disposable and is wiped + redeployed fresh, so take BOTH addresses (registry and verifier) from `contracts/deployments/roax.json` / `contracts/.env` at execution time - never from a doc.
> This step replaces the self-run verifier with the production-ceremony verifier.

### 5.1 Build + deploy the production verifier

```bash
cp circuits/Groth16Verifier.consent.sol contracts/src/Groth16VerifierConsent.sol
cd contracts && forge build

# deploy (deployer must hold DEFAULT_ADMIN_ROLE on the registry)
VERIFIER=$(forge create src/Groth16VerifierConsent.sol:Groth16VerifierConsent \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --json | jq -r .deployedTo)
echo "new verifier: $VERIFIER"
```

### 5.2 Propose -> timelock -> execute

```bash
REG=$(jq -r .VerificationRegistryConsent contracts/deployments/roax.json)   # the live registry

# 1. propose (starts the 2-day ZK_TIMELOCK; emits ZkVerifierProposed(v, eta))
cast send "$REG" "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy

# 2. WAIT >= 2 days (ZK_TIMELOCK = 2 days). Confirm the pending proposal + eta:
cast call "$REG" "pendingZkVerifier()(address)" --rpc-url "$ROAX_RPC"
cast call "$REG" "zkVerifierEta()(uint256)"     --rpc-url "$ROAX_RPC"

# 3. AFTER the eta, execute (emits ZkVerifierUpdated(verifier))
cast send "$REG" "executeZkVerifier()" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy
```

### 5.3 Confirm the swap

```bash
# the live registry must now report the NEW production verifier:
cast call "$REG" "zkVerifier()(address)" --rpc-url "$ROAX_RPC"
#   expect: $VERIFIER  (NOT the testnet self-run verifier)
```

Then:
- Update `contracts/deployments/roax.json`: set the verifier entry to `$VERIFIER` and replace the ceremony note (drop "testnet self-run") with the production transcript reference + final zkey sha256.
- Ship `circuits/build/consent_final.zkey` (+ the witness graph) into every prover surface - the mobile app bundles and the `/prove-consent` server-prove fallback - and update the pinned hashes (zkey + VK json) in the version's `ArtifactDescriptor` (`crates/dogtag-prover-rs/src/artifact.rs`) so a mismatched key fails the build instead of proving.
- Re-run the circuit suite against the new key (`npm run test-consent`, `npm run gen-consent-fixture`) and confirm `recordVerificationZK` accepts a freshly generated proof on-chain (`scripts/e2e-zk.sh` drives that end-to-end).

### 5.4 Re-run condition

Any change to `consent.circom` (constraints) **invalidates the key** and requires a brand-new ceremony from §3.1.
A new VK is also a new protocol version: the frozen VK is what the internal protocol version key pins, so a re-ceremony lands together with a version bump, never silently under the same version.

---

## Appendix A - `ceremony-consent.sh` end-to-end check

`circuits/scripts/ceremony-consent.sh` implements the full phase sequence in one run: compile (deterministic r1cs), pinned public Hermez ptau download + sha256 check (optional full `powersoftau verify`), `groth16 setup` (zero contribution), one phase-2 contribution, a public **drand** beacon, `zkey verify` (must print `ZKey Ok!`), VK + Solidity-verifier export (renamed `Groth16VerifierConsent`), and a printed summary (ptau sha256, drand chain/round/randomness/signature, final zkey + VK sha256s) for the transcript.
It uses the **public Hermez** phase-1 ptau (not a self-generated one) and ends in a public beacon, matching the 1-of-N security model.
As committed it performs exactly ONE contribution - the testnet self-run - which is why its output is not production-grade; the production run repeats the §3.2 contribute step per independent contributor between setup and beacon.

**Documented, not auto-scripted (by design):** per-contributor attestation publishing (§4.1) is a human step - the tooling prints the contribution hash but does not produce the signed attestation file, since signing keys and identities live with the contributors, not the script.
