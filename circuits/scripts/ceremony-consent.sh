#!/usr/bin/env bash
# DogTag Level-B DogTagConsent (M3) — TESTNET phase-2 trusted-setup ceremony.
#
# ⚠️⚠️  TESTNET-GRADE, NOT MAINNET.  ⚠️⚠️
# A SINGLE phase-2 contribution is performed on our own infra, then finalized with a PUBLIC drand
# beacon. Because one party performs the only contribution, that party could in principle retain the
# toxic waste and forge consent attestations. This is ACCEPTABLE FOR ROAX TESTNET ONLY — it is the
# captain-approved M3 scope (level-b-spec.md "M3", data/dogtag-zkverify-z2). The mainnet ceremony
# (>=3 genuinely independent external contributors, pre-announced beacon) stays DEFERRED and is what
# circuits/scripts/ceremony.sh is written for. See docs/CEREMONY_TRANSCRIPT.consent.md.
#
# This is a REAL ceremony, NOT the dev throwaway (scripts/setup-consent.sh). The two things that make
# it real rather than forgeable-by-construction:
#   - phase-1 is the PUBLIC Hermez / Perpetual Powers-of-Tau (power 17), NOT a locally generated ptau
#     (a self-generated ptau would let the operator know tau); the file is sha256-pinned below and is
#     BYTE-IDENTICAL to the ptau the repo already established trust in for the v2 verification ceremony
#     (docs/CEREMONY_TRANSCRIPT.md). `snarkjs powersoftau verify` is the independent audit (slow; opt in
#     with CEREMONY_RUN_PTAU_VERIFY=1). The final `snarkjs zkey verify` below also reads this exact ptau
#     and cryptographically validates the whole phase-2 chain against it.
#   - the final contribution is a PUBLIC, verifiable drand randomness beacon (not a hardcoded pseudo-beacon).
#
# consent.circom -> DogTagConsent(6): 38,501 non-linear constraints, 0 public inputs, 7 public outputs
#   [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]  ->  needs 2^16; the pow-17 ptau covers it.
#
# Run once (single contributor):  ./scripts/ceremony-consent.sh
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
BUILD="$ROOT/build"
CLIB="node_modules/circomlib/circuits"
POWER=17
PTAU="$ROOT/ptau/powersOfTau28_hez_final_${POWER}.ptau"
# The original Hermez S3 bucket returns AccessDenied; Polygon's official zkEVM mirror serves the identical
# file. The URL is NOT the trust anchor — the pinned sha256 + `zkey verify` are. Override with PTAU_URL=.
PTAU_URL="${PTAU_URL:-https://storage.googleapis.com/zkevm/ptau/powersOfTau28_hez_final_${POWER}.ptau}"
PTAU_SHA256="6b662a324867139fb1a20a324d90b6ff61856dfb23f59326909f14b0e2483ae0"
# drand League-of-Entropy mainnet chain (same chain the v2 ceremony used).
DRAND_CHAIN="8990e7a9aaed2ffed73dbd7092123d6f289930540d7651336225dc172e51b2ce"
R1CS="$BUILD/consent.r1cs"
SNARKJS="${SNARKJS:-snarkjs}"

echo "==> [1/8] compile consent.circom (r1cs is deterministic from the committed circuit)"
if [ ! -f "$R1CS" ]; then
  circom consent.circom --r1cs --wasm --sym -l "$CLIB" -l . -o "$BUILD"
else
  echo "    reusing existing $R1CS"
fi

echo "==> [2/8] ensure public Hermez ptau (power $POWER) + verify sha256"
mkdir -p "$ROOT/ptau"
if [ ! -f "$PTAU" ]; then
  echo "    downloading $PTAU_URL"
  curl -L --fail -o "$PTAU" "$PTAU_URL"
fi
ACTUAL_PTAU_SHA="$(shasum -a 256 "$PTAU" | awk '{print $1}')"
[ "$ACTUAL_PTAU_SHA" = "$PTAU_SHA256" ] || { echo "PTAU SHA MISMATCH: $ACTUAL_PTAU_SHA != $PTAU_SHA256"; exit 1; }
echo "    ptau sha256 OK: $PTAU_SHA256"
if [ "${CEREMONY_RUN_PTAU_VERIFY:-0}" = 1 ]; then
  echo "    running full powersoftau verify (slow, single-threaded)…"
  "$SNARKJS" powersoftau verify "$PTAU"
