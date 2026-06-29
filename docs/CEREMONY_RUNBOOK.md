# DogTag — Production Trusted-Setup Ceremony Runbook

> **Status: go-live blocker (audit Finding H3).**
> The key currently securing the on-chain ZK path was produced by a **single-operator testnet self-run** (`docs/CEREMONY_TRANSCRIPT.md`): one party performed every contribution and the beacon, so that party could hold the toxic waste and **forge ZK attestations**.
> This runbook is the procedure that replaces it with a real **≥3-independent-contributor** ceremony.
>
> **This document is the runbook the ceremony coordinator (the "captain") fills in and executes.**
> It does **not** generate production keys by itself, and reading or merging it changes nothing on-chain.
> Every fill-in field is marked `<FILL-IN>`.

This runbook drives `circuits/scripts/ceremony.sh` (the real multi-party ceremony), **not** `circuits/scripts/setup.sh` (the single-contributor DEV setup, which is for tests only and must never secure production).

---

## 0. Scope and what stays unaffected

The ceremony **only** gates the **ZK** proof-of-verification path (`VerificationRegistry.recordVerificationZK`).
The core three-pillar trust model (integrity + on-chain status + DNS) and the **normal/ECDSA** path (`recordVerification`) do **not** depend on this ceremony and keep working throughout.

So you can run this ceremony without freezing the product: the only thing it swaps is the Groth16 verifier behind a 2-day timelock.

---

## 1. What it produces + the security model

### 1.1 The two phases and their outputs

A Groth16 trusted setup has two phases. Phase 1 is circuit-independent; phase 2 binds to **this** circuit.

| Phase | What it is | Artifact | Source in this repo |
|---|---|---|---|
| **Phase 1 — Powers of Tau** | Universal, circuit-independent setup over the BN254 curve. | `powersOfTau28_hez_final_17.ptau` | Downloaded by `ceremony.sh init` from the public **Hermez / Perpetual Powers of Tau** ceremony. We do **not** generate our own phase-1 (a self-generated ptau means the operator knows `tau` → can forge). |
| **Phase 2 — circuit-specific** | The multi-party part *you* run, binding the ptau to `DogTagVerification(24,5)`. | `verification_final.zkey` (**proving key**) + the **verification key** baked into `Groth16Verifier.sol` + `verification_key.json` | Produced by `ceremony.sh` `init` → `contribute` (×N) → `beacon` → `finalize`. |

Concretely, a completed ceremony yields these pinned artifacts:

- `circuits/build/verification_final.zkey` — the **proving key** the prover loads (`crates/dogtag-prover-rs` reads `build/verification_final.zkey` and pins its SHA-256 at load, impl §11.8(f)). This is also what `stacks/vet/api` and the parity crates consume.
- `circuits/Groth16Verifier.sol` — the Solidity **verifier**, with the verification key compiled in. Copied to `contracts/src/Groth16Verifier.sol`, deployed, and wired into `VerificationRegistry`.
- `circuits/build/verification_key.json` — the JSON verification key so anyone can run `snarkjs groth16 verify` independently.
- The **final zkey SHA-256** — pinned in CI and the prover image so a swapped key is detected.

The circuit is `DogTagVerification(24,5)`, **94,459** non-linear constraints → needs **2^17 = 131,072** powers of tau (`PTAU_POW=17` in `ceremony.sh`).

### 1.2 Toxic waste — what each contributor MUST destroy

Every contribution injects fresh secret randomness (snarkjs takes it via the interactive entropy prompt and/or `/dev/urandom`). That secret is the **toxic waste**.

- Each `snarkjs zkey contribute` step mixes in a contributor's secret scalar.
- If **any single party** ever learns the *product* of all contributors' secrets (plus the beacon), that party can forge proofs that the verifier accepts — i.e. forge ZK attestations for pets that were never verified.
- Therefore each contributor MUST destroy their entropy the moment their contribution is written: close the terminal, wipe shell history, and ideally power off / wipe the ephemeral machine. The entropy is **never** recorded in the transcript — only the resulting contribution hashes are.

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

