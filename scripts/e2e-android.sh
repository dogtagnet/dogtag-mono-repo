#!/usr/bin/env bash
# Launch the Android end-to-end (Maestro) flows against a device or emulator. ONE command.
#
#   make e2e-android
#   scripts/e2e-android.sh --flow apps/android/maestro/zk_e2e.yaml
#
# WHY THIS EXISTS
#
# Same reason as its iOS twin: the CI job is `workflow_dispatch`-only on a self-hosted arm64 runner,
# so the only real signal was a hand-run behind a multi-step recipe. "Could not run" was the honest
# answer every time, and a gate nobody can launch is not a gate.
#
# THE OUTCOME THAT MATTERS IS THE THIRD ONE
#
# When there is no device, no NDK-built native library, or no vendored proving artifacts, this says
# exactly which and exits 78. It never skips quietly and never reports green - the failure this fleet
# keeps finding, most recently both mobile UNIT suites failing to COMPILE and reading as "no
# failures", because a module that does not run reports nothing.
#
# WHY arm64 IS CHECKED RATHER THAN ASSUMED
#
# The prover ships only as `arm64-v8a`/`armeabi-v7a`, so an x86_64 emulator cannot load it. It would
# fail deep inside the flow with an `UnsatisfiedLinkError` that reads like a product defect rather
# than like the wrong emulator, so the ABI is established up front and refused by name.
#
# WHY ANDROID_HOME IS RESOLVED RATHER THAN REQUIRED
#
# `adb` lives at ~/Library/Android/sdk on a normal macOS install while ANDROID_HOME is routinely
# unset, so demanding the variable would refuse a machine that is perfectly capable. It is resolved
# from the environment, then the conventional locations, and only then refused.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/e2e.sh
source "$ROOT_DIR/scripts/lib/e2e.sh"
E2E_SUITE="e2e-android"
cd "$ROOT_DIR"

readonly APP_ID="io.liberalize.dogtag"
readonly FLOW_DIR="apps/android/maestro"
FLOWS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --flow) FLOWS+=("$2"); shift 2 ;;
    -h|--help)
      printf 'usage: scripts/e2e-android.sh [--flow <file>]...\n'
      printf '  --flow  run only this flow (repeatable); default is every flow in %s\n' "$FLOW_DIR"
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

SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "$SDK_DIR" ]]; then
  for cand in "$HOME/Library/Android/sdk" "$HOME/Android/Sdk" /usr/local/lib/android/sdk; do
    [[ -d "$cand" ]] && { SDK_DIR="$cand"; break; }
  done
fi
[[ -n "$SDK_DIR" && -d "$SDK_DIR" ]] || could_not_run \
  "the Android SDK (ANDROID_HOME/ANDROID_SDK_ROOT unset, and none of the usual locations exist)" \
  "Install the Android SDK, or export ANDROID_HOME=/path/to/sdk."
export ANDROID_HOME="$SDK_DIR"
export ANDROID_SDK_ROOT="$SDK_DIR"
ADB="$SDK_DIR/platform-tools/adb"
[[ -x "$ADB" ]] || ADB="$(command -v adb 2>/dev/null || true)"
[[ -n "$ADB" && -x "$ADB" ]] || could_not_run \
  "\`adb\` (looked in $SDK_DIR/platform-tools and on PATH)" \
  "Install platform-tools: \`\$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager platform-tools\`."
info "Android SDK at $SDK_DIR"

command -v gradle >/dev/null 2>&1 || could_not_run \
  "\`gradle\` (the wrapper jar is gitignored by a global *.jar rule, so ./gradlew may be unusable)" \
  "Install Gradle 9.5.1 (\`brew install gradle\`), or regenerate the wrapper with \`gradle wrapper\`."

# `local.properties` is gitignored and Gradle refuses to configure without it. Writing it is safe and
# idempotent - it holds one line naming the SDK we just resolved - so create it rather than refusing
# for something we can supply ourselves.
if [[ ! -f apps/android/local.properties ]]; then
  info "writing apps/android/local.properties (gitignored) -> sdk.dir=$SDK_DIR"
  printf 'sdk.dir=%s\n' "$SDK_DIR" > apps/android/local.properties
