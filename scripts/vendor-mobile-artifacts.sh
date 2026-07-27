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
# BOTH copies are hash-verified first. The graph's bytes are attested by
# `dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256` and the zkey's by the descriptor's own
# mandatory `zkey.sha256` pin, so what this copies is a known artifact rather than whatever a given
# machine happened to build or fetch. That matters most for the zkey: the SERVER prover re-hashes it
# against the same pin on every load, but a mobile bundle is never integrity-checked at runtime, so
# this script is the only gate standing between a corrupt or wrong-ceremony key and a signed app.
#
# Usage: scripts/vendor-mobile-artifacts.sh   (or: make vendor-mobile-artifacts)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/circuits/build"
IOS="$ROOT/apps/ios/DogTag"
ANDROID="$ROOT/apps/android/app/src/main/assets"

# Both attested hashes, READ FROM `artifact.rs` rather than duplicated here. Duplicating either would
# create exactly the drift these checks exist to prevent: a rotation that updated the Rust side but
# not this script would leave the script silently enforcing a retired hash.
#
# The `|| true` inside each substitution is load-bearing, not defensive noise. Under `set -euo
# pipefail` the pipeline's exit status escapes the substitution, so a grep that matches nothing (an
# absent file, a renamed constant, a reordered struct field) aborts the whole script BEFORE the
# explicit refusal below can name the problem - and `head -1` can even SIGPIPE a *successful* grep to
# 141. Either way the operator gets a bare non-zero exit instead of the `::error::` line, so the
# guards must own the failure rather than inherit it.
ARTIFACT_RS="$ROOT/crates/dogtag-prover-rs/src/artifact.rs"
EXPECTED_GRAPH_SHA256="$(
  { grep -A2 '^pub const LEVEL_B_V1_WITNESS_GRAPH_SHA256' "$ARTIFACT_RS" \
      | grep -oE '[0-9a-f]{64}' | head -1; } 2>/dev/null || true
)"
EXPECTED_ZKEY_SHA256="$(
  { grep -A2 'rel_path: "consent_final.zkey"' "$ARTIFACT_RS" \
      | grep -oE '[0-9a-f]{64}' | head -1; } 2>/dev/null || true
)"
if [ -z "$EXPECTED_GRAPH_SHA256" ]; then
  echo "::error::could not read LEVEL_B_V1_WITNESS_GRAPH_SHA256 from $ARTIFACT_RS" >&2
  echo "Refusing to vendor without an attested hash to check the graph against." >&2
  exit 1
fi
if [ -z "$EXPECTED_ZKEY_SHA256" ]; then
  echo "::error::could not read the consent zkey pin (ZkeyArtifact.sha256) from $ARTIFACT_RS" >&2
  echo "Refusing to vendor without a pinned hash to check consent_final.zkey against." >&2
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

verify_attested() {
  local file="$1" expected="$2" got
  got="$(shasum -a 256 "$BUILD/$file" | cut -d' ' -f1)"
  [ "$got" = "$expected" ] && return 0
  echo "::error::circuits/build/$file does not match its attested SHA-256." >&2
  echo "  expected $expected" >&2
  echo "  got      $got" >&2
  echo "Refusing to vendor an unattested proving artifact: the app would prove with bytes nothing" >&2
  echo "attests to, which is exactly what pinning exists to prevent - and unlike the server prover," >&2
  echo "the mobile bundle re-checks nothing at runtime, so this is the only gate. If $file was" >&2
  echo "replaced on purpose, rotate it per docs/ARTIFACT_PIN_RUNBOOK.md." >&2
  exit 1
}

verify_attested consent_final.zkey "$EXPECTED_ZKEY_SHA256"
verify_attested consent.graph "$EXPECTED_GRAPH_SHA256"

mkdir -p "$IOS" "$ANDROID"
for dest in "$IOS" "$ANDROID"; do
  cp "$BUILD/consent_final.zkey" "$dest/consent_final.zkey"
  cp "$BUILD/consent.graph" "$dest/consent.graph"
done

echo "vendored the consent pair into both bundles (zkey ${EXPECTED_ZKEY_SHA256:0:12}…, graph ${EXPECTED_GRAPH_SHA256:0:12}…, both attested):"
echo "  $IOS/{consent_final.zkey,consent.graph}"
echo "  $ANDROID/{consent_final.zkey,consent.graph}"
echo
echo "iOS: run this BEFORE 'xcodegen' - it sweeps apps/ios/DogTag/, so regenerating first silently"
echo "drops the prover resources from the pbxproj (docs/MOBILE_BUILD.md §4/§5)."
