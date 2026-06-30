# DogTag ZK Ceremony — Transcript (TESTNET self-run, v2)

> **Trust caveat (read first).** This is a **testnet** phase-2 trusted setup. All three phase-2
> contributions **and** the beacon application were performed on **our own infrastructure** (the
> captain's infra), authorized for testnet only. It is therefore **NOT production-trustworthy**: because
> no contributor is independent of us, a single party (us) could in principle have retained the toxic
> waste and **forge ZK attestations**. It is a **real ceremony process producing a testnet-grade key** —
> not a security theatre stand-in — but it does **not** yet provide the **1-of-N-independent-honest**
> guarantee a mainnet setup needs. Before mainnet, re-run `circuits/scripts/ceremony.sh` with **≥3
> genuinely independent external contributors** (different orgs/jurisdictions/hardware) and a
> pre-announced public beacon (see `docs/CEREMONY_RUNBOOK.md`). The core three-pillar trust model and the
> normal/ECDSA verification path do **not** depend on this ceremony.
>
> **What this v2 run improved over the prior testnet self-run:** phase-1 is now the **public Hermez /
> Perpetual Powers-of-Tau** output (not a locally-generated dev ptau, which would let the operator know
> `tau`), and the beacon is a **real, publicly verifiable drand round** (not a `sha256("…")`-derived
> pseudo-beacon). Both are strict upgrades; the remaining gap is purely the **independence of the
> contributors**, which only the mainnet re-run closes.

## Circuit
- `circuits/verification.circom` → `DogTagVerification(24, 5)`
- Constraints: **94,459** (labels 171,389) → powers of tau **2^17**
- Circuit hash (snarkjs, constant across all steps):
  `2db9b8f7 a2b90af9 397203dc 07e1d450 17557327 f084e11e ec9e6db2 b158acf7 2d5d66d4 4748cee4 fdfc21cd 8956b4e6 a2d4b839 ebfd4272 cd41b085 bfb6fe9c`
- Phase-1 ptau: **public Hermez `powersOfTau28_hez_final_17.ptau`** (the Perpetual Powers-of-Tau output).

## Phase-1 powers of tau (public, not self-generated)
- File: `powersOfTau28_hez_final_17.ptau` (2^17), size **151,078,040** bytes.
- Source fetched: Polygon's official zkEVM mirror
  `https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_17.ptau`
  (the original Hermez S3 bucket now returns `AccessDenied`; this mirror serves the identical file).
- sha256: `6b662a324867139fb1a20a324d90b6ff61856dfb23f59326909f14b0e2483ae0`
- **How phase-1 integrity was established on this run (be honest):** the canonical cryptographic check is
  `snarkjs powersoftau verify` (it replays the entire Hermez contribution chain + beacon and prints
  `Powers of Tau Ok!`). On a 2^17 file it is single-threaded and very slow (20-30 min) and on this run it
  **saturated a shared host** (load spiked > 300), so per the coordinator it was **NOT completed inline**
  — `ceremony.sh init` was run with `CEREMONY_SKIP_PTAU_VERIFY=1`. Phase-1 integrity here rests on three
  things instead: (1) the file is the **canonical public Hermez/Perpetual-PoT output** fetched from
  Polygon's official zkEVM mirror; (2) its **sha256 is pinned above** (transit integrity); and (3) the
  finalize step ran `snarkjs zkey verify … <ptau> verification_final.zkey` and printed **`ZKey Ok!`**,
  which reads this exact ptau and cryptographically validates the whole phase-2 contribution chain against
  it — a malformed/truncated ptau would fail there. The full `powersoftau verify` remains the recommended
  **independent audit step** and is in the reproduce section below; an auditor on an idle machine should
  run it and confirm `Powers of Tau Ok!`.

## Phase-2 contributions (in order — all on captain infra, NOT independent)
| # | Name | zkey sha256 | contribution hash (first words) |
|---|---|---|---|
| 0 | groth16 setup (zero contribution) | `ad26dbd035dc793f4154178990493ccd2779fe8a2b0d52406d06f373f79abbb6` | — |
| 1 | dogtag-selfrun-testnet-1 (captain infra) | `aa2ca080e916e909066d9b7e695408c6de5dfab6d5386dfcd2cda39e321d4ef4` | `37690937 54c169f7 d6797710 9a3a7299 …` |
| 2 | dogtag-selfrun-testnet-2 (captain infra) | `d448ee32df1eea767cc5e948d368a299c271336487050b582eb307219a45a069` | `27258339 c437bcdd ea31fbbe ea14c855 …` |
| 3 | dogtag-selfrun-testnet-3 (captain infra) | `f34393b59b95e9fc2b91a2b20437a94a82bbb94f4b70b104d06a8218edd020f1` | `4f25b6fb 5a4dd5fb 52ddb95c 1359a95b …` |
| beacon | drand public beacon | `9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992` | `f96a8988 4815b097 1b3caaa8 25254360 …` |

Each contribution used fresh 64-byte entropy fed to the `snarkjs zkey contribute` prompt (plus the OS
RNG snarkjs mixes in); the entropy was **never recorded and is destroyed by design** — only the resulting
contribution hashes are published above.

## Public beacon (drand — League of Entropy mainnet)
- drand chain hash: `8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce`
- Round: `6245947`
- Randomness (beacon hex, passed to `snarkjs zkey beacon … 10`): `f3e899223b769606e838a6f4c1bbb8f33d877d8969702cbe819c957d5d96dfc1`
- Round signature: `8ad096e63f5b910938255c30a2d82843f57f97903233967c545da7dc66c933e940a340802a41406e75e05f892009a49014ae98c07e53de88214265cc8125b7e3744549fd50262ae1cb9031fde439ddf5c456a8c9ea72e1f526288b37aa124c98`
- Fetched (UTC): `2026-06-30T08:51:00Z`
- Anyone can re-fetch and verify the round:
  `curl https://api.drand.sh/8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce/public/6245947`
- **Honesty note:** in a fully independent ceremony the beacon round is **pre-announced before**
  contributions so no party can grind against it. In this compressed self-run the latest drand round was
  taken **after** the last contribution; the value is a genuine public-randomness beacon, but its
  unpredictability-binding only fully holds in the independent mainnet re-run.

## Final artifacts (PINNED)
- **Final zkey sha256:** `9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992`
  → committed as `circuits/build/verification_final.zkey` (the prover `dogtag-prover-rs` loads this and
  pins this hash in `EXPECTED_ZKEY_SHA256_HEX`; re-vendored into both mobile apps + the prover-service).
- **Verifier:** `circuits/Groth16Verifier.sol` (9390 bytes) → copied to
  `contracts/src/Groth16Verifier.sol` (drives the `ZkIntegration.t.sol` on-chain proof test).
- **Verification key:** `circuits/build/verification_key.json` (lets anyone `snarkjs groth16 verify`).
- `snarkjs zkey verify r1cs ptau verification_final.zkey` → **ZKey Ok!**

## Reproduce / audit
```bash
cd circuits
# r1cs is deterministic from the committed verification.circom (COMPILE ONLY — never run setup.sh /
# build-circuit, which would overwrite the ceremony zkey with a forgeable dev key):
npm run compile-circuit
# obtain the SAME public Hermez ptau (any mirror; the verify below is the trust anchor):
curl -L -o ptau/powersOfTau28_hez_final_17.ptau \
  https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_17.ptau
node_modules/.bin/snarkjs powersoftau verify ptau/powersOfTau28_hez_final_17.ptau            # Powers of Tau Ok!
node_modules/.bin/snarkjs zkey verify build/verification.r1cs ptau/powersOfTau28_hez_final_17.ptau build/verification_final.zkey  # ZKey Ok!
shasum -a 256 build/verification_final.zkey   # expect: 9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992
# (optional) verify each intermediate contribution + a real proof:
node_modules/.bin/snarkjs groth16 verify build/verification_key.json <public.json> <proof.json>
```
`snarkjs zkey verify` printing **`ZKey Ok!`** and listing the contribution hashes **in the same order**
as the table above is the end-to-end proof the chain is intact.

## On-chain wiring (CUTOVER STARTED — deploy + propose done, execute pending the 2-day timelock)
This v2 ceremony key is **vendored into the repo** (prover crate pin, both app bundles, prover-service).
The captain has **authorized the cutover**; the v2 verifier has now been **deployed and proposed** on the
live registry, **starting the 2-day timelock**. The final `executeZkVerifier()` swap is **NOT yet done** —
it is deferred to firstmate/captain after the wait. As of this run:

- **v2 `Groth16Verifier` DEPLOYED** at `0xEEFCfAF026931b7325472A88fd14Ee780Da13559`
  (zkey sha `9e3636b9…d992`), ROAX `--legacy`, deploy tx
  `0x8204fb78957a87d5cdc775012b89cdc516cfed0755ab6d8501b968848cfa3f54` (block 88968, status success,
  codesize 1932 bytes).
- **`proposeZkVerifier(0xEEFCfAF0…)` SENT** on the live `VerificationRegistry`
  `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1` (single deployer EOA / DEFAULT_ADMIN), propose tx
  `0xa0d0d1f94a65716999bccccb1b1e33c92a78d797bafaa87f9f654278fe4e08ce` (block 88971, status success).
  This **started ZK_TIMELOCK = 2 days**: `zkVerifierEta = 1782988652` → earliest `executeZkVerifier()`
  at **`2026-07-02T10:37:32Z`**. Verified post-state: `pendingZkVerifier() == 0xEEFCfAF0…`,
  `zkVerifier()` STILL `0x138b433071Ad806E841B5AD53623290a9bf21761`.
- **`executeZkVerifier()` HAS NOT BEEN CALLED** — the timelock is cancellable until then; the swap is the
  irreversible step and is left for firstmate/captain after ≥2 days.
- Until execute, the live registry still verifies with the **prior** verifier
  `0x138b433071Ad806E841B5AD53623290a9bf21761` (the earlier testnet key, sha `45d0b6fb…03e`). A proof from
  the v2 key will **not** be accepted on-chain until the swap executes — so `scripts/e2e-zk.sh` (which hits
  the LIVE chain) stays on the old verifier until then; local validation uses the Rust prover self-verify
  oracle (`cargo test -p dogtag-prover-rs`), `snarkjs groth16 verify`, and the on-chain `forge test`
  (`ZkIntegration.t.sol`) against the new verifier source + regenerated fixture.
- **Governance stays the single deployer EOA** (`0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96`) for
  testnet — the multisig migration was deferred by the captain.

The deploy + propose above are done. The remaining **`executeZkVerifier()` is the irreversible on-chain
swap** and runs after the timelock. Exact commands (single deployer EOA, ROAX `--legacy`):

```bash
# 1. Stage the v2 verifier source (already done in this PR) + build:
cp circuits/Groth16Verifier.sol contracts/src/Groth16Verifier.sol
cd contracts && forge build

# 2. Deploy the v2 verifier (deployer = registry DEFAULT_ADMIN = the single EOA above):
VERIFIER=$(forge create src/Groth16Verifier.sol:Groth16Verifier \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy --json | jq -r .deployedTo)
echo "new verifier: $VERIFIER"

# 3. Propose it — starts the 2-day ZK_TIMELOCK:
REG=0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1
cast send "$REG" "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy
#   verify: cast call "$REG" "pendingZkVerifier()(address)" --rpc-url "$ROAX_RPC"  == $VERIFIER

# 4. AFTER >= 2 days, execute:
cast send "$REG" "executeZkVerifier()" \
  --rpc-url "$ROAX_RPC" --private-key "$DEPLOYER_PRIVATE_KEY" --legacy
#   confirm: cast call "$REG" "zkVerifier()(address)" --rpc-url "$ROAX_RPC"  == $VERIFIER

# 5. After the swap: update contracts/deployments/roax.json (Groth16Verifier -> $VERIFIER,
#    _zk_ceremony sha -> 9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992), set EXPECTED_ZKEY_SHA256=9e3636b9c12b57b8662e34505a01e19bfc87a99189c994b0d87bc2e3dcdcd992 on the prover-service if it
#    runs from a build dir whose pin differs, and re-run scripts/e2e-zk.sh against the live chain.
```
See `docs/PRODUCTION_DEPLOYMENT.md` §3.2 and `docs/CEREMONY_RUNBOOK.md` §5 for the full procedure.