fi

# The gitignored bundle inputs. A fresh checkout has none, and the `requireRoaxConfig` gradle check
# exists precisely so a missing bundle fails the BUILD rather than shipping an APK that builds clean
# and crashes on the first screen that reads an address.
for pair in \
  "apps/android/app/src/main/assets/consent_final.zkey|make vendor-mobile-artifacts" \
  "apps/android/app/src/main/assets/consent.graph|make vendor-mobile-artifacts" \
  "apps/android/app/src/main/assets/roax.json|make gen-mobile-config"
do
  f="${pair%%|*}"; fix="${pair##*|}"
  [[ -f "$f" ]] || could_not_run "$f (gitignored; a fresh checkout has none)" "Run \`$fix\`."
done

# The native prover. Gradle does NOT run cargo-ndk, so an absent jniLibs/ yields an APK that installs
# and then dies at the first FFI call.
if ! ls apps/android/app/src/main/jniLibs/arm64-v8a/*.so >/dev/null 2>&1; then
  could_not_run \
    "apps/android/app/src/main/jniLibs/arm64-v8a/*.so (the native prover; gitignored, and Gradle does NOT build it)" \
    "Run: cargo ndk -t arm64-v8a -t armeabi-v7a -o apps/android/app/src/main/jniLibs \\
                    build --release -p dogtag-standard-rs --features prover
             The \`--features prover\` is mandatory - the committed Kotlin bindings are checksum-verified
             at library load, so a default-feature build dies with UnsatisfiedLinkError before any test runs."
fi

if [[ ${#FLOWS[@]} -eq 0 ]]; then
  while IFS= read -r f; do FLOWS+=("$f"); done < <(find "$FLOW_DIR" -name '*.yaml' | sort)
fi
[[ ${#FLOWS[@]} -gt 0 ]] || could_not_run "any Maestro flow under $FLOW_DIR" "This checkout looks incomplete."
for f in "${FLOWS[@]}"; do
  [[ -f "$f" ]] || could_not_run "the flow file $f" "Check the path passed to --flow."
done
info "${#FLOWS[@]} flow(s) to run"

# ---------------------------------------------------------------------------------------------
# A device. This is the prerequisite most likely to be absent, and the one whose absence must never
# be mistaken for a pass.
# ---------------------------------------------------------------------------------------------
step "resolving a device"
"$ADB" start-server >/dev/null 2>&1 || true
DEVICES="$("$ADB" devices | awk 'NR>1 && $2=="device" {print $1}')"
if [[ -z "$DEVICES" ]]; then
  UNAUTH="$("$ADB" devices | awk 'NR>1 && $2=="unauthorized" {print $1}')"
  AVDS="$(ls "$HOME/.android/avd/"*.ini 2>/dev/null | xargs -n1 basename 2>/dev/null | sed 's/\.ini$//' | tr '\n' ' ' || true)"
  if [[ -n "$UNAUTH" ]]; then
    could_not_run "an AUTHORIZED Android device (found $UNAUTH, but it is unauthorized)" \
      "Unlock the device and accept the 'Allow USB debugging' prompt, then retry."
  fi
  # One branch or the other, never both: `${x:+a}` followed by `${x:-b}` prints the list twice when
  # it is non-empty, which reads like the script stuttering rather than like a considered message.
  if [[ -n "$AVDS" ]]; then
    avd_hint="AVDs on this machine: $AVDS"
  else
    avd_hint="No AVDs exist on this machine; create one in Android Studio (arm64-v8a system image)."
  fi
  could_not_run \
    "any connected Android device or running emulator (\`adb devices\` lists none)" \
    "Start an arm64 emulator and retry, e.g.
               \$ANDROID_HOME/emulator/emulator -avd <name> &
             $avd_hint"
fi
DEVICE="$(printf '%s\n' "$DEVICES" | head -1)"
COUNT="$(printf '%s\n' "$DEVICES" | wc -l | tr -d ' ')"
[[ "$COUNT" -gt 1 ]] && info "note: $COUNT devices attached; using the first ($DEVICE)"

# The ABI, established rather than assumed - see the header.
ABI="$("$ADB" -s "$DEVICE" shell getprop ro.product.cpu.abi 2>/dev/null | tr -d '\r' || true)"
case "$ABI" in
  arm64-v8a|armeabi-v7a) info "device $DEVICE (abi $ABI)" ;;
  "") could_not_run "a responsive device ($DEVICE did not answer getprop)" "Check \`$ADB -s $DEVICE shell echo ok\`." ;;
  *)  could_not_run "an ARM device or emulator (found $DEVICE with abi '$ABI')" \
        "The bundled prover ships only arm64-v8a/armeabi-v7a slices, so an x86_64 image cannot
             load it. Create an arm64-v8a AVD and retry." ;;
esac

# ---------------------------------------------------------------------------------------------
# Build + install
# ---------------------------------------------------------------------------------------------
step "building the DEBUG apk (the ZK self-test card is BuildConfig.DEBUG, so Release proves nothing)"
BUILD_LOG="$E2E_TMPDIR/gradle.log"
if ! ( cd apps/android && gradle --console=plain :app:assembleDebug ) >"$BUILD_LOG" 2>&1; then
  printf '\n--- last 40 lines of gradle ---\n' >&2
  tail -40 "$BUILD_LOG" >&2
  failed "the Android app did not build (log above). That is a code finding, not a missing
  prerequisite - every prerequisite was checked before this point."
fi
APK="apps/android/app/build/outputs/apk/debug/app-debug.apk"
# `BUILD SUCCESSFUL` is not evidence the artifact exists - the trap this whole file is about, in its
# Gradle flavour. Check for the file.
[[ -f "$APK" ]] || failed "gradle reported success but $APK is absent."
info "built $APK"

step "installing on $DEVICE"
"$ADB" -s "$DEVICE" install -r "$APK" >"$E2E_TMPDIR/install.log" 2>&1 || {
  tail -20 "$E2E_TMPDIR/install.log" >&2
  failed "adb install failed (log above)."
}
info "installed $APP_ID"

# ---------------------------------------------------------------------------------------------
# Run the flows
# ---------------------------------------------------------------------------------------------
step "running ${#FLOWS[@]} Maestro flow(s)"
REPORT="$E2E_TMPDIR/android-junit.xml"
set +e
# Bounded: a Maestro that hangs must become a loud failure rather than an indefinite wait. Measured
# here, not imagined - see `run_with_deadline`. 900s, because a cold emulator pays for Maestro's
# driver install AND the flow's own on-device Groth16 proof, which the flow itself waits 180s for.
MAESTRO_DEBUG="$E2E_TMPDIR/maestro-debug"
# `--debug-output` so the harness log lands somewhere WE can read: Maestro otherwise writes it
# under ~/.maestro/tests/<timestamp>/, and a driver failure prints only `[Failed] <flow> (0s)` on
# the console - the one line that cannot be told from a failed first assertion.
run_with_deadline 900 "maestro" maestro --udid "$DEVICE" test --format JUNIT --output "$REPORT" \
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

assert_ran "e2e-android" "$TESTS"

step "summary"
printf '  flows executed : %d\n' "$TESTS"
printf '  failures       : %d\n' "$FAILURES"
printf '  skipped        : %d\n' "$SKIPPED"
printf '  device         : %s (abi %s)\n' "$DEVICE" "$ABI"

[[ "$SKIPPED" -eq 0 ]] || failed "$SKIPPED flow(s) were SKIPPED. A skipped flow proves nothing and
  must never be counted as passing - investigate why the flow declined to run."
[[ "$FAILURES" -eq 0 && $MAESTRO_RC -eq 0 ]] || failed "$FAILURES flow(s) failed (maestro exit $MAESTRO_RC)."

printf '\ne2e-android: OK - %d flow(s) passed on %s.\n' "$TESTS" "$DEVICE"
