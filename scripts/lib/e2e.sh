#!/usr/bin/env bash
# Shared plumbing for the end-to-end launchers (`scripts/e2e-{web,ios,android}.sh`).
#
# WHY THIS EXISTS
#
# The e2e suites were unlaunchable. Playwright needed a portal somebody had already served; Maestro
# needed a device, an emulator and vendored artifacts. Every crew honestly reported "could not run"
# for both halves, so the suites were decoration rather than a gate. This library is the part all
# three launchers share, and almost all of it is about ONE property.
#
# THREE OUTCOMES, AND THE THIRD IS THE POINT
#
#   0   the suite RAN and every test passed
#   1   the suite RAN and something FAILED - a code finding
#   78  the suite DID NOT RUN - a prerequisite is genuinely absent
#
# A skipped suite reporting success is the defect this fleet keeps finding: most recently both mobile
# unit modules failed to COMPILE and read as "no failures", because a module that does not run reports
# nothing and nothing is indistinguishable from success. So `could_not_run` prints an unmissable
# banner, names the SPECIFIC missing prerequisite, and exits non-zero. It never exits 0.
#
# 78 rather than a fresh number, and the same "THE CHECK DID NOT RUN" banner, because
# `scripts/ensure-ts-prereqs.sh` already established that convention here. A second dialect for the
# same idea is how a caller comes to handle one and not the other.
#
# ZERO TESTS IS A FAILURE, NEVER "NOTHING TO RUN"
#
# Playwright exits 0 when a filter matches nothing, and a Gradle `BUILD SUCCESSFUL` can mean no test
# executed at all. Both are the same trap: a runner that trusts an exit code alone certifies a suite
# that never ran. `assert_ran` is what every launcher calls before reporting a pass, and it takes the
# count from a machine-readable report rather than from scraped console text.
#
# WHY TEARDOWN KILLS BY RECORDED PID ONLY
#
# `pkill -f target/release/government-api` has destroyed the captain's live service three separate
# times: this monorepo is checked out many times at once and every checkout builds the same binary to
# the same relative path, so the pattern matches whichever instance it happens to hit. Nothing here
# ever matches a process by name or path. `track_pid` records what we started, `stop_tracked` kills
# exactly those, and a `trap` runs it on every exit path including a Ctrl-C.

set -euo pipefail

readonly E2E_DID_NOT_RUN=78
readonly E2E_RULE='================================================================================'

# ---------------------------------------------------------------------------------------------
# Outcome reporting
# ---------------------------------------------------------------------------------------------

# The suite never executed, because a prerequisite this branch cannot supply is missing. Say plainly
# that NOTHING was verified, and name the specific thing plus how to get it - a generic "setup your
# environment" teaches the reader to wave the next one through.
could_not_run() {
  local what="$1" fix="${2:-}"
  printf '\n%s\n' "$E2E_RULE" >&2
  printf '%s: THE SUITE DID NOT RUN - a prerequisite is missing.\n' "${E2E_SUITE:-e2e}" >&2
  printf '  missing:   %s\n' "$what" >&2
  [[ -n "$fix" ]] && printf '  next step: %s\n' "$fix" >&2
  printf '  NOTHING was tested. This is not a pass and must never be reported as one.\n' >&2
  printf '%s\n' "$E2E_RULE" >&2
  exit "$E2E_DID_NOT_RUN"
}

# The suite ran and found a problem. Stated just as plainly, so the inverse mistake - burying a real
# defect as an environment blip - cannot happen either.
failed() {
  printf '\n%s\n' "$E2E_RULE" >&2
  printf '%s: FAILED - the suite ran and did not pass.\n' "${E2E_SUITE:-e2e}" >&2
  printf '  %s\n' "$1" >&2
  printf '%s\n' "$E2E_RULE" >&2
  exit 1
}

info() { printf '  %s\n' "$*"; }
step() { printf '\n[%s] %s\n' "${E2E_SUITE:-e2e}" "$*"; }

# A suite that executed zero tests is a FAILURE. `total` is read from the runner's own machine-
# readable report; a runner that cannot produce one has not proven it ran.
assert_ran() {
  local label="$1" total="${2:-}"
  if [[ -z "$total" || ! "$total" =~ ^[0-9]+$ ]]; then
    failed "$label reported no test count at all, so it cannot be shown to have run.
  A suite that cannot state how many tests executed has not proven it executed any."
  fi
  if [[ "$total" -eq 0 ]]; then
    failed "$label executed ZERO tests.
  That is a failure, never 'nothing to run': a filter that matches nothing, a spec that
  failed to compile, and a suite that genuinely has no cases are indistinguishable here,
  and the first two are defects."
  fi
}