**Design goal that follows directly:** maximize the number of **diverse, independent** contributors — different organizations, jurisdictions, hardware, and incentives — so that "all of them colluded" is implausible. Diversity and independence beat reputation. There is no downside to adding more contributors.

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

After (or interleaved with) the anchors, you MAY open a **public contribution round** so anyone can add entropy. Each public participant runs the same `contribute` step on the latest zkey and passes it on. This further strengthens the 1-of-N guarantee at near-zero marginal cost.

The public round is **closed and made unforgeable by a public verifiable beacon**: a value that was *unpredictable at contribution time*, chosen **after** the last contribution — e.g. a specific **future Bitcoin/Ethereum block hash** or a **drand round**. Because the beacon is fixed in advance by reference (e.g. "the hash of BTC block N, which has not yet been mined") but unknowable until it occurs, no contributor (or coordinator) can grind their contribution against it.

### 2.3 Participant slot table (captain fills in)

> The captain replaces every `<FILL-IN>`. Keep ordering explicit — contributions are sequential and the `zkey` is handed off in this exact order. Append rows for any public-round participants.

| # | Role | Name / Org | Category | Jurisdiction | Contact | Machine (air-gapped?) | Scheduled date |
|---|---|---|---|---|---|---|---|
| 0 | Coordinator (setup, zero contribution) | `<FILL-IN: DogTag coordinator>` | Protocol | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| 1 | Contributor | `<FILL-IN>` | Protocol (DogTag) | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 2 | Contributor | `<FILL-IN>` | Government / registry authority | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 3 | Contributor | `<FILL-IN>` | Veterinary association | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 4 | Contributor | `<FILL-IN>` | Security firm / university | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN: yes/no>` | `<FILL-IN>` |
| 5…n | Public round (optional) | `<FILL-IN>` | Public | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| beacon | Coordinator applies public beacon | n/a | Public randomness | n/a | n/a | n/a | `<FILL-IN>` |

**Minimum:** ≥3 genuinely independent contributors (the anchor set gives 4). The beacon is **not** a contributor — it is a separate finalization step that no party controls.

---

## 3. Step-by-step procedure

> **Golden rules.**
> Every contributor works on a **clean, ideally air-gapped machine** dedicated to this ceremony.
> Every contributor **verifies the zkey they receive** before adding to it (`contribute` does this automatically).
> Every contributor **publishes** their output zkey + attestation and then **destroys their entropy**.
> Nothing here generates a production key just by being read — the captain runs these commands with real participants.

### 3.0 Prerequisites (each machine)

- Node.js + the repo's `snarkjs` (resolved by `ceremony.sh` from `node_modules/.bin/snarkjs`).
- The committed circuit, so `build/verification.r1cs` exists:
  ```bash
  cd circuits
  npm ci
  npm run build-circuit     # produces build/verification.r1cs (+ wasm), deterministic from verification.circom
  ```
- A secure file-transfer channel for handing the `zkey` from one contributor to the next (the zkey is public; the channel just needs integrity, not secrecy).

### 3.1 Coordinator — initialize (contribution #0)

```bash
cd circuits
bash scripts/ceremony.sh init
```

This downloads the Hermez phase-1 ptau (`2^17`), runs `snarkjs powersoftau verify` on it, and creates `build/ceremony_0000.zkey` (the zero contribution).

Publish `build/ceremony_0000.zkey` and its SHA-256, then send it to **contributor #1**.

### 3.2 Each contributor — contribute (in sequence, ≥3)

On their **own clean machine**, having received the previous contributor's zkey (e.g. `ceremony_0001.zkey`):

```bash
cd circuits
bash scripts/ceremony.sh contribute ceremony_<prev>.zkey ceremony_<mine>.zkey "<Name @ Org>"
```

`contribute` will:
1. Run `snarkjs zkey verify` on the received zkey (confirms the chain is intact before you add to it).
2. Prompt for **unpredictable entropy** — type a long unpredictable string; snarkjs also mixes OS randomness.
3. Write `ceremony_<mine>.zkey` with your contribution.

Then the contributor:
- **Destroys the toxic waste:** close the terminal, wipe shell history (`history -c` / delete the shell history file), and wipe/poweroff the ephemeral machine if possible.
- **Publishes** `ceremony_<mine>.zkey`, its SHA-256, and their contribution hash (printed by snarkjs) — see §4.
- **Hands off** `ceremony_<mine>.zkey` to the next contributor (or back to the coordinator if last).

Use the participant slot table (§2.3) for the exact ordering and hand-off.

### 3.3 Coordinator — apply the public beacon

After the **last** contribution, choose the beacon value that was unpredictable at contribution time (see §2.2), and only once it is known:

```bash
cd circuits
bash scripts/ceremony.sh beacon ceremony_<lastN>.zkey 0x<beaconHex> "final beacon: <source, e.g. BTC block 9xxxxxx>"
```

This writes `build/ceremony_final.zkey` (`zkey beacon`, 2^10 hash iterations). Record the beacon value **and its public source** so anyone can confirm it later.

### 3.4 Coordinator — finalize

```bash
cd circuits
bash scripts/ceremony.sh finalize build/ceremony_final.zkey
```

`finalize` will:
1. `snarkjs zkey verify` the final zkey against the r1cs + ptau (must print **`ZKey Ok!`**).
2. Export `circuits/Groth16Verifier.sol` (verifier with the vkey compiled in).
3. Export `circuits/build/verification_key.json` (for independent `snarkjs groth16 verify`).
4. Copy the final zkey to `circuits/build/verification_final.zkey` (the name the prover loads).
5. Print the **final zkey SHA-256** — **PIN THIS** in CI and the prover image (§11.8(f)).

Then publish the full transcript (§4) and proceed to deployment (§5).

---

## 4. Attestation + transcript

The ceremony's auditability rests on a **public hash chain**: each contribution's hash commits to the previous one, so anyone can replay the whole chain and confirm no step was tampered with or skipped.

### 4.1 What each contributor publishes (attestation)

Each contributor publishes a short signed attestation. Recommended format:

```
DogTag ZK Ceremony — Contribution Attestation
  Contributor:        <Name @ Org>
  Position in chain:  #<k>
  Input zkey sha256:  <sha256 of the zkey I received>
  Output zkey sha256: <sha256 of the zkey I produced>     # shasum -a 256 ceremony_<mine>.zkey
  Contribution hash:  <the "Contribution Hash" snarkjs printed during contribute>
  Machine:            <clean/air-gapped machine description>
  Entropy:            fresh, unpredictable, and DESTROYED (not recorded)
  Date (UTC):         <FILL-IN>
  Signature:          <PGP / signed message over the above>
