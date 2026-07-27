#!/usr/bin/env bash
# Vendor the owner-hidden consent proving artifacts into BOTH app bundles.
#
# Each app loads exactly one artifact set - the consent pair `consent_final.zkey` + `consent.graph`
# (`ZkeyAsset.kt` / `ZkeyAsset.swift`). Both live under `circuits/build/`; the bundle copies are
# gitignored (`apps/.gitignore`) so the blobs are never double-committed. A fresh checkout therefore
# has the SOURCES but none of the four bundle copies, and neither app can prove until this runs.
#
# Both sources are committed, so this needs no network and no out-of-band build. That is a change:
# `consent.graph` used to be an uncommitted local build, which is why the Android bundle shipped
# without it and the owner-hidden flow was inert there (audit M9 rec 10).
#
# The graph's bytes are attested by `dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256` and
# checked by that module's tests, so what this copies is a known artifact rather than whatever a
# given machine happened to build.
#
# Usage: scripts/vendor-mobile-artifacts.sh   (or: make vendor-mobile-artifacts)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/circuits/build"
IOS="$ROOT/apps/ios/DogTag"
ANDROID="$ROOT/apps/android/app/src/main/assets"

# The attested graph hash, READ FROM `artifact.rs` rather than duplicated here. Duplicating it would
# create exactly the drift this check exists to prevent: a rotation that updated the Rust constant
# but not this script would leave the script silently enforcing a retired hash.
ARTIFACT_RS="$ROOT/crates/dogtag-prover-rs/src/artifact.rs"
EXPECTED_GRAPH_SHA256="$(
  grep -A2 '^pub const LEVEL_B_V1_WITNESS_GRAPH_SHA256' "$ARTIFACT_RS" 2>/dev/null \
    | grep -oE '[0-9a-f]{64}' | head -1
)"
if [ -z "$EXPECTED_GRAPH_SHA256" ]; then
  echo "::error::could not read LEVEL_B_V1_WITNESS_GRAPH_SHA256 from $ARTIFACT_RS" >&2
  echo "Refusing to vendor without an attested hash to check the graph against." >&2
  exit 1
fi

missing=()
for f in consent_final.zkey consent.graph; do
  [ -f "$BUILD/$f" ] || missing+=("$f")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "::error::cannot vendor - missing source artifact(s) under circuits/build: ${missing[*]}" >&2
  echo "Both consent_final.zkey and consent.graph are COMMITTED (force-added past the build/" >&2
  echo "ignore). If either is absent your checkout is incomplete - re-clone or 'git checkout" >&2
  echo "-- circuits/build'. Neither needs an out-of-band build any more." >&2
  exit 1
fi

got="$(shasum -a 256 "$BUILD/consent.graph" | cut -d' ' -f1)"
if [ "$got" != "$EXPECTED_GRAPH_SHA256" ]; then
  echo "::error::circuits/build/consent.graph does not match its attested SHA-256." >&2
  echo "  expected $EXPECTED_GRAPH_SHA256" >&2
  echo "  got      $got" >&2
  echo "Refusing to vendor an unattested witness graph: the app would prove with bytes nothing" >&2
  echo "attests to, which is exactly what pinning exists to prevent. If the graph was rebuilt on" >&2
  echo "purpose, rotate it per docs/ARTIFACT_PIN_RUNBOOK.md." >&2
  exit 1
fi

mkdir -p "$IOS" "$ANDROID"
for dest in "$IOS" "$ANDROID"; do
  cp "$BUILD/consent_final.zkey" "$dest/consent_final.zkey"
  cp "$BUILD/consent.graph" "$dest/consent.graph"
done

echo "vendored the consent pair (attested graph ${EXPECTED_GRAPH_SHA256:0:12}…) into both bundles:"
echo "  $IOS/{consent_final.zkey,consent.graph}"
echo "  $ANDROID/{consent_final.zkey,consent.graph}"
echo
echo "iOS: run this BEFORE 'xcodegen' - it sweeps apps/ios/DogTag/, so regenerating first silently"
echo "drops the prover resources from the pbxproj (docs/MOBILE_BUILD.md §4/§5)."
