#!/usr/bin/env bash
# Contract checks for the TypeScript prerequisite step used by the lint and test commands.
#
# The property under test is legibility, not convenience. Two mistakes are equally bad and the
# step must make both impossible: reporting an unsatisfiable prerequisite as a code finding
# (which buries the fact that nothing ran), and reporting a branch defect as an environment
# failure (which teaches a reader to wave through a genuine break, with a remedy that cannot
# fix the cause). So these cases pin the CLASSIFICATION of each failure, not just that it
# fails. They stay offline and deterministic by driving the script inside a throwaway git
# repository with a scripted pnpm on PATH; the real install path is exercised by every gate run.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
prereqs="$repo_root/scripts/ensure-ts-prereqs.sh"
bash_bin=$(command -v bash)
sdk_rel='packages/dogtag-standard-ts'
canned_remedy='pnpm install --frozen-lockfile && pnpm --filter @dogtag/standard build'
test_root=$(mktemp -d "${TMPDIR:-/tmp}/dogtag-tsprereq.XXXXXX")
trap 'rm -rf "$test_root"' EXIT

fail() {
  printf 'not ok - %s\n' "$*" >&2
  exit 1
}

pass() {
  printf 'ok - %s\n' "$*"
}

# Run the prerequisite script from a working directory with an explicit PATH, capturing status
# and combined output without tripping `set -e`. bash is addressed absolutely so a stripped
# PATH still works.
run_prereqs() {
  local workdir=$1 path_value=$2
  shift 2
  set +e
  run_output=$(cd "$workdir" && env "PATH=$path_value" "$bash_bin" "$prereqs" "$@" 2>&1)
  run_status=$?
  set -e
}

expect_contains() {
  case "$run_output" in
    *"$1"*) ;;
    *) fail "missing '$1' in: $run_output" ;;
  esac
}

expect_absent() {
  case "$run_output" in
    *"$1"*) fail "unexpected '$1' in: $run_output" ;;
  esac
}

expect_status() {
  [ "$run_status" -eq "$1" ] || fail "$2 (got exit $run_status: $run_output)"
}

# A throwaway workspace shaped just enough for the script to reach its probes.
new_fake_repo() {
  local repo="$test_root/repo-$1"
  mkdir -p "$repo/$sdk_rel"
  git -C "$repo" init -q -b main
  git -C "$repo" config user.name 'Dogtag prereq test'
  git -C "$repo" config user.email 'prereq-test@dogtag.invalid'
  printf '{"name":"fake-root"}\n' > "$repo/package.json"
  git -C "$repo" add package.json
  git -C "$repo" commit -q -m 'test: base'
  printf '%s\n' "$repo"
}

# Place a scripted pnpm ahead of the real one. Anything the shim does not recognize succeeds,
# so a later change to the script cannot turn into a confusing shim failure.
new_pnpm_shim() {
  local dir="$test_root/bin-$1"
  mkdir -p "$dir"
  {
    printf '#!/usr/bin/env bash\n'
    printf 'if [ "$1" = install ]; then\n'
    printf '%s\n' "$2"
    printf 'fi\n'
    printf 'exit 0\n'
  } > "$dir/pnpm"
  chmod +x "$dir/pnpm"
  printf '%s:%s\n' "$dir" "$PATH"
}

make_bin_stub() {
  mkdir -p "$1/$sdk_rel/node_modules/.bin"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$1/$sdk_rel/node_modules/.bin/$2"
  chmod +x "$1/$sdk_rel/node_modules/.bin/$2"
}

# --- a missing toolchain the branch cannot influence is the environment case ---------------
#
# An unsatisfiable prerequisite must not exit 0 (silently unchecked) and must not exit 1,
# which the gate presents as a code finding. 78 keeps "the environment is unusable" a
# distinct answer from both.
shim="$test_root/bin-without-pnpm"
mkdir -p "$shim"
ln -s "$(command -v git)" "$shim/git"
run_prereqs "$repo_root" "$shim"
expect_status 78 'a missing toolchain must exit 78'
expect_contains 'THE CHECK DID NOT RUN'
expect_contains 'NOT a code finding'
expect_contains 'packageManager'
expect_contains 'Do not approve past this'
pass 'a missing toolchain fails closed as environment, with a next step that installs it'

