#!/usr/bin/env bash
# Make the TypeScript workspace runnable before a gate check runs, or explain exactly why it is not.
#
# Why this exists
# ---------------
# @dogtag/ui resolves @dogtag/standard's types from the SDK's gitignored dist/, so a fresh
# checkout cannot typecheck until dependencies are installed and the SDK is built. The
# validation pipeline creates a fresh worktree per run, so without this the lint step fails
# with `ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL Command "tsc" not found` and the test step with
# `vitest: command not found`. That reads like an environmental blip, and approving past it
# signs off a check that never executed - a check that did not run counted as a check that
# passed.
#
# How a failure is classified, and why the environment label is narrow
# -------------------------------------------------------------------
# Stamping a branch defect "environment" is the same defect pointed the other way: it teaches
# a reader to wave through a genuine break, and a canned remedy that cannot fix the cause
# confirms the wrong diagnosis when they follow it. So only what can be PROVEN environmental
# carries that label - a toolchain the branch cannot influence (no repository, no pnpm), and
# an install failure whose own output carries a network, registry, store, permission or disk
# signature. Each of those prints a cause-specific next step, never the command that just
# failed.
#
# Everything else is the branch's own defect and exits 1 as a code finding carrying pnpm's or
# tsc's real diagnostics: any other install failure, a stale lockfile, a binary still missing
# after install reported success (install succeeding proves the environment worked, so the
# devDependencies or the lockfile are implicated), and a missing dist/index.d.ts after the SDK
# build reported success (the SDK tsconfig no longer emits what @dogtag/ui resolves).
#
# Usage: ensure-ts-prereqs.sh [--sdk-dist]
#   --sdk-dist  additionally build packages/dogtag-standard-ts/dist, required by any check
#               that resolves @dogtag/standard's types (e.g. `pnpm --filter @dogtag/ui typecheck`).
#
# Exit codes
#   0   prerequisites satisfied; any later failure is a code finding
#   78  prerequisites could NOT be satisfied for a proven environmental reason; the check DID NOT RUN
#   1   the branch itself is broken; a code finding
#   2   this script was called wrong

set -euo pipefail

readonly PREREQ_EXIT=78
readonly SDK_FILTER='@dogtag/standard'
readonly SDK_DIR='packages/dogtag-standard-ts'
readonly RULE='================================================================================'

# The only install failures that may be called environmental. Kept to signatures that name a
# network, registry, store, permission or disk fault, because anything broader would let a
# branch-caused install error borrow the "did not run" label.
readonly ENV_INSTALL_SIGNATURES='ERR_PNPM_NO_OFFLINE_TARBALL|ERR_PNPM_META_FETCH_FAIL|ENOTFOUND|EAI_AGAIN|ECONNREFUSED|ETIMEDOUT|EACCES|ENOSPC'

# The check never executed, for a reason this branch cannot have caused. Say plainly that
# nothing was verified, and name a next step that addresses the cause rather than re-running
# whatever just failed.
prereq_failure() {
  printf '%s\n' "$RULE" >&2
  printf 'ts prereqs: THE CHECK DID NOT RUN - environment failure, NOT a code finding.\n' >&2
  printf '  cause:     %s\n' "$1" >&2
  printf '  next step: %s\n' "$2" >&2
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

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  prereq_failure \
    'not inside a Git repository (or git is not on PATH)' \
    'run this from a checkout of the repository, with git on PATH'
cd "$repo_root"

command -v pnpm >/dev/null 2>&1 ||
  prereq_failure \
    'pnpm is not on PATH' \
    'install the pnpm version pinned by the "packageManager" field in package.json (corepack enable && corepack prepare --activate)'

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

  # Name the signature that earned the environment label, so a later misclassification is
  # diagnosable from this log alone instead of by re-deriving the pattern.
  env_signature=$(grep -oE "$ENV_INSTALL_SIGNATURES" "$install_log" | head -n 1 || true)
  if [ -n "$env_signature" ]; then
    prereq_failure \
      "pnpm install --frozen-lockfile failed with the environmental signature $env_signature (install output above)" \
      'restore registry/store access - network, credentials, disk space - and re-run this check'
  fi

  code_finding \
    'pnpm install --frozen-lockfile failed with no environmental signature, so the dependency graph on this branch is what rejected it; pnpm diagnostics are above' \
    'fix the package.json / pnpm-lock.yaml error pnpm reported above (e.g. a workspace:* dependency naming a package that does not exist)'
fi

# Probe the binary the CALLING command actually needs, which is also the exact artifact whose
# absence produced that caller's original cold-worktree failure: `tsc not found` for the
# --sdk-dist caller (commands.lint) and `vitest: command not found` for the no-flag caller
# (commands.test). A bare `[ -d node_modules ]` would not do: the workspace root keeps a
# node_modules/ holding only pnpm bookkeeping even when no package has been linked.
# Install having succeeded proves the environment worked, so a still-missing binary implicates
# this branch's devDependencies or lockfile and is reported as such.
require_installed_bin() {
  [ -x "$SDK_DIR/node_modules/.bin/$1" ] ||
    code_finding \
      "pnpm install --frozen-lockfile reported success but $SDK_DIR/node_modules/.bin/$1 is still missing" \
      "restore $1 to $SDK_DIR's devDependencies and commit the regenerated pnpm-lock.yaml"
}

if [ "$want_sdk_dist" = yes ]; then
  require_installed_bin tsc

  # tsc is present, so a failure here is tsc rejecting the SDK source. Let its diagnostics
  # through unwrapped rather than relabelling a genuine type error as a missing prerequisite.
  if ! pnpm --filter "$SDK_FILTER" build; then
    code_finding \
      "$SDK_FILTER failed to compile; tsc diagnostics are above" \
      'fix the type errors in packages/dogtag-standard-ts/src'
  fi

  # The build reported success, so this is the SDK's tsconfig no longer emitting the
  # declarations @dogtag/ui resolves - a branch change, and one that would make
  # `pnpm --filter @dogtag/ui typecheck` genuinely fail.
  [ -f "$SDK_DIR/dist/index.d.ts" ] ||
    code_finding \
      "$SDK_FILTER built without error but $SDK_DIR/dist/index.d.ts is missing" \
      "restore declaration output and the dist/ outDir in $SDK_DIR/tsconfig.json"

  printf 'ts prereqs: ready - dependencies installed and %s dist built.\n' "$SDK_FILTER"
else
  require_installed_bin vitest

  printf 'ts prereqs: ready - dependencies installed.\n'
fi

printf 'ts prereqs: any failure after this line is a code finding, not an environment problem.\n'
