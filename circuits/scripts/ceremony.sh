#!/usr/bin/env bash
# DogTag PRODUCTION Groth16 phase-2 trusted-setup ceremony (impl §11.8(f), architecture §13.7/§4.7).
#
# This is the REAL, multi-party ceremony — distinct from scripts/setup.sh (a single-contributor DEV
# setup for tests only). It uses the universal Hermez Powers-of-Tau (phase 1) + a >=3-contributor
# phase 2 ending in a PUBLIC RANDOM BEACON. If even ONE contributor is honest and destroys their
# toxic waste, the setup is sound; if a single party performs every step, that party can forge
# attestations. So contributors MUST be independent and MUST destroy their entropy.
#
# The circuit is DogTagVerification(24,5) ~= 94,459 constraints -> needs 2^17 powers of tau.
# Run the subcommands below in order, passing the .zkey from one contributor to the next.
#
#   ./ceremony.sh init                         # COORDINATOR: download ptau + make contribution #0
#   ./ceremony.sh contribute IN.zkey OUT.zkey "Alice @ orgA"   # each CONTRIBUTOR (>=3), in sequence
#   ./ceremony.sh beacon LAST.zkey 0x<beaconHex> "final beacon" # COORDINATOR: public random beacon
#   ./ceremony.sh finalize FINAL.zkey          # COORDINATOR: verify + export Groth16Verifier.sol + hash
#
# Always `snarkjs zkey verify` the .zkey you RECEIVE before adding your contribution (the script does
# this for you in `contribute`). Publish every intermediate .zkey + the transcript so anyone can audit.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/build"
PTAU_POW=17
PTAU="$ROOT/ptau/powersOfTau28_hez_final_${PTAU_POW}.ptau"
PTAU_URL="https://hermez.s3-eu-west-1.amazonaws.com/powersOfTau28_hez_final_${PTAU_POW}.ptau"
R1CS="$BUILD/verification.r1cs"
SNARKJS="$ROOT/node_modules/.bin/snarkjs"
[ -x "$SNARKJS" ] || SNARKJS="$ROOT/../node_modules/.bin/snarkjs"

cmd="${1:-}"; shift || true

case "$cmd" in
  init)
    [ -f "$R1CS" ] || { echo "missing $R1CS — run 'npm run compile-circuit' first (compile only; NOT 'build-circuit', which runs the dev setup)"; exit 1; }
    mkdir -p "$ROOT/ptau"
    if [ ! -f "$PTAU" ]; then
      echo "Downloading Hermez phase-1 powers-of-tau (2^${PTAU_POW})…"
      curl -L --fail -o "$PTAU" "$PTAU_URL"
    fi
    # sanity: phase-1 file is internally consistent
    "$SNARKJS" powersoftau verify "$PTAU"
    echo "Creating contribution #0 (verification_0000.zkey)…"
    "$SNARKJS" groth16 setup "$R1CS" "$PTAU" "$BUILD/ceremony_0000.zkey"
    echo
    echo "DONE. Send build/ceremony_0000.zkey to the FIRST contributor."
    echo "Each contributor runs:  ./ceremony.sh contribute <in.zkey> <out.zkey> \"<name @ org>\""
    ;;

  contribute)
    IN="$1"; OUT="$2"; NAME="${3:-anonymous}"
    [ -f "$IN" ] || { echo "input zkey not found: $IN"; exit 1; }
    echo "Verifying the zkey you received (chain integrity) before contributing…"
    "$SNARKJS" zkey verify "$R1CS" "$PTAU" "$IN"
    echo
    echo ">>> Add your own UNPREDICTABLE entropy when prompted, then DESTROY it (close the terminal,"
    echo ">>> wipe shell history). Your secrecy is what makes the ceremony sound."
    "$SNARKJS" zkey contribute "$IN" "$OUT" --name="$NAME" -v
    echo
    echo "DONE. Send $OUT to the NEXT contributor (or to the coordinator if you are the last)."
    ;;

  beacon)
    IN="$1"; BEACON="$2"; NAME="${3:-final beacon}"
    # BEACON must be a PUBLIC, unpredictable-in-advance hex value chosen AFTER all contributions:
    # e.g. a future Bitcoin block hash or a drand round. 10 = 2^10 hash iterations.
    echo "Applying public random beacon $BEACON …"
    "$SNARKJS" zkey beacon "$IN" "$BUILD/ceremony_final.zkey" "${BEACON#0x}" 10 -n="$NAME"
    echo "DONE -> build/ceremony_final.zkey"
    ;;

  finalize)
    FINAL="${1:-$BUILD/ceremony_final.zkey}"
    echo "Verifying the final zkey against the circuit + ptau…"
    "$SNARKJS" zkey verify "$R1CS" "$PTAU" "$FINAL"   # must print 'ZKey Ok!'
    echo "Exporting Groth16Verifier.sol…"
    "$SNARKJS" zkey export solidityverifier "$FINAL" "$ROOT/Groth16Verifier.sol"
    echo "Exporting verification_key.json (lets anyone run 'snarkjs groth16 verify')…"
    "$SNARKJS" zkey export verificationkey "$FINAL" "$BUILD/verification_key.json"
    cp "$FINAL" "$BUILD/verification_final.zkey"      # the prover loads this name
    HASH=$(shasum -a 256 "$FINAL" | awk '{print $1}')
    echo
    echo "CEREMONY COMPLETE."
    echo "  final zkey sha256: $HASH   <-- PIN THIS in CI + the prover image (§11.8(f))"
    echo "  verifier:          circuits/Groth16Verifier.sol"
    echo "  vkey:              circuits/build/verification_key.json"
    echo "Next (coordinator): copy the verifier into contracts/src/, deploy it, and wire it into the"
    echo "  live VerificationRegistry via the 2-day timelock — see docs/CEREMONY_RUNBOOK.md §5."
    ;;

  *)
    sed -n '1,30p' "$0"; exit 1 ;;
esac
