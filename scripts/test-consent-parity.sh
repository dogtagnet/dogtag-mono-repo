#!/usr/bin/env bash
# DogTag consent prove<->VK parity gate - the LOUD entry point.
#
# Runs the repo's ONLY empirical proof that the on-device consent prover agrees with the frozen M3
# consent VK (`crates/dogtag-standard-rs/tests/consent_prove_parity.rs`).
#
# It exists because the bare cargo invocation can report green in two ways WITHOUT running the check,
# and neither is visible in the output:
#
#   1. The test file is `#![cfg(feature = "prover")]`, so `cargo test -p dogtag-standard-rs` (no
#      feature flag) compiles it away and prints `running 0 tests`. This script always passes
#      `--features prover`.
#   2. The test self-skips when the proving artifacts are absent, and libtest CAPTURES stdout for
#      PASSING tests - so a print from inside the test cannot annotate anything. This script checks
#      the artifacts FIRST, from the shell, where a `::error::` line is parsed by GitHub and an
#      exit code is a real failure.
#
# Usage: scripts/test-consent-parity.sh   (or: make test-consent-parity)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/circuits/build"
ZKEY="$BUILD/consent_final.zkey"
GRAPH="$BUILD/consent.graph"

missing=()
[ -f "$ZKEY" ]  || missing+=("consent_final.zkey ($ZKEY)")
[ -f "$GRAPH" ] || missing+=("consent.graph ($GRAPH)")

if [ ${#missing[@]} -gt 0 ]; then
  echo "::error::consent prove<->VK parity NOT run - missing proving artifact(s): ${missing[*]}"
  echo "BOTH consent_final.zkey and consent.graph are committed (force-added past the build/ ignore)," >&2
  echo "so a missing one means your checkout is incomplete, not that you skipped an out-of-band build." >&2
  echo "Restore them with 'git checkout -- circuits/build'." >&2
  echo "The graph no longer needs iden3's build-circuit: its bytes are committed and attested by" >&2
  echo "dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256 (see docs/ARTIFACT_PIN_RUNBOOK.md)." >&2
  exit 1
fi

echo "consent proving artifacts present - running the prove<->VK parity check (real Groth16, slow)."
exec cargo test -p dogtag-standard-rs --features prover --test consent_prove_parity
