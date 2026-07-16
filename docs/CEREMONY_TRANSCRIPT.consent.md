# DogTag Level-B `DogTagConsent` ZK Ceremony — Transcript (TESTNET self-run, M3)

> **Trust caveat (read first).**
> This is a **testnet** phase-2 trusted setup for the Level-B owner-unlinkable consent circuit
> (`circuits/consent.circom` → `DogTagConsent(6)`), redesign milestone **M3**.
> It uses a **single** phase-2 contribution performed on **our own infrastructure**, finalized with a
> **public drand beacon**.
> Because a single party performs the only contribution, that party could in principle have retained
> the toxic waste and **forge consent attestations**.
> It is therefore **NOT production-trustworthy**: it does **not** provide the
> **1-of-N-independent-honest** guarantee a mainnet setup needs.
> It is a **real ceremony process producing a testnet-grade key** — not a security-theatre stand-in —
> and it is the captain-approved M3 scope (`level-b-spec.md` "M3", `data/dogtag-zkverify-z2`), where a
> "standard single-contributor snarkjs phase-2 ceremony is acceptable for now" on ROAX (a testnet).
> Before mainnet, re-run the phase-2 with **≥3 genuinely independent external contributors** (different
> orgs/jurisdictions/hardware) and a pre-announced public beacon — the multi-party
> `circuits/scripts/ceremony.sh` is written for exactly that (see `docs/CEREMONY_RUNBOOK.md`).
>
> **What makes this a REAL ceremony (not the dev throwaway `scripts/setup-consent.sh`):**
> phase-1 is the **public Hermez / Perpetual Powers-of-Tau** output (not a locally generated dev ptau,
> which would let the operator know `tau`), and the final contribution is a **real, publicly verifiable
> drand round** (not a `sha256("…")`-derived pseudo-beacon).
> The remaining gap is purely the **independence of the (single) contributor**, which only the mainnet
> re-run closes.
> This ceremony gates only the Level-B ZK consent path — the three-pillar trust model and the
> ECDSA/normal verification path do not depend on it.

## Circuit

- Source: `circuits/consent.circom` → `DogTagConsent(6)` (depth 6; inclusion paths up to 6 siblings).
- Compiled with **circom 2.1.9** + **circomlib 2.0.5**, verified with **snarkjs 0.7.6** (same toolchain as the v2 verification ceremony).
- **38,501** non-linear constraints, **0 public inputs**, **7 public outputs** → needs `2^16 = 65536` powers of tau (the pow-17 ptau covers it).
- Public-signal vector (declaration order IS load-bearing for M4 calldata; all seven are declared as OUTPUTS):
  `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` — **no `subject`, no `keyHash`** (owner-unlinkable).
- Circuit hash (snarkjs, constant across setup / contribute / beacon / verify):
  `a4f636a8 6aeb60b9 5f187637 c9428ebc c6396c37 2566aada 06979e10 ceb874a0 7330e734 247abd9c 811427ca 6201eb91 c6096652 961ead4b 345b06f9 3d4b95b1`

## Phase-1 powers of tau (public, not self-generated — REUSED, phase-1 was NOT re-run)

- File: `powersOfTau28_hez_final_17.ptau` (`2^17`), size **151,078,040** bytes.
- This is the **same** public Hermez / Perpetual-Powers-of-Tau file the repo's v2 verification ceremony already established trust in (`docs/CEREMONY_TRANSCRIPT.md`); phase-1 is circuit-independent, so it is reused unchanged.
- Source fetched: Polygon's official zkEVM mirror
  `https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_17.ptau`
  (the original Hermez S3 bucket now returns `AccessDenied`; this mirror serves the identical file).
- sha256: `6b662a324867139fb1a20a324d90b6ff61856dfb23f59326909f14b0e2483ae0`
  — **byte-identical** to the value pinned in `docs/CEREMONY_TRANSCRIPT.md` (the ptau the repo already trusts).
