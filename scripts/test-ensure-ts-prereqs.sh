#!/usr/bin/env bash
# Contract checks for the TypeScript prerequisite step used by the lint and test commands.
#
# The property under test is legibility, not convenience: when the prerequisites cannot be
# satisfied the step must state that the check DID NOT RUN and name the remedy, so nobody
# reads it as a flaky code finding and approves past a check that never executed. These cases
# stay offline and deterministic - the success path is exercised by every gate run.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
prereqs="$repo_root/scripts/ensure-ts-prereqs.sh"
bash_bin=$(command -v bash)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/dogtag-tsprereq.XXXXXX")
trap 'rm -rf "$test_root"' EXIT

fail() {
  printf 'not ok - %s\n' "$*" >&2
  exit 1
}

pass() {
  printf 'ok - %s\n' "$*"
}

# Run the prerequisite script under an optional command prefix, capturing status and combined
# output without tripping `set -e`. bash is addressed absolutely so a stripped PATH still works.
run_prereqs() {
  set +e
  run_output=$(cd "$repo_root" && "$@" "$bash_bin" "$prereqs" 2>&1)
  run_status=$?
  set -e
}

expect_contains() {
  local expected=$1
  case "$run_output" in
    *"$expected"*) ;;
    *) fail "missing '$expected' in: $run_output" ;;
  esac
}

# An unsatisfiable prerequisite must not exit 0 (silently unchecked) and must not exit 1,
# which the gate presents as a code finding. 78 keeps "the environment is unusable" a
# distinct answer from both.
shim="$test_root/bin-without-pnpm"
mkdir -p "$shim"
ln -s "$(command -v git)" "$shim/git"
run_prereqs env "PATH=$shim"
[ "$run_status" -eq 78 ] || fail "unsatisfiable prerequisites must exit 78 (got $run_status: $run_output)"
expect_contains 'THE CHECK DID NOT RUN'
expect_contains 'NOT a code finding'
expect_contains 'pnpm install --frozen-lockfile && pnpm --filter @dogtag/standard build'
expect_contains 'Do not approve past this'
pass 'unsatisfiable prerequisites fail closed with the remedy, not a code finding'

set +e
usage_output=$(bash "$prereqs" --not-a-real-flag 2>&1)
usage_status=$?
set -e
[ "$usage_status" -eq 2 ] || fail "unknown arguments must exit 2 (got $usage_status: $usage_output)"
pass 'unknown arguments are rejected rather than silently ignored'

# --frozen-lockfile is what keeps the tracked pnpm-lock.yaml immutable. An install able to
# rewrite it would dirty the worktree and fail the next run's Document guard with a fresh
# confusing error, recreating the ambiguity this step exists to remove.
executed_installs=$(grep -nE '^[[:space:]]*(if[[:space:]]+!?[[:space:]]*)?pnpm install' "$prereqs" || true)
[ -n "$executed_installs" ] || fail 'ensure-ts-prereqs.sh no longer runs `pnpm install` at all'
if printf '%s\n' "$executed_installs" | grep -qv -- '--frozen-lockfile'; then
  fail "ensure-ts-prereqs.sh invokes \`pnpm install\` without --frozen-lockfile: $executed_installs"
fi
pass 'the prerequisite install cannot rewrite the tracked lockfile'