# --- an install failure is environmental ONLY when its output proves it ---------------------
path=$(new_pnpm_shim env-signature "printf 'ERR_PNPM_META_FETCH_FAIL GET https://registry.npmjs.org/: getaddrinfo ENOTFOUND registry.npmjs.org\\n' >&2; exit 1")
run_prereqs "$(new_fake_repo env-install)" "$path"
expect_status 78 'an install failure carrying a network signature must exit 78'
expect_contains 'THE CHECK DID NOT RUN'
expect_contains 'ERR_PNPM_META_FETCH_FAIL'
# The next step must address the cause. Re-printing the command that just failed would make a
# reader who follows it fail again and conclude the diagnosis was right.
expect_absent "$canned_remedy"
pass 'a network/registry install failure is environment, named by its signature'

path=$(new_pnpm_shim branch-install "printf 'ERR_PNPM_WORKSPACE_PKG_NOT_FOUND In packages/ui: \"@dogtag/nope@workspace:*\" is in the dependencies but no package named \"@dogtag/nope\" is in the workspace\\n' >&2; exit 1")
run_prereqs "$(new_fake_repo branch-install)" "$path"
expect_status 1 'an install failure with no environmental signature must be a code finding'
expect_contains 'THIS IS A CODE FINDING'
expect_absent 'THE CHECK DID NOT RUN'
expect_absent "$canned_remedy"
pass 'a branch-caused install failure is a code finding, not an environment blip'

path=$(new_pnpm_shim stale-lockfile "printf 'ERR_PNPM_OUTDATED_LOCKFILE Cannot install with \"frozen-lockfile\" because pnpm-lock.yaml is not up to date\\n' >&2; exit 1")
run_prereqs "$(new_fake_repo stale-lockfile)" "$path"
expect_status 1 'a stale lockfile must be a code finding'
expect_contains 'pnpm-lock.yaml is out of date'
pass 'a stale lockfile is reported as the branch defect it is'

# --- install succeeded, so anything still missing implicates the branch ----------------------
#
# A successful install proves the environment worked. An absent binary after it can only come
# from this branch's devDependencies or lockfile, so it must never borrow the environment label.
path=$(new_pnpm_shim install-ok ':')
run_prereqs "$(new_fake_repo missing-tsc)" "$path" --sdk-dist
expect_status 1 'a post-install missing tsc must be a code finding'
expect_contains 'THIS IS A CODE FINDING'
expect_contains "$sdk_rel/node_modules/.bin/tsc is still missing"
expect_absent 'THE CHECK DID NOT RUN'
pass 'tsc missing after a successful install is a code finding (--sdk-dist path)'

run_prereqs "$(new_fake_repo missing-vitest)" "$path"
expect_status 1 'a post-install missing vitest must be a code finding'
expect_contains "$sdk_rel/node_modules/.bin/vitest is still missing"
expect_absent 'THE CHECK DID NOT RUN'
pass 'vitest missing after a successful install is a code finding (no-flag path)'

# The no-flag caller is `commands.test`, whose original cold symptom was `vitest: command not
# found`, so the probe has to cover that caller's own artifact and not only tsc.
repo=$(new_fake_repo vitest-only)
make_bin_stub "$repo" vitest
run_prereqs "$repo" "$path"
expect_status 0 'the no-flag path must succeed once vitest is installed'
expect_contains 'ts prereqs: ready'
expect_contains 'any failure after this line is a code finding'
pass 'the no-flag path probes the artifact its caller actually needs'

# --- the SDK built, so a missing declaration is a tsconfig change on this branch ------------
repo=$(new_fake_repo missing-dist)
make_bin_stub "$repo" tsc
run_prereqs "$repo" "$path" --sdk-dist
expect_status 1 'a post-build missing dist/index.d.ts must be a code finding'
expect_contains 'THIS IS A CODE FINDING'
expect_contains "$sdk_rel/dist/index.d.ts is missing"
expect_absent 'THE CHECK DID NOT RUN'
pass 'a build that emits no declarations is a code finding, not a missing prerequisite'

repo=$(new_fake_repo sdk-dist-ok)
make_bin_stub "$repo" tsc
mkdir -p "$repo/$sdk_rel/dist"
printf 'export {};\n' > "$repo/$sdk_rel/dist/index.d.ts"
run_prereqs "$repo" "$path" --sdk-dist
expect_status 0 'the --sdk-dist path must succeed once the declarations exist'
expect_contains 'ts prereqs: ready'
pass 'the --sdk-dist path succeeds when its prerequisites are genuinely satisfied'

# --- calling conventions and the lockfile guarantee -----------------------------------------
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