# ---------------------------------------------------------------------------------------------
# Process lifecycle - by PID, never by name
# ---------------------------------------------------------------------------------------------

E2E_PIDS=()
E2E_TMPDIR=""
# A stable, gitignored ledger of what we started. bash defers a trap until the current FOREGROUND
# command returns, so a hard kill during a long `playwright test` can leave the servers behind - the
# trap is the normal path, this is the recovery one. Each line is `pid<TAB>command`, and `--cleanup`
# refuses to kill a pid whose command no longer matches, so a recycled pid cannot be hit.
E2E_PIDFILE="${E2E_PIDFILE:-}"

track_pid() {
  E2E_PIDS+=("$1")
  if [[ -n "$E2E_PIDFILE" ]]; then
    printf '%s\t%s\n' "$1" "$(ps -p "$1" -o command= 2>/dev/null || echo unknown)" >> "$E2E_PIDFILE"
  fi
}

# Kill what a PREVIOUS run left behind, from its own ledger. Identity is re-verified before each kill:
# a recorded pid whose command has changed belongs to something else now and is left alone.
cleanup_stale() {
  local file="$1" pid cmd now killed=0
  [[ -f "$file" ]] || { printf 'nothing recorded in %s - nothing to clean up.\n' "$file"; return 0; }
  while IFS=$'\t' read -r pid cmd; do
    [[ -z "${pid:-}" ]] && continue
    kill -0 "$pid" 2>/dev/null || { printf '  pid %s already gone\n' "$pid"; continue; }
    now="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    if [[ "$now" != "$cmd" ]]; then
      printf '  pid %s is NO LONGER what this ledger recorded - leaving it alone:\n    recorded: %s\n    now:      %s\n' \
        "$pid" "$cmd" "$now"
      continue
    fi
    printf '  stopping pid %s (%s)\n' "$pid" "${cmd:0:70}"
    kill -TERM "$pid" 2>/dev/null || true
    killed=$((killed+1))
  done < "$file"
  rm -f "$file"
  printf 'cleanup: stopped %d recorded process(es).\n' "$killed"
}

