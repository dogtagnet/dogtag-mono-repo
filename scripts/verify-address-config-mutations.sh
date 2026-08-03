#!/usr/bin/env bash
# Mutation gate for the addresses-as-configuration properties.
#
#   bash scripts/verify-address-config-mutations.sh    (or: make verify-address-config-mutations)
#
# WHY THIS EXISTS
#
# The claims this branch makes are all of the form "an unconfigured X refuses instead of quietly
# using a baked-in address". That shape is trivially easy to assert and trivially easy to leave
# unpinned, because the happy path passes either way: a test that only ever runs with everything
# configured never notices when a literal default creeps back in. So each property is checked by
# BREAKING it and requiring a specific named test to go red.
#
# A MUTATION THAT DOES NOT APPLY IS REPORTED AS **INERT**, NOT AS A PASS. That distinction is the
# whole point and this repo has shipped the mistake it prevents: a mutation whose scrutinee had moved
# silently matched nothing, its "red" was a build failure rather than an assertion failure, and it was
# counted as evidence. Every mutation below asserts its scrutinee is PRESENT first, and a run in which
# any mutation is inert exits non-zero.
#
# THE INJECTED LITERALS ARE SYNTHETIC, and must stay so. `make check-addresses` watches THIS FILE
# too - it caught this harness carrying real retired addresses on its first run, which is the gate
# working. What each mutation demonstrates is that A LITERAL crept back in; whether that literal is
# a real deployment is irrelevant to the property and would put real addresses back in the tree.
#
# COMMIT BEFORE RUNNING. Restoration is `git checkout --`, so uncommitted work in a target file is
# DESTROYED - a hazard already recorded in AGENTS.md, and one that has bitten this repo.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! git diff --quiet -- packages/ui stacks/indexer/api; then
  echo "::error::uncommitted changes in a target directory. This harness restores with 'git checkout --'," >&2
  echo "          which would DESTROY them. Commit first." >&2
  exit 1
fi

pass=0; fail=0; inert=0

# apply <file> <old> <new> — replace exactly once, refusing if the scrutinee is absent.
apply() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path, encoding='utf-8').read()
if old not in s:
    sys.exit(3)
open(path, 'w', encoding='utf-8').write(s.replace(old, new, 1))
PY
}

# mutate <label> <file> <old> <new> <runner> <test-name>
mutate() {
  local label="$1" file="$2" old="$3" new="$4" runner="$5" test="$6"
  printf '%-58s' "$label"
  if ! apply "$file" "$old" "$new"; then
    echo "INERT (scrutinee absent - this pins NOTHING)"
    inert=$((inert + 1))
    return
  fi
  local out
  out="$("$runner" "$test" 2>&1)"
  local rc=$?
  git checkout -- "$file"
  if [ $rc -eq 0 ]; then
    echo "NOT PINNED (mutation applied, '$test' still green)"
    fail=$((fail + 1))
  elif printf '%s' "$out" | grep -qiE "error\[E[0-9]+\]|could not compile|Parse failure|SyntaxError"; then
    # A build break is not evidence about the assertion - it is a broken mutation.
    echo "INERT (did not compile - its red is a build failure, not an assertion)"
    inert=$((inert + 1))
  else
    echo "pinned by $test"
    pass=$((pass + 1))
  fi
}

run_cargo_indexer() { cargo test -p indexer-api --lib -- --exact "$1"; }
run_vitest_ui() { pnpm --filter @dogtag/ui exec vitest run -t "$1"; }

echo "=== addresses-as-configuration mutation gate ==="
echo

mutate "M1 unconfigured live indexer gets a baked-in triple" \
  stacks/indexer/api/src/app.rs \
  'let demo_generation = demo_generation.ok_or_else(|| {' \
  'let demo_generation = demo_generation.or_else(|| watch_generation(
        "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
        "0x2B4d6f8a0c1e3a5b7d9f0e2C4a6b8d0F1E3A5c70",
        "0x3c5e7A9b0D2F4a6c8E0b1d3F5A7c9e0B2D4F6a80",
        vec![],
    ).ok()).ok_or_else(|| {' \
  run_cargo_indexer \
  app::tests::an_unconfigured_live_instance_refuses_to_start_rather_than_watching_a_baked_in_triple

mutate "M2 legacy singleton defaults the omitted keys" \
  stacks/indexer/api/src/app.rs \
  'value.clone().ok_or_else(|| {' \
  'return Ok(value.clone().unwrap_or_else(|| "0x0000000000000000000000000000000000000000".to_string()));
        #[allow(unreachable_code)]
        value.clone().ok_or_else(|| {' \
  run_cargo_indexer \
  app::tests::a_partial_legacy_singleton_configuration_names_the_missing_variable

mutate "M3 contracts.ts reinstates a literal default" \
  packages/ui/src/wallet/contracts.ts \
  'ProviderRegistry: configured("VITE_PROVIDER_REGISTRY_ADDR"),' \
  'ProviderRegistry: configured("VITE_PROVIDER_REGISTRY_ADDR") || "0x4d6F8B0C2E4A6b8d0F2c4e6a8b0D2F4c6e8A0b90",' \
  run_vitest_ui \
  "holds no address of its own"

mutate "M4 verifier drops the unconfigured-factory guard" \
  packages/ui/src/wallet/verifyCredential.ts \
  '  if (!isConfiguredAddress(factoryAddr)) {' \
  '  if (false && !isConfiguredAddress(factoryAddr)) {' \
  run_vitest_ui \
  "refuses by name"

mutate "M5 verifier falls back to the document's documentStore" \
  packages/ui/src/wallet/verifyCredential.ts \
  '  if (!isConfiguredAddress(factoryAddr)) {
    throw new Error(' \
  '  if (!isConfiguredAddress(factoryAddr)) {
    factoryAddr = doc.issuer.documentStore as `0x${string}`;
  }
  if (false) {
    throw new Error(' \
  run_vitest_ui \
  "does NOT fall back"

echo
echo "pinned: $pass   NOT pinned: $fail   inert: $inert"
if [ "$fail" -ne 0 ] || [ "$inert" -ne 0 ]; then
  echo "::error::a mutation was unpinned or inert - neither is a pass." >&2
  exit 1
fi
git diff --quiet || { echo "::error::the tree was left mutated" >&2; exit 1; }
echo "all mutations pinned, and the tree is clean."
