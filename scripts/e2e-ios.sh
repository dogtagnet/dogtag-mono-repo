#!/usr/bin/env bash
# Launch the iOS end-to-end (Maestro) flows against a simulator. ONE command.
#
#   make e2e-ios
#   scripts/e2e-ios.sh --flow apps/ios/maestro/zk_e2e.yaml
#
# WHY THIS EXISTS
#
# These flows have never been launchable on demand. The CI workflow is `workflow_dispatch`-only and
# needs a self-hosted arm64 runner, so the only real signal was a hand-run that required somebody to
# know a nine-step recipe from docs/MOBILE_BUILD.md first. "Could not run" was the honest answer every
# time, and a suite nobody can launch is decoration rather than a gate.
#
# THE OUTCOME THAT MATTERS IS THE THIRD ONE
#
# Passing and failing are easy. What this script exists to get right is refusing: when there is no
# simulator, no xcframework, or no vendored proving artifacts, it says exactly which and exits 78. It
# never skips quietly and never reports green. That is not hypothetical here - both mobile UNIT suites
# once failed to COMPILE and read as "no failures", because a module that does not run reports nothing
# and nothing is indistinguishable from success.
#
# WHAT IT DELIBERATELY DOES NOT DO
#
# It never runs `xcodegen`. The committed `project.pbxproj` references the two gitignored proving
# artifacts as bundle resources, and xcodegen enumerates the DogTag/ folder - so regenerating in a
# checkout that has not vendored them SILENTLY drops both Copy-Bundle-Resources entries and the app
# then builds clean and proves with nothing. The project is committed; this script builds it as-is.
#
# It also never boots a simulator it did not find already available, never deletes one, and never
# kills a process it did not start.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/e2e.sh
source "$ROOT_DIR/scripts/lib/e2e.sh"
E2E_SUITE="e2e-ios"
cd "$ROOT_DIR"

readonly APP_ID="io.liberalize.dogtag"
readonly FLOW_DIR="apps/ios/maestro"
FLOWS=()
KEEP_BOOTED=0
INCLUDE_DEPLOYMENT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --flow) FLOWS+=("$2"); shift 2 ;;
    --keep-booted) KEEP_BOOTED=1; shift ;;
    --include-deployment) INCLUDE_DEPLOYMENT=1; shift ;;
    -h|--help)
      printf 'usage: scripts/e2e-ios.sh [--flow <file>]... [--keep-booted]\n'
      printf '  --flow         run only this flow (repeatable); default is every flow in %s\n' "$FLOW_DIR"
      printf '  --keep-booted  leave a simulator this script booted running, for a follow-up run\n'
      printf '  --include-deployment  ALSO run flows tagged requires-deployment (they need a\n'
      printf '                        reachable AppConfig.centralApi; excluded by default)\n'
      exit 0 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done

arm_teardown

# ---------------------------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------------------------
step "preflight"
need_cmd maestro "Install Maestro: \`curl -Ls https://get.maestro.mobile.dev | bash\`."
need_cmd xcrun "Install Xcode and its command line tools (\`xcode-select --install\`)."
command -v xcodebuild >/dev/null 2>&1 || could_not_run \
  "\`xcodebuild\` (the full Xcode toolchain; the command line tools alone are not enough)" \
  "Install Xcode, then \`sudo xcode-select -s /Applications/Xcode.app\`."

[[ -d apps/ios/DogTag.xcodeproj ]] || could_not_run \
  "apps/ios/DogTag.xcodeproj" \
  "It is committed - this checkout looks incomplete. Do NOT run xcodegen to recreate it without
             vendoring the proving artifacts first; see the header of this script."

# The three gitignored bundle inputs. A fresh checkout has none of them, and the build fails with a
# bare "Build input file cannot be found" that reads like a project fault - so name each one and the
# command that produces it. That failure IS the guard working, not a bug.
for pair in \
  "apps/ios/DogTag/consent_final.zkey|make vendor-mobile-artifacts" \
  "apps/ios/DogTag/consent.graph|make vendor-mobile-artifacts" \
  "apps/ios/DogTag/roax.json|make gen-mobile-config"