- **How phase-1 integrity was established on this run:**
  (1) the file is the **canonical public Hermez/Perpetual-PoT output** fetched from Polygon's official mirror;
  (2) its **sha256 matches the repo's already-trusted pinned value** above (`ceremony-consent.sh` fails closed on any mismatch), so no new trust in a download URL is required;
  (3) the finalize step ran `snarkjs zkey verify consent.r1cs <ptau> consent_final.zkey` and printed **`ZKey Ok!`**, which reads this exact ptau and cryptographically validates the whole phase-2 contribution chain against it — a malformed/truncated ptau would fail there.
  The canonical full check, `snarkjs powersoftau verify` (it replays the entire Hermez contribution chain + beacon and prints `Powers of Tau Ok!`), is single-threaded and slow (~20–30 min on a `2^17` file). It was **launched on this run but did not complete inline** — it saturated this shared host (as it did on the v2 run, `docs/CEREMONY_TRANSCRIPT.md`), so it was stopped. Phase-1 trust here therefore rests on anchors (1)–(3) above (canonical file + sha256 byte-match to the repo's already-trusted ptau + `ZKey Ok!`), exactly as the v2 ceremony did. The full `powersoftau verify` remains the recommended **independent audit** (reproduce section) — an auditor on an idle machine should run it and confirm `Powers of Tau Ok!`.

## Phase-2 contributions (in order — single contributor on our infra, NOT independent)

| # | Name | zkey sha256 | contribution hash (first words) |
|---|------|-------------|---------------------------------|
| 0 | groth16 setup (zero contribution) | (intermediate, not committed) | — |
| 1 | `dogtag-consent-selfrun-testnet-1` (our infra) | (intermediate, not committed) | `18c49eee 2bbe2700 b04dc321 feaf53b7 …` |
| beacon | drand public beacon (round 6286835) | `f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868` (final) | `54cb6539 184740ca b721eb76 192f875d …` |

The single contribution used fresh 64-byte entropy fed to the `snarkjs zkey contribute` prompt (plus the OS RNG snarkjs mixes in); the entropy was **never recorded and is destroyed by design** — only the resulting contribution hash is published above.
The `_0000`/`_0001` intermediate zkeys are **gitignored** (not committed): only the beacon-finalized `consent_final.zkey` is committed.

## Public beacon (drand — League-of-Entropy mainnet chain)

- drand chain hash: `8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce`
- Round: `6286835`
- Randomness (beacon hex, passed to `snarkjs zkey beacon … 10`): `cd2dd8d727d55696dd1358ca6d86aa602d999636f1a90118d9be84cabf7cfa9a`
- Round signature: `ad7bffbc893979160491236df68a84aa093ec297334225cff1f499d8368abb698d96cdee0c30e570ad573b4bf4a70c830e874d61affb19e025dd15eceaff728173f2e979528b7c69dd2ac82e223dafe0d2a3e6d2a92ae54f31ec35b65acc29a7`
- Round canonical time (genesis `1595431050` + `(round-1)·30`): `1784036070` = **2026-07-14T13:34:30Z**.
- Anyone can re-fetch and verify the round:
  `curl https://api.drand.sh/8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce/public/6286835`
- **Honesty note:** in a fully independent ceremony the beacon round is **pre-announced before** contributions so no party can grind against it.
  In this compressed self-run the latest drand round was taken **after** the contribution; the value is a genuine public-randomness beacon, but its unpredictability-binding only fully holds in the independent mainnet re-run.

## Final artifacts (PINNED)

- **Final zkey sha256:** `f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868`
  → committed as `circuits/build/consent_final.zkey` (the M7 prover will load this and pin this hash; the on-chain verifier below is its paired key).
- **Verification key:** `circuits/build/consent_verification_key.json` (sha256 `27879dd7c4eabb6acea4d1be1249ba3c4212f95a27237e7e1e1220557b4e2d7f`; lets anyone run `snarkjs groth16 verify`). `protocol=groth16`, `curve=bn128`, `nPublic=7`.
- **Verifier:** `circuits/Groth16Verifier.consent.sol` (contract **`Groth16VerifierConsent`**, renamed so it does not collide with the live v2 `Groth16Verifier`) → copied to `contracts/src/Groth16VerifierConsent.sol`. Exposes `verifyProof(uint[2] a, uint[2][2] b, uint[2] c, uint[7] pubSignals)`.
- `snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_final.zkey` → **ZKey Ok!**
- This VK/zkey is **NEW** and distinct from the M2 DEV throwaway (dev VK sha `3f79a5ff…`, dev zkey sha `12df8ea4…`) — the dev artifacts are gitignored and must never be deployed.

## Functional validation (the VK actually verifies real proofs)

`node circuits/scripts/test-consent.mjs` against these committed artifacts → **ALL GREEN (33/33)**:

- **witness → groth16 prove → groth16 verify OK**, with all 7 public signals equal to the expected `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` (order + count verified).
- **R-parity**: circuit root `R` == SDK `buildMerkle` root, and prove+verify OK, across tree sizes `{3,4,5,7,10,20}` leaves.
- **negatives** (each FAILS as required): tampered EdDSA `S`; wrong signer; tampered owner-secret (not in tree); inconsistent inclusion path; tampered `R` public signal; **keyPath-substitution** (attribute leaf presented as owner-secret) — the D5 replay-soundness guard.
- **nullifier D5 semantics**: same `(context, nonce)` → identical nullifier (replay rejected); fresh nonce → new nullifier (repeat visit allowed).

## Reproduce / audit

```bash
cd circuits
# 1. r1cs is deterministic from the committed consent.circom (COMPILE ONLY — never run setup-consent.sh /
#    build-consent, which would overwrite the ceremony zkey with a forgeable DEV key):
circom consent.circom --r1cs --wasm --sym -l node_modules/circomlib/circuits -l . -o build
# 2. obtain the SAME public Hermez ptau (any mirror; the sha256 + verify below are the trust anchor):
mkdir -p ptau && curl -L -o ptau/powersOfTau28_hez_final_17.ptau \
  https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_17.ptau
shasum -a 256 ptau/powersOfTau28_hez_final_17.ptau   # expect: 6b662a324867139fb1a20a324d90b6ff61856dfb23f59326909f14b0e2483ae0
snarkjs powersoftau verify ptau/powersOfTau28_hez_final_17.ptau                                  # Powers of Tau Ok!
# 3. the committed final zkey verifies against the circuit + ptau, and re-exports the committed VK:
snarkjs zkey verify build/consent.r1cs ptau/powersOfTau28_hez_final_17.ptau build/consent_final.zkey   # ZKey Ok!
shasum -a 256 build/consent_final.zkey            # expect: f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868
snarkjs zkey export verificationkey build/consent_final.zkey /tmp/vk.json
diff <(jq -S . /tmp/vk.json) <(jq -S . build/consent_verification_key.json)   # identical -> VK is this zkey's
# 4. (functional) prove + verify a real witness + all negatives:
node scripts/test-consent.mjs                     # ALL GREEN (33/33)
```

`snarkjs zkey verify` printing **`ZKey Ok!`** and listing the contribution + beacon (round 6286835) is the end-to-end proof the chain is intact; the `zkey export verificationkey` diff proves the committed `Groth16VerifierConsent.sol` / VK belong to this exact zkey.

## On-chain deployment (ROAX, chainId 135)

M3 deploys the verifier; **wiring it in was M4** — not done in this ceremony, and since **DONE**: the
owner-blind `VerificationRegistryConsent` `0x53F988Ae0124b96069d90CBC78E6245FeB01E125` verifies against
this VK (AGENTS.md "Level-B `VerificationRegistryConsent` (M4)"). The rest of this section is the M3
deploy record as executed.

- **`Groth16VerifierConsent` DEPLOYED at `0x272be146C0aEd6401000E9Aa8241201F6f0fdF1a`** on ROAX (chainId 135),
  ROAX `--legacy`, deployer `0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96`.
  - deploy tx: `0xcd1cd5fa968981c5d18a41e38346622b917f3b2e78bd1e4a1880989e3c0540af` (block **190760**, status success).
  - on-chain `cast code` == the compiled `Groth16VerifierConsent` runtime bytecode (1933 bytes) — byte-identical, so the deployed contract is exactly this VK's verifier.
  - **on-chain functional check:** a real consent proof (built via `test-consent.mjs`'s honest witness) → `verifyProof(a,b,c,pub[7])` returns **`true`**; the same proof with a tampered `pub[4] /*R*/` returns **`false`**. Public signals decoded on-chain match `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
  - This is a **separate** verifier for the Level-B consent circuit; it does **not** replace the live Level-A `Groth16Verifier` `0xEEFCf…` (wiring it into a registry was M4 - since shipped, additively, as `VerificationRegistryConsent`; see the M4 note below).

Deploy command used (reusing the v2 deploy path; forge 1.5.1 needs `--broadcast`):

```bash
cp circuits/Groth16Verifier.consent.sol contracts/src/Groth16VerifierConsent.sol   # staged in this PR
cd contracts && forge build
forge create src/Groth16VerifierConsent.sol:Groth16VerifierConsent \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --broadcast --json
```

- **M4 (out of scope here, since SHIPPED):** M3 does **not** touch the registry. M4 has since added a NEW
  registry alongside the frozen Level-A one - `VerificationRegistryConsent`
  `0x53F988Ae0124b96069d90CBC78E6245FeB01E125` - which does `require(verifyProof(a,b,c,pub[7]))` against
  THIS verifier, asserts `pub[4] /*R*/ == profileRoot(pub[0] /*dogTagId*/)`, enforces `deadline`,
  consumes `pub[3] /*nullifier*/`, and emits an **owner-blind** `Verified` event. It is additive and not
  yet live (the app cutover is M7); see AGENTS.md "Level-B `VerificationRegistryConsent` (M4)".

## ⚠ M3 VK-freeze checkpoint — review conclusion

The M2 handoff flagged one open item to reconsider **before the VK is locked** (`circuits/README.consent.md`, `AGENTS.md`):
the EdDSA message `M = Poseidon5(dogTagId, purpose, relayer, deadline, consentNonce)` shares arity + first slot
with the leaf hash `Poseidon5(DS_LEAF=1, …)` when `dogTagId == 1`.

**Conclusion (freeze as-is):** no exploit exists — EdDSA verification requires the private consent key, and
leaves are never signed, so an `M`/leaf-hash arity coincidence yields nothing forgeable; the public-signal
order/count was re-verified from the freshly compiled circuit (7 outputs, 0 public inputs, `nPublic == 7`);
and the captain-approved spec fixes `M` in this exact form (no domain tag).
Changing `M` would require changing the spec, `consent.circom`, and M7's app proof-gen **together** and re-running
the ceremony — explicitly out of M3 scope.
The VK is therefore **frozen** against `consent.circom` as merged in #42.
