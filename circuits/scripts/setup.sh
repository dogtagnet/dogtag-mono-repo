#!/usr/bin/env bash
# DEV trusted setup for the DogTag verification circuit (impl §11.10(a-f)).
#
# ⚠️ DEV / TEST ONLY. This pipeline uses a SMALL locally-generated powers-of-tau and a
# SINGLE contributor with a throwaway beacon. It is NOT secure for production.
#
# PRODUCTION REQUIRES:
#   - the public Hermez/Perpetual powers-of-tau (`powersOfTau28_hez_final_*.ptau`), NOT a
#     locally generated one (a locally generated ptau means the operator knows tau -> can forge);
#   - a phase-2 ceremony with >= 3 independent contributors, each on isolated hardware,
#     ending in a PUBLIC, verifiable randomness beacon (e.g. a future Ethereum block hash /
#     drand round) so no single party knows the toxic waste.
#
# The circuit has ~31.4k non-linear constraints -> needs 2^15 = 32768 powers (power 15).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
BUILD="$ROOT/build"
CLIB="node_modules/circomlib/circuits"
POWER=15            # 2^15 = 32768 >= 31369 constraints
ENTROPY1="dogtag-dev-ptau-contribution-DO-NOT-USE-IN-PROD"
ENTROPY2="dogtag-dev-zkey-contribution-DO-NOT-USE-IN-PROD"
BEACON="0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"

mkdir -p "$BUILD"

echo "==> [1/7] compile circuit"
circom verification.circom --r1cs --wasm --sym -l "$CLIB" -o "$BUILD"

echo "==> [2/7] powersoftau new (bn128, power $POWER) — DEV ONLY"
npx snarkjs powersoftau new bn128 "$POWER" "$BUILD/pot_0000.ptau" -v

echo "==> [3/7] powersoftau contribute (single dev contributor)"
npx snarkjs powersoftau contribute "$BUILD/pot_0000.ptau" "$BUILD/pot_0001.ptau" \
  --name="dogtag-dev-1" -e="$ENTROPY1" -v

echo "==> [4/7] powersoftau prepare phase2"
npx snarkjs powersoftau prepare phase2 "$BUILD/pot_0001.ptau" "$BUILD/pot_final.ptau" -v

echo "==> [5/7] groth16 setup -> initial zkey"
npx snarkjs groth16 setup "$BUILD/verification.r1cs" "$BUILD/pot_final.ptau" "$BUILD/verification_0000.zkey"

echo "==> [6/7] zkey contribute + beacon (single dev contributor)"
npx snarkjs zkey contribute "$BUILD/verification_0000.zkey" "$BUILD/verification_0001.zkey" \
  --name="dogtag-dev-zkey-1" -e="$ENTROPY2" -v
npx snarkjs zkey beacon "$BUILD/verification_0001.zkey" "$BUILD/verification_final.zkey" \
  "$BEACON" 10 --name="dogtag-dev-beacon" -v

echo "==> [7/7] export verification key + Solidity verifier"
npx snarkjs zkey export verificationkey "$BUILD/verification_final.zkey" "$BUILD/verification_key.json"
npx snarkjs zkey export solidityverifier "$BUILD/verification_final.zkey" "$ROOT/Groth16Verifier.sol"

echo "==> DONE. Pinned zkey: $BUILD/verification_final.zkey"
echo "    Solidity verifier: $ROOT/Groth16Verifier.sol"
