#!/usr/bin/env bash
# Make the TypeScript workspace runnable before a gate check runs, or explain exactly why it is not.
#
# Why this exists
# ---------------
# @dogtag/ui resolves @dogtag/standard's types from the SDK's gitignored dist/, so a fresh
# checkout cannot typecheck until dependencies are installed and the SDK is built. The
# validation pipeline creates a fresh worktree per run, so without this the lint step fails
# with `ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL Command "tsc" not found`. That reads like an
# environmental blip, and approving past it signs off a lint step that never executed - a
# check that did not run counted as a check that passed.
#
# This script attempts the prerequisites, and when they cannot be satisfied it fails with the
# exact remedy and an unmissable statement that no checking happened.
#
# Usage: ensure-ts-prereqs.sh [--sdk-dist]
#   --sdk-dist  additionally build packages/dogtag-standard-ts/dist, required by any check
#               that resolves @dogtag/standard's types (e.g. `pnpm --filter @dogtag/ui typecheck`).
#
# Exit codes
#   0   prerequisites satisfied; any later failure is a code finding
#   78  prerequisites could NOT be satisfied; the check DID NOT RUN (environment)
#   1   the branch itself is broken (stale lockfile, SDK does not compile); a code finding
#   2   this script was called wrong

set -euo pipefail

readonly PREREQ_EXIT=78
readonly SDK_FILTER='@dogtag/standard'
readonly SDK_DIR='packages/dogtag-standard-ts'
readonly REMEDY='pnpm install --frozen-lockfile && pnpm --filter @dogtag/standard build'
readonly RULE='================================================================================'

# The check never executed. Name the remedy and say plainly that nothing was verified, so a
# reader cannot mistake this for a flaky code finding worth approving past.
prereq_failure() {
  printf '%s\n' "$RULE" >&2
  printf 'ts prereqs: THE CHECK DID NOT RUN - environment failure, NOT a code finding.\n' >&2
  printf '  cause:  %s\n' "$1" >&2
  printf '  remedy: %s\n' "$REMEDY" >&2
  printf '  Nothing was type-checked or tested. Do not approve past this as flaky: a green\n' >&2
  printf '  result is impossible here, and approving would sign off a check that never ran.\n' >&2
  printf '%s\n' "$RULE" >&2
  exit "$PREREQ_EXIT"
}

# The prerequisites ran and rejected the branch. This is a real finding; say so just as
# plainly, so the inverse mistake - burying a genuine defect as an environment blip - cannot
# happen either.
code_finding() {
  printf '%s\n' "$RULE" >&2
  printf 'ts prereqs: THIS IS A CODE FINDING, not an environment problem.\n' >&2
  printf '  cause:  %s\n' "$1" >&2
  printf '  fix:    %s\n' "$2" >&2
  printf '%s\n' "$RULE" >&2
  exit 1
}

want_sdk_dist=no
while [ "$#" -gt 0 ]; do
  case "$1" in
    --sdk-dist)
      want_sdk_dist=yes
      shift
      ;;
    *)
      printf 'ensure-ts-prereqs: unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || prereq_failure 'not inside a Git repository'
cd "$repo_root"

command -v pnpm >/dev/null 2>&1 ||
  prereq_failure 'pnpm is not on PATH (this repo pins pnpm via "packageManager" in package.json)'

install_log=$(mktemp)
trap 'rm -f "$install_log"' EXIT

# --frozen-lockfile is load-bearing beyond hygiene: it keeps the tracked pnpm-lock.yaml
# immutable. An install that rewrote the lockfile would dirty the worktree and fail the next
# run's document guard with a brand-new confusing error.
if ! pnpm install --frozen-lockfile 2>&1 | tee "$install_log"; then
  if grep -q 'ERR_PNPM_OUTDATED_LOCKFILE' "$install_log"; then
    code_finding \
      'pnpm-lock.yaml is out of date with the package.json files on this branch' \
      'run `pnpm install --lockfile-only` and commit the updated pnpm-lock.yaml'
  fi
  prereq_failure 'pnpm install --frozen-lockfile failed (see the install output above)'
fi

# Probe the exact artifact whose absence produced the original failure. A bare `[ -d node_modules ]`
# would not do: the workspace root keeps a node_modules/ holding only pnpm bookkeeping even when
# no package has been linked.
[ -x "$SDK_DIR/node_modules/.bin/tsc" ] ||
  prereq_failure "pnpm install reported success but $SDK_DIR/node_modules/.bin/tsc is still missing"

if [ "$want_sdk_dist" = yes ]; then
  # tsc is present, so a failure here is tsc rejecting the SDK source. Let its diagnostics
  # through unwrapped rather than relabelling a genuine type error as a missing prerequisite.
  if ! pnpm --filter "$SDK_FILTER" build; then
    code_finding \
      "$SDK_FILTER failed to compile; tsc diagnostics are above" \
      'fix the type errors in packages/dogtag-standard-ts/src'
  fi

  [ -f "$SDK_DIR/dist/index.d.ts" ] ||
    prereq_failure "$SDK_FILTER built without error but $SDK_DIR/dist/index.d.ts is missing"

  printf 'ts prereqs: ready - dependencies installed and %s dist built.\n' "$SDK_FILTER"
else
  printf 'ts prereqs: ready - dependencies installed.\n'
fi

printf 'ts prereqs: any failure after this line is a code finding, not an environment problem.\n'