```

The contribution hash and the input/output SHA-256 are the load-bearing fields: they pin the contributor's exact position in the chain.

### 4.2 What the coordinator publishes (transcript)

Append the production ceremony to a transcript table (mirroring `docs/CEREMONY_TRANSCRIPT.md`, but marked **PRODUCTION**), with one row per step:

| # | Name | zkey sha256 | contribution hash (first words) | attestation link |
|---|---|---|---|---|
| 0 | groth16 setup (zero) | `<FILL-IN>` | — | n/a |
| 1 | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` | `<FILL-IN>` |
| … | … | … | … | … |
| beacon | final beacon | `<FILL-IN>` | `<FILL-IN>` | beacon source + value |

Also record: circuit hash, ptau file (Hermez `powersOfTau28_hez_final_17.ptau`), beacon generator hex + iterations + **public source**, and the final pinned artifacts (zkey sha256, verifier bytes).

### 4.3 How anyone verifies the full transcript

```bash
cd circuits
# 1. Reproduce the r1cs deterministically from the committed circuit:
npm run build-circuit

# 2. Verify the final zkey against circuit + ptau (replays the whole contribution chain):
node_modules/.bin/snarkjs zkey verify build/verification.r1cs ptau/powersOfTau28_hez_final_17.ptau build/verification_final.zkey
#    expect: "ZKey Ok!" — and the printed contribution hashes must match the transcript, in order.

# 3. Confirm the pinned hash:
shasum -a 256 build/verification_final.zkey      # must equal the transcript's final zkey sha256

# 4. (optional) Verify each intermediate contribution independently:
node_modules/.bin/snarkjs zkey verify build/verification.r1cs ptau/powersOfTau28_hez_final_17.ptau ceremony_<k>.zkey

# 5. Verify a proof with the exported vkey (proves the verifier matches the key):
node_modules/.bin/snarkjs groth16 verify build/verification_key.json <public.json> <proof.json>
```