fi

echo "==> [3/8] groth16 setup -> contribution #0 (zero contribution)"
"$SNARKJS" groth16 setup "$R1CS" "$PTAU" "$BUILD/consent_0000.zkey"

echo "==> [4/8] phase-2 contribution #1 (single testnet contributor)"
# Fresh 64-byte OS entropy fed to the contribute prompt (snarkjs also mixes in its own OS RNG). The
# entropy is NOT recorded and is destroyed when this shell exits — only the contribution hash is published.
ENTROPY="$(openssl rand -hex 64)"
"$SNARKJS" zkey contribute "$BUILD/consent_0000.zkey" "$BUILD/consent_0001.zkey" \
  --name="dogtag-consent-selfrun-testnet-1" -e="$ENTROPY" -v
unset ENTROPY

echo "==> [5/8] public drand beacon -> final zkey"
BEACON_JSON="$(curl -sS --fail "https://api.drand.sh/${DRAND_CHAIN}/public/latest")"
jget() { node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(String(JSON.parse(s)["'"$1"'"])))'; }
BEACON_ROUND="$(printf '%s' "$BEACON_JSON" | jget round)"
BEACON_HEX="$(printf '%s' "$BEACON_JSON" | jget randomness)"
BEACON_SIG="$(printf '%s' "$BEACON_JSON" | jget signature)"
echo "    drand chain=$DRAND_CHAIN round=$BEACON_ROUND randomness=$BEACON_HEX"
"$SNARKJS" zkey beacon "$BUILD/consent_0001.zkey" "$BUILD/consent_final.zkey" "$BEACON_HEX" 10 \
  -n="dogtag-consent-drand-round-$BEACON_ROUND"

echo "==> [6/8] verify final zkey against r1cs + ptau (MUST print 'ZKey Ok!')"
"$SNARKJS" zkey verify "$R1CS" "$PTAU" "$BUILD/consent_final.zkey"

echo "==> [7/8] export verification key + Groth16VerifierConsent.sol"
"$SNARKJS" zkey export verificationkey "$BUILD/consent_final.zkey" "$BUILD/consent_verification_key.json"
"$SNARKJS" zkey export solidityverifier "$BUILD/consent_final.zkey" "$BUILD/consent_verifier.sol.tmp"
# snarkjs names the contract `Groth16Verifier`; rename so it does NOT collide with the LIVE v2 verifier
# contract of the same name (circuits/Groth16Verifier.sol / contracts/src/Groth16Verifier.sol).
sed 's/contract Groth16Verifier {/contract Groth16VerifierConsent {/' \
  "$BUILD/consent_verifier.sol.tmp" > "$ROOT/Groth16Verifier.consent.sol"
rm -f "$BUILD/consent_verifier.sol.tmp"

echo "==> [8/8] DONE (TESTNET-GRADE). Ceremony summary (record in docs/CEREMONY_TRANSCRIPT.consent.md):"
echo "    ptau:              $(basename "$PTAU")  (power $POWER, sha256 $PTAU_SHA256)"
echo "    drand chain:       $DRAND_CHAIN"
echo "    drand round:       $BEACON_ROUND"
echo "    drand randomness:  $BEACON_HEX"
echo "    drand signature:   $BEACON_SIG"
echo "    final zkey sha256: $(shasum -a 256 "$BUILD/consent_final.zkey" | awk '{print $1}')"
echo "    VK json sha256:    $(shasum -a 256 "$BUILD/consent_verification_key.json" | awk '{print $1}')"
echo "    verifier:          circuits/Groth16Verifier.consent.sol (contract Groth16VerifierConsent)"
echo "    NON-PRODUCTION-MAINNET: single-contributor testnet key; re-run this script (ceremony-consent.sh) with >=3 independent contributors before mainnet."
