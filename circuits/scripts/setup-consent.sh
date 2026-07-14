#!/usr/bin/env bash
# DEV trusted setup for the Level-B DogTagConsent circuit (M2).
#
# ⚠️⚠️  DEV / TEST ONLY — THROWAWAY KEY, **NON-PRODUCTION**.  ⚠️⚠️
# This pipeline uses a SMALL locally-generated powers-of-tau and a SINGLE contributor with a
# throwaway beacon. The operator therefore knows the toxic waste (tau) and could forge proofs.
# The resulting build/consent_final.zkey and Groth16Verifier.consent.sol are ONLY for exercising
# witness/proof round-trips in scripts/test-consent.mjs. They MUST NOT be deployed.
#
# The REAL VK is produced by the M3 trusted-setup ceremony (DONE - scripts/ceremony-consent.sh,
# transcript docs/CEREMONY_TRANSCRIPT.consent.md), which:
#   - reuses the public Hermez/Perpetual powers-of-tau (NOT a locally generated one);
#   - ran a phase-2 ceremony ending in a PUBLIC verifiable randomness beacon.
# The committed M3 key is TESTNET-GRADE (single contributor, captain-approved for ROAX). The mainnet
# re-run must use >= 3 independent contributors on isolated hardware so no single party knows tau.
#
# DogTagConsent (depth=6) has ~38.5k non-linear constraints -> needs 2^16 = 65536 powers (power 16).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
BUILD="$ROOT/build"
CLIB="node_modules/circomlib/circuits"
POWER=16            # 2^16 = 65536 >= 38501 constraints (depth=6)
ENTROPY1="dogtag-consent-DEV-ptau-DO-NOT-USE-IN-PROD"
ENTROPY2="dogtag-consent-DEV-zkey-DO-NOT-USE-IN-PROD"
BEACON="0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20"
SNARKJS="node_modules/.bin/snarkjs"
[ -x "$SNARKJS" ] || SNARKJS="../node_modules/.bin/snarkjs"

mkdir -p "$BUILD"

echo "==> [1/7] compile consent.circom"
circom consent.circom --r1cs --wasm --sym -l "$CLIB" -l . -o "$BUILD"

echo "==> [2/7] powersoftau new (bn128, power $POWER) — DEV ONLY"
"$SNARKJS" powersoftau new bn128 "$POWER" "$BUILD/consent_pot_0000.ptau" -v

echo "==> [3/7] powersoftau contribute (single dev contributor)"
"$SNARKJS" powersoftau contribute "$BUILD/consent_pot_0000.ptau" "$BUILD/consent_pot_0001.ptau" \
  --name="dogtag-consent-dev-1" -e="$ENTROPY1" -v

echo "==> [4/7] powersoftau prepare phase2"
"$SNARKJS" powersoftau prepare phase2 "$BUILD/consent_pot_0001.ptau" "$BUILD/consent_pot_final.ptau" -v

echo "==> [5/7] groth16 setup -> initial zkey"
"$SNARKJS" groth16 setup "$BUILD/consent.r1cs" "$BUILD/consent_pot_final.ptau" "$BUILD/consent_0000.zkey"

echo "==> [6/7] zkey contribute + beacon (single dev contributor)"
"$SNARKJS" zkey contribute "$BUILD/consent_0000.zkey" "$BUILD/consent_0001.zkey" \
  --name="dogtag-consent-dev-zkey-1" -e="$ENTROPY2" -v
"$SNARKJS" zkey beacon "$BUILD/consent_0001.zkey" "$BUILD/consent_final.zkey" \
  "$BEACON" 10 --name="dogtag-consent-dev-beacon" -v

echo "==> [7/7] export verification key + Solidity verifier (DEV)"
"$SNARKJS" zkey export verificationkey "$BUILD/consent_final.zkey" "$BUILD/consent_verification_key.json"
"$SNARKJS" zkey export solidityverifier "$BUILD/consent_final.zkey" "$ROOT/Groth16Verifier.consent.dev.sol"

echo "==> DONE (DEV/THROWAWAY). zkey: $BUILD/consent_final.zkey"
echo "    ⚠️ NON-PRODUCTION — do not deploy. Real VK == M3 ceremony."
