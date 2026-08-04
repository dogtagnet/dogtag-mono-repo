#!/usr/bin/env bash
# Behaviour tests for scripts/lib/e2e.sh - the two guarantees the e2e runners rest on that are NOT
# exercised by an ordinary passing run, and so would otherwise be assumptions.
#
#   1. `run_with_deadline` stops a hang and reports it as a hang, leaving no child behind.
#   2. `maestro_harness_failure` tells Maestro's own driver failing to start apart from a flow that
#      genuinely failed - and does NOT claim a plain flow failure is environmental.
#
# WHY EACH OF THESE NEEDS A TEST RATHER THAN A CODE REVIEW
#
# Both only run on a path a green run never touches. The classifier in particular was written from a
# real captured failure and then never fired, because every subsequent Android run hit the deadline
# instead - so the branch whose whole job is separating could-not-run from a code finding had zero
# evidence behind it. A guard nobody has seen work is a guard nobody should trust.
#
# The signature fixtures below are TRANSCRIBED FROM REAL MAESTRO OUTPUT
# (`~/.maestro/tests/<ts>/maestro.log`, Maestro 2.6.1 against an arm64 AVD), not invented, because a
# fixture written to match the implementation proves only that the implementation matches itself.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/e2e.sh
source "$ROOT_DIR/scripts/lib/e2e.sh"
E2E_SUITE="e2e-lib-test"

TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dogtag-e2elib.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT

fail_case() { printf 'not ok - %s\n' "$*" >&2; exit 1; }
pass_case() { printf 'ok - %s\n' "$*"; }

# ---------------------------------------------------------------------------------------------
# 1. The watchdog
# ---------------------------------------------------------------------------------------------
set +e
run_with_deadline 10 "quick" sleep 1
rc=$?
set -e
[[ $rc -eq 0 ]] || fail_case "a command that finishes in time should return its own status (got $rc)"
pass_case "a command that finishes within the deadline returns its own status"

set +e
run_with_deadline 2 "hanger" sleep 300 2>/dev/null
rc=$?
set -e
[[ $rc -eq 124 ]] || fail_case "a hanging command must report 124, not $rc"
pass_case "a hanging command is stopped and reported as a hang (124), never as a pass"

# The child must be REAPED. A watchdog that returns while leaving the process running would free the
# runner to exit and report, with the real work still going - the worst of both outcomes.
sleep 1
if pgrep -f "sleep 300" >/dev/null 2>&1; then
  fail_case "the watchdog left its child running after the deadline"
fi
pass_case "the stopped command leaves no child behind"

# ---------------------------------------------------------------------------------------------
# 2. The Maestro harness classifier
# ---------------------------------------------------------------------------------------------

# REAL output, from a run where Maestro's Android driver never came up. The console said only
# `[Failed] zk_e2e (0s)` - indistinguishable from a false first assertion, which is the whole reason
# this classifier reads the debug log instead.
real_driver_failure="$TEST_ROOT/driver"; mkdir -p "$real_driver_failure"
cat > "$real_driver_failure/maestro.log" <<'LOG'
09:54:19.205 [ERROR] maestro.drivers.AndroidDriver.runDeviceCall: Not able to reach the gRPC server while processing deviceInfo command
09:54:19.205 [ERROR] maestro.cli.runner.TestSuiteInteractor.runFlow: Failed to complete flow
io.grpc.StatusRuntimeException: UNAVAILABLE
Caused by: java.io.IOException: Command failed (tcp:51697): closed
LOG
if hit="$(maestro_harness_failure "$real_driver_failure")"; then
  case "$hit" in
    *"gRPC server"*) pass_case "a driver that never started is classified as a harness failure, quoting the line" ;;
    *) fail_case "classified, but the quoted line is not the diagnostic one: $hit" ;;
  esac
else
  fail_case "a real captured driver failure was NOT classified - the signature set has drifted"
fi

# THE OTHER DIRECTION, and the one that matters more. Stamping a genuine product break "environment"
# teaches a reader to wave the next one through, so an ordinary failed assertion must NOT match.
flow_failure="$TEST_ROOT/flow"; mkdir -p "$flow_failure"
cat > "$flow_failure/maestro.log" <<'LOG'
10:02:11.500 [ INFO] maestro.cli.runner.TestSuiteInteractor.runFlow: Running flow zk_e2e
10:02:41.802 [ERROR] maestro.cli.runner.TestSuiteInteractor.runFlow: Failed to complete flow
Assertion is false: "ZK-SELFTEST: PASS" is visible
LOG
if maestro_harness_failure "$flow_failure" >/dev/null; then
  fail_case "a plain failed ASSERTION was misclassified as an environment failure - the set is too broad"
fi
pass_case "a failed assertion stays a code finding and is never called environmental"

# An absent directory is not a harness failure either: it means we have nothing to go on, and
# inventing a verdict from no evidence is the defect this whole library is about.
if maestro_harness_failure "$TEST_ROOT/does-not-exist" >/dev/null; then
  fail_case "a MISSING debug directory was reported as a harness failure - that is a verdict from no evidence"
fi
pass_case "a missing debug directory yields no verdict"

# ---------------------------------------------------------------------------------------------
# 3. assert_ran - zero tests is a failure, and so is a missing count
# ---------------------------------------------------------------------------------------------
( assert_ran "suite" 0 ) >/dev/null 2>&1 && fail_case "assert_ran accepted ZERO tests as a pass"
pass_case "a suite reporting zero tests is a failure"
( assert_ran "suite" "" ) >/dev/null 2>&1 && fail_case "assert_ran accepted an ABSENT count as a pass"
pass_case "a suite that cannot state a count is a failure"
( assert_ran "suite" 3 ) >/dev/null 2>&1 || fail_case "assert_ran rejected a real count"
pass_case "a suite reporting a real count passes"

printf '\ne2e-lib: all cases passed\n'