# Kill exactly what we started, in reverse order, escalating only if a process ignores TERM. Never
# `pkill -f`, never `killall`: see the header.
stop_tracked() {
  local pid i
  for (( i=${#E2E_PIDS[@]}-1 ; i>=0 ; i-- )); do
    pid="${E2E_PIDS[$i]}"
    [[ -z "$pid" ]] && continue
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
      local waited=0
      while kill -0 "$pid" 2>/dev/null && [[ $waited -lt 50 ]]; do
        perl -e 'select undef,undef,undef,0.1' 2>/dev/null || true
        waited=$((waited+1))
      done
      kill -0 "$pid" 2>/dev/null && kill -KILL "$pid" 2>/dev/null || true
    fi
  done
  E2E_PIDS=()
  [[ -n "$E2E_PIDFILE" ]] && rm -f "$E2E_PIDFILE"
  [[ -n "$E2E_TMPDIR" && -d "$E2E_TMPDIR" ]] && rm -rf "$E2E_TMPDIR"
  return 0
}

# Install teardown on every exit path. INT/TERM are listed explicitly: a Ctrl-C that left four vite
# servers holding their ports would make the next run fail on a port collision and look like a bug in
# the runner.
arm_teardown() {
  E2E_TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/dogtag-e2e.XXXXXX")"
  trap stop_tracked EXIT INT TERM
}

# ---------------------------------------------------------------------------------------------
# Ports
# ---------------------------------------------------------------------------------------------

# A port nothing is listening on. Checked rather than assumed: the captain's stack may be up, and
# these servers must never bind a port that is his.
port_is_free() { ! lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }

require_free_port() {
  local port="$1" what="$2"
  port_is_free "$port" || could_not_run \
    "port $port is already in use, and $what needs it" \
    "Something is listening there - possibly another run of this script, possibly a service you
             did not start. Identify it with 'lsof -nP -iTCP:$port -sTCP:LISTEN' and decide
             deliberately; this script will not kill a process it did not start."
}

# Wait for a server WE started to answer. Two failure modes are distinguished on purpose: a process
# that died (its log is the diagnosis) and one that is alive but never became ready (a hang).
#
# EVERY CANDIDATE URL IS PROBED IN THE SAME ITERATION, and that is not a nicety. Vite dev binds
# IPv6-only in some setups, so a `127.0.0.1` probe reports a perfectly healthy server as dead - a
# hazard already recorded in AGENTS.md. Probing sequentially with a fallback does not work here:
# `wait_for_http` EXITS on failure, so an `a || b` form never reaches `b` and simply burns the first
# timeout before dying. Both families have to be candidates in one loop, and the caller passes both.
#
# Usage: wait_for_http <pid> <label> <log> <timeout> <url> [url...]
wait_for_http() {
  local pid="$1" label="$2" log="$3" timeout="$4"; shift 4
  local urls=("$@") waited=0 u
  while [[ $waited -lt $timeout ]]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      printf '\n--- %s log ---\n' "$label" >&2
      tail -40 "$log" >&2 || true
      failed "$label exited before it became ready (see its log above)."
    fi
    for u in "${urls[@]}"; do
      if curl -fsS -o /dev/null --max-time 3 "$u" 2>/dev/null; then
        E2E_READY_URL="$u"
        return 0
      fi
    done
    sleep 1
    waited=$((waited+1))
  done
  printf '\n--- %s log ---\n' "$label" >&2
  tail -40 "$log" >&2 || true
  failed "$label never answered any of [${urls[*]}] within ${timeout}s (it is still running, so this is a hang)."
}

# ---------------------------------------------------------------------------------------------
# Shared prerequisite probes
# ---------------------------------------------------------------------------------------------

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || could_not_run "\`$1\` is not on PATH" "${2:-Install $1 and retry.}"
}

# ---------------------------------------------------------------------------------------------
# Telling a Maestro HARNESS failure apart from a flow failure
# ---------------------------------------------------------------------------------------------

# Maestro can fail before it runs a single assertion: its Android driver talks to an on-device gRPC
# server, and when that server does not come up the run reports `[Failed] <flow> (0s)` - which is
# indistinguishable, on the console, from a flow whose first assertion was false.
#
# Those are DIFFERENT ANSWERS and must not share an exit code. A flow failure is a finding about the
# product; a driver that never started is a finding about the machine, and reporting it as the former
# sends a reader hunting for a regression that does not exist. This is the same split
# `scripts/ensure-ts-prereqs.sh` makes between an environment failure and a code finding.
#
# NARROW ON PURPOSE, exactly as that script's is: only signatures that PROVE Maestro's own harness
# failed count, and the matched line is printed so a later misclassification is diagnosable from the
# log alone. Anything else stays a failure - stamping a real product break "environment" teaches a
# reader to wave the next one through, which is the worse error of the two.
readonly MAESTRO_HARNESS_SIGNATURES='Not able to reach the gRPC server|StatusRuntimeException: UNAVAILABLE|Unable to launch the (driver|app)|Failed to start (the )?(driver|xctest)'

# Print the matching line if Maestro's harness failed; return 1 if this looks like a real flow failure.
maestro_harness_failure() {
  local dir="$1" hit
  [[ -d "$dir" ]] || return 1
  hit="$(grep -rhoE "$MAESTRO_HARNESS_SIGNATURES.*" "$dir" 2>/dev/null | head -1)"
  [[ -n "$hit" ]] || return 1
  printf '%s\n' "$hit"
}

# ---------------------------------------------------------------------------------------------
# Watchdog
# ---------------------------------------------------------------------------------------------

# Run a command with a deadline, and stop it by RECORDED PID if it overruns.
#
# WHY THIS EXISTS - it was found by using the tool, not by imagining a case
#
# Maestro 2.6.1 hung on a cold Android emulator: it logged "Selected device emulator-5554", installed
# its driver, and then made no further progress for over half an hour. Without a deadline the runner
# waits with it, forever, printing nothing. That is the WORST outcome this library is built to
# prevent - not a wrong answer but NO answer, which a caller eventually kills by hand and records as
# "could not run" with nothing to show. A hang has to become a loud, bounded FAILURE.
#
# `timeout(1)` is deliberately not used: it is GNU coreutils and absent from a stock macOS, so relying
# on it would make the guard silently do nothing on the platform this repo is developed on.
#
# The kill is by the PID we spawned and its children FROM THE PROCESS TABLE - never `pkill -f
# maestro`, which on this machine would also match an unrelated JVM. Same rule as everywhere else here.
run_with_deadline() {
  local seconds="$1" label="$2"; shift 2
  "$@" &
  local pid=$! waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [[ $waited -ge $seconds ]]; then
      printf '\n  %s exceeded its %ds deadline - stopping it (pid %s).\n' "$label" "$seconds" "$pid" >&2
      local kid
      for kid in $(pgrep -P "$pid" 2>/dev/null); do kill -TERM "$kid" 2>/dev/null || true; done
      kill -TERM "$pid" 2>/dev/null || true
      sleep 2
      kill -KILL "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      return 124
    fi
    sleep 1
    waited=$((waited+1))
  done
  wait "$pid"
}