`snarkjs zkey verify` printing **`ZKey Ok!`** and listing the contribution hashes **in the same order** as the transcript is the end-to-end proof that the chain is intact. Confirm the beacon line shows your chosen generator + iterations.

---

## 5. Deployment hand-off (swap the on-chain verifier)

The dev/testnet verifier is currently live. Swapping in the production verifier is behind a **2-day timelock** on `VerificationRegistry`: there is **no** single-call `setZkVerifier` — you `propose`, wait, then `execute`.

> **Where things stand now (M2 scout confirmed):** the live `VerificationRegistry` (`0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1`) already points at the deployed verifier `0x138b433071Ad806E841B5AD53623290a9bf21761`. That verifier is the **testnet self-run** key. This step replaces it with the production-ceremony verifier.

### 5.1 Build + deploy the production verifier

```bash
cp circuits/Groth16Verifier.sol contracts/src/Groth16Verifier.sol
cd contracts && forge build

# deploy (deployer must hold DEFAULT_ADMIN_ROLE on the registry)
VERIFIER=$(forge create src/Groth16Verifier.sol:Groth16Verifier \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --json | jq -r .deployedTo)
echo "new verifier: $VERIFIER"
```

### 5.2 Propose → timelock → execute

```bash
REG=0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1   # VerificationRegistry (contracts/deployments/roax.json)

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
#   expect: $VERIFIER  (NOT the old 0x138b4330… testnet verifier)
```

Then:
- Update `contracts/deployments/roax.json`: set `Groth16Verifier` to `$VERIFIER` and replace the `_zk_ceremony` note (drop "testnet self-run") with the production transcript reference + final zkey sha256.
- Ship `circuits/build/verification_final.zkey` into the prover image and **pin its sha256 in CI** so a mismatched key fails the build.
- Re-run the prover's proof vectors against the new zkey and confirm `recordVerificationZK` accepts a freshly generated proof on-chain.

### 5.4 Re-run condition

Any change to `verification.circom` (constraints) **invalidates the key** and requires a brand-new ceremony from §3.1.

---

## Appendix A — `ceremony.sh` end-to-end check

`circuits/scripts/ceremony.sh` implements the full procedure: `init` (Hermez ptau download + verify + zero contribution), `contribute` (verify-then-contribute, repeatable per contributor), `beacon` (public beacon), `finalize` (verify + export verifier + export vkey + copy pinned zkey + print sha256). It uses the **public Hermez** phase-1 ptau (not a self-generated one) and ends in a public beacon, matching the 1-of-N security model.

**Gap found and fixed in this change:** `finalize` previously exported only `Groth16Verifier.sol` and never wrote `verification_key.json`, so the transcript's documented `snarkjs groth16 verify` audit step had no vkey to run against. `finalize` now also runs `snarkjs zkey export verificationkey` (matching the dev `setup.sh`) and its closing pointer now references this runbook (§5) instead of the older `docs/CEREMONY.md`.

**Documented, not auto-scripted (by design):** per-contributor attestation publishing (§4.1) is a human step — `ceremony.sh` prints the contribution hash but does not produce the signed attestation file, since signing keys and identities live with the contributors, not the script.