do
  f="${pair%%|*}"; fix="${pair##*|}"
  [[ -f "$f" ]] || could_not_run "$f (gitignored; a fresh checkout has none)" "Run \`$fix\`."
done

# The native prover. Gitignored and NOT produced by an Xcode build - without it the app does not link,
# and with a STALE one it links and then traps at the first FFI call on a UniFFI checksum mismatch.
[[ -d apps/ios/DogTagFFI.xcframework ]] || could_not_run \
  "apps/ios/DogTagFFI.xcframework (the on-device Rust prover; gitignored, and NOT built by Xcode)" \
  "Build it per docs/MOBILE_BUILD.md - in short: \`rustup target add aarch64-apple-ios-sim\`,
             \`cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios-sim --lib\`,
             then \`xcodebuild -create-xcframework\`. The \`--features prover\` is mandatory: without it
             proveConsent is absent from the FFI surface and the app will not link."

if [[ ${#FLOWS[@]} -eq 0 ]]; then
  while IFS= read -r f; do FLOWS+=("$f"); done < <(find "$FLOW_DIR" -name '*.yaml' | sort)
fi
[[ ${#FLOWS[@]} -gt 0 ]] || could_not_run "any Maestro flow under $FLOW_DIR" "This checkout looks incomplete."
for f in "${FLOWS[@]}"; do
  [[ -f "$f" ]] || could_not_run "the flow file $f" "Check the path passed to --flow."
done
info "${#FLOWS[@]} flow(s) to run"

# ---------------------------------------------------------------------------------------------
# A simulator. arm64 ONLY: the prover ships as an arm64 slice, so an x86_64 simulator cannot load it
# and would fail deep inside the flow with something that looks like a product defect.
# ---------------------------------------------------------------------------------------------
step "resolving a simulator"
SIM_UDID="$(xcrun simctl list devices booted -j 2>/dev/null | python3 -c '
import json,sys
d=json.load(sys.stdin)
for runtime, devs in d.get("devices", {}).items():
    for dev in devs:
        if dev.get("state")=="Booted" and dev.get("isAvailable", True):
            print(dev["udid"]); raise SystemExit
' || true)"

BOOTED_BY_US=""
if [[ -n "$SIM_UDID" ]]; then
  info "using the already-booted simulator $SIM_UDID (this script did not start it and will not stop it)"
else
  # Nothing booted. Pick an available iPhone runtime and boot it - recording that WE booted it, so
  # teardown only ever shuts down a simulator this script started.
  SIM_UDID="$(xcrun simctl list devices available -j 2>/dev/null | python3 -c '
import json,sys
d=json.load(sys.stdin)
best=None
for runtime, devs in d.get("devices", {}).items():
    if "iOS" not in runtime: continue
    for dev in devs:
        if not dev.get("isAvailable", False): continue
        if "iPhone" not in dev.get("name",""): continue
        best=(runtime, dev["udid"], dev["name"])
if best: print(best[1])
' || true)"
  [[ -n "$SIM_UDID" ]] || could_not_run \
    "any available iOS simulator (none booted, and none installable found)" \
    "Install an iOS runtime in Xcode > Settings > Platforms, then retry. On Apple Silicon the
             simulator is arm64, which is what the bundled prover slice requires."
  info "booting simulator $SIM_UDID"
  xcrun simctl boot "$SIM_UDID" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$SIM_UDID" -b >/dev/null 2>&1 || could_not_run \
    "a bootable iOS simulator (\`simctl boot $SIM_UDID\` did not come up)" \
    "Open Simulator.app once by hand and check the runtime is installed."
  BOOTED_BY_US="$SIM_UDID"
fi

shutdown_our_sim() {
  if [[ -n "$BOOTED_BY_US" && "$KEEP_BOOTED" -eq 0 ]]; then
    xcrun simctl shutdown "$BOOTED_BY_US" >/dev/null 2>&1 || true
  fi
  stop_tracked
}
trap shutdown_our_sim EXIT INT TERM

# ---------------------------------------------------------------------------------------------
# Build + install
# ---------------------------------------------------------------------------------------------
step "building the DEBUG app (the ZK self-test card is #if DEBUG, so Release proves nothing)"
DERIVED="$E2E_TMPDIR/derived"
BUILD_LOG="$E2E_TMPDIR/xcodebuild.log"
if ! ( cd apps/ios && xcodebuild \
        -project DogTag.xcodeproj -scheme DogTag -configuration Debug \
        -sdk iphonesimulator -destination "platform=iOS Simulator,id=$SIM_UDID" \
        -derivedDataPath "$DERIVED" \
        ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
        build ) >"$BUILD_LOG" 2>&1
then
  printf '\n--- last 40 lines of xcodebuild ---\n' >&2
  tail -40 "$BUILD_LOG" >&2
  failed "the iOS app did not build (log above). That is a code finding, not a missing prerequisite -
  every prerequisite was checked before this point."
fi
APP="$DERIVED/Build/Products/Debug-iphonesimulator/DogTag.app"
[[ -d "$APP" ]] || failed "xcodebuild reported success but $APP is absent."
info "built $APP"

step "installing on $SIM_UDID"
xcrun simctl install "$SIM_UDID" "$APP" || failed "simctl install failed."
info "installed $APP_ID"

# ---------------------------------------------------------------------------------------------
# Run the flows
# ---------------------------------------------------------------------------------------------
# SOME FLOWS NEED A REAL DEPLOYMENT, AND THAT MUST NOT LOOK LIKE A FAILURE OR LIKE A PASS.
#
# The app's directory host is a fixed production constant with no debug override - deliberately, since
# it is the one service the owner cannot swap - so a dev machine cannot render a provider row at all.
# A flow that depends on one is tagged `requires-deployment` and is excluded here by default.
#
# It is EXCLUDED, never SILENT: what was left out is listed below with the reason and the flag that
# runs it, because a suite that quietly covers less than it appears to is the failure this whole
# runner exists to prevent. AGENTS.md's rule for exactly this: if a run bounds its coverage, log what
# was dropped - silent truncation reads as "covered everything" when it did not.
EXCLUDED=()
if [[ "$INCLUDE_DEPLOYMENT" -eq 0 ]]; then
  for f in "${FLOWS[@]}"; do
    grep -q 'requires-deployment' "$f" 2>/dev/null && EXCLUDED+=("$f")
  done
  if [[ ${#EXCLUDED[@]} -gt 0 ]]; then
    REMAINING=()
    for f in "${FLOWS[@]}"; do
      skip_this=0
      for x in "${EXCLUDED[@]}"; do [[ "$f" == "$x" ]] && skip_this=1 && break; done
      [[ $skip_this -eq 0 ]] && REMAINING+=("$f")
    done
    FLOWS=("${REMAINING[@]}")
  fi
fi

step "running ${#FLOWS[@]} Maestro flow(s)"
if [[ ${#EXCLUDED[@]} -gt 0 ]]; then
  printf '  NOT RUN - %d flow(s) need a reachable deployment at AppConfig.centralApi:\n' "${#EXCLUDED[@]}"
  for f in "${EXCLUDED[@]}"; do printf '    %s\n' "$f"; done
  printf '  They are NOT counted below, in either direction. Against a real deployment, run:\n'
  printf '    xcrun simctl location %s set <lat>,<lng>\n' "$SIM_UDID"
  printf '    scripts/e2e-ios.sh --include-deployment\n'
fi
[[ ${#FLOWS[@]} -gt 0 ]] || could_not_run \
  "any runnable flow (every one selected needs a deployment)" \
  "Pass --include-deployment against a real deployment, or select a hermetic flow with --flow."

REPORT="$E2E_TMPDIR/ios-junit.xml"
set +e
# Bounded: a Maestro that hangs must become a loud failure rather than an indefinite wait. See
# `run_with_deadline` for the incident this came from.
MAESTRO_DEBUG="$E2E_TMPDIR/maestro-debug"
# `--debug-output` so the harness log lands somewhere WE can read: Maestro otherwise writes it
# under ~/.maestro/tests/<timestamp>/, and a driver failure prints only `[Failed] <flow> (0s)` on
# the console - the one line that cannot be told from a failed first assertion.
run_with_deadline 600 "maestro" maestro --udid "$SIM_UDID" test --format JUNIT --output "$REPORT" \
  --debug-output "$MAESTRO_DEBUG" --flatten-debug-output "${FLOWS[@]}"
MAESTRO_RC=$?
set -e
if [[ $MAESTRO_RC -ne 0 ]]; then
  if HARNESS_HIT="$(maestro_harness_failure "$MAESTRO_DEBUG")"; then
    could_not_run \
      "a working Maestro driver on this device - its own harness failed before any assertion ran" \
      "Maestro reported: $HARNESS_HIT
             Nothing was asserted, so this says nothing about the app. Try a fresh emulator, or
             reinstall the driver with \`maestro --udid <id> test --reinstall-driver <flow>\`."
  fi
fi
[[ $MAESTRO_RC -eq 124 ]] && failed "Maestro did not finish within its deadline and was stopped.
  It is NOT a pass and NOT a normal failure: nothing can be concluded about the flows. Maestro 2.6.1
  is known to stall at driver setup on a cold emulator - retry against a warm one, or run the flow by
  hand with \`maestro --udid <id> test <flow>\` to see where it stops."

# The counts come from the JUnit XML, never from the console. A Maestro run that matched no flow, or
# a runner whose exit code says one thing while it executed nothing, both look identical on stdout.
[[ -f "$REPORT" ]] || failed "Maestro produced no JUnit report (exit $MAESTRO_RC), so it cannot be
  shown to have run. An exit code on its own is not evidence that anything executed."

COUNTS="$(python3 - "$REPORT" <<'PY'
import sys, xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
suites = [root] if root.tag == "testsuite" else root.findall(".//testsuite")
t = f = s = 0
for su in suites:
    t += int(su.get("tests", 0) or 0)
    f += int(su.get("failures", 0) or 0) + int(su.get("errors", 0) or 0)
    s += int(su.get("skipped", 0) or 0)
print("%d %d %d" % (t, f, s))
PY
)"
read -r TESTS FAILURES SKIPPED <<< "$COUNTS"

assert_ran "e2e-ios" "$TESTS"

step "summary"
printf '  flows executed : %d\n' "$TESTS"
printf '  failures       : %d\n' "$FAILURES"
printf '  skipped        : %d\n' "$SKIPPED"
printf '  simulator      : %s%s\n' "$SIM_UDID" "$([[ -n "$BOOTED_BY_US" ]] && echo ' (booted by this script)' || echo ' (was already booted)')"

# A skipped flow verifies nothing. This runner supplies everything the flows gate on, so a skip means
# that setup did not hold rather than that the flow was inapplicable.
[[ "$SKIPPED" -eq 0 ]] || failed "$SKIPPED flow(s) were SKIPPED. A skipped flow proves nothing and
  must never be counted as passing - investigate why the flow declined to run."
[[ "$FAILURES" -eq 0 && $MAESTRO_RC -eq 0 ]] || failed "$FAILURES flow(s) failed (maestro exit $MAESTRO_RC)."

printf '\ne2e-ios: OK - %d flow(s) passed on %s.\n' "$TESTS" "$SIM_UDID"
