# DogTag ZK Ceremony — Transcript (TESTNET self-run)

> **Trust caveat (read first).** This is a **testnet** phase-2 trusted setup that was run end-to-end on
> a single machine — all three "contributors" and the beacon were produced by one operator. It is
> therefore **NOT production-trustworthy**: a single party performed every contribution and could, in
> principle, hold the toxic waste and forge ZK attestations. It exists so the testnet ZK path is fully
> wired and reproducible-on-record. For mainnet, re-run `circuits/scripts/ceremony.sh` with **≥3
> genuinely independent contributors** and a real public beacon (see `docs/CEREMONY.md`). The core
> three-pillar trust model and the normal/ECDSA verification path do **not** depend on this ceremony.

## Circuit
- `circuits/verification.circom` → `DogTagVerification(24, 5)`
- Constraints: **94,459** (labels 171,389) → powers of tau **2^17**
- Circuit hash (snarkjs, constant across all steps):
  `6d78539a 6675aa95 1abf3e3b 9bd1d284 5965dd2d 5b327031 6e1e7bcf 8770c79c 5cd93acf f79138f9 93b80825 a28e378f 9ca7f877 cad6238a 9d58e1b3 345cfdd1`
- Phase-1 ptau: `circuits/build/pot_final.ptau` (locally generated for testnet; mainnet uses Hermez).

## Phase-2 contributions (in order)
| # | Name | zkey sha256 | contribution hash (first words) |
|---|---|---|---|
| 0 | groth16 setup (zero contribution) | `7512a3733486d1c8782d75685fc52fbb9a800652f7c416e0693fb989733b1612` | — |
| 1 | testnet-contributor-1 | `fc198a0540a462606bfcee250d31e69b11516e3fa8e702699ad00b466a0e14b9` | `a2cb5bc0 86f5a975 3f7e2611 de59a9ff …` |
| 2 | testnet-contributor-2 | `7062dd62618269b5f86fa2e4d77171ce0b4e7764453b6bf35f6d5eb3e005f870` | `e96912b8 eded79a8 b0c7b399 6161c7e8 …` |
| 3 | testnet-contributor-3 | `d621bd1d94fb58e65736a9aaf72f5b7f209135b90790d55e0fdc2dacbeb7c212` | `350b6e8e d17de9aa 80e0357a 4d85eaf4 …` |
| beacon | final beacon (testnet) | `45d0b6fb78591548f5763e86f614d1c04cf48a80d35445d1740c0ba561fdc03e` | `d61d84c2 ea221998 89d7c3a0 a935dba6 …` |

Each contribution used 64 bytes of `/dev/urandom` entropy (`-e`), not recorded (destroyed by design).

## Public beacon
- Generator (hex): `ead1c929dbf9f056be33438062738bc09f34e984372e12c75418281d67298c99`
- Iterations exp: `10`
- Derivation (testnet): `sha256("dogtag-testnet-ceremony-<UTC-timestamp>")`. For mainnet, use an
  unpredictable public value chosen AFTER the last contribution (a future BTC block hash / drand round).

## Final artifacts (PINNED)
- **Final zkey sha256:** `45d0b6fb78591548f5763e86f614d1c04cf48a80d35445d1740c0ba561fdc03e`
  → committed as `circuits/build/verification_final.zkey` (the prover `dogtag-prover-rs` loads this).
- **Verifier:** `circuits/Groth16Verifier.sol` (9,391 bytes) → copied to `contracts/src/Groth16Verifier.sol`.
- **Verification key:** `circuits/build/verification_key.json` (lets anyone `snarkjs groth16 verify`).
- `snarkjs zkey verify r1cs ptau verification_final.zkey` → **ZKey Ok!**

## Reproduce / audit
```bash
cd circuits
# r1cs + wasm are deterministic from the committed verification.circom:
circom verification.circom --r1cs --wasm --sym -l node_modules/circomlib/circuits -o build   # DO NOT re-run setup.sh (it would overwrite the ceremony zkey)
node_modules/.bin/snarkjs zkey verify build/verification.r1cs build/pot_final.ptau build/verification_final.zkey   # expect: ZKey Ok!
shasum -a 256 build/verification_final.zkey   # expect: 45d0b6fb…03e
```

## On-chain wiring
The ceremony `Groth16Verifier` is deployed to ROAX and wired into the live `VerificationRegistry`
(testnet: the registry was redeployed pointing at the verifier so the ZK path is active immediately,
rather than waiting out the 2-day `setZkVerifier` timelock). Addresses: `contracts/deployments/roax.json`.
