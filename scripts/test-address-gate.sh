#!/usr/bin/env bash
# Behaviour tests for scripts/check-no-hardcoded-addresses.sh.
#
# THE CLAIM UNDER TEST, and why it needs a test at all
#
# The gate's retired-address set used to be hand-curated, which inverted its guarantee: a redeploy
# REPLACES addresses in the ledger in place, so every superseded address dropped out of the pattern at
# the exact moment it became wrong. The gate was strongest while a pasted address was still correct
# and blind once it was stale - the failure its own header is about, reproduced inside it.
#
# The fix derives that set from the ledger's git history. `the_widening_is_load_bearing` is the case
# that proves the fix is doing the work: it runs a MUTATED copy of the gate with the history read
# removed and asserts that copy reports OK on a tree the real gate refuses. Without that case, every
# other assertion here would pass just as happily against the unfixed script.
#
# WHY EVERY CASE BUILDS ITS OWN THROWAWAY REPO
#
# Never mutate the working tree. AGENTS.md records that a mutation harness restoring with
# `git checkout --` destroyed uncommitted work in this repo and then reported every later mutation as
# INERT, which reads like a harness bug rather than like your work having just been deleted. Each case
# here builds a repo under `mktemp -d` and the gate is copied into it, so there is nothing of yours to
# lose and no restore step to get wrong.
#
# WHY THE MUTATION SELF-TESTS
#
# A mutation that fails to apply leaves the script unchanged, the assertion passes, and an UNPINNED
# claim reads exactly like a pinned one. So the scrutinee is asserted present before the edit and
# absent after; either way round it reports INERT and exits non-zero rather than counting a green.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$REPO_ROOT/scripts/check-no-hardcoded-addresses.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dogtag-addrgate.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT

[[ -f "$GATE" ]] || { echo "not ok - gate script not found: $GATE" >&2; exit 1; }

fail() { printf 'not ok - %s\n' "$*" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$*"; }

# Addresses used by the fixtures. Deliberately outside anything this repo ever deployed: a fixture
# that reused a live address would pass for the wrong reason the day the ledger moved.
OLD_ADDR="0xAaAaAaAa11111111111111111111111111111111"   # published, then superseded by a redeploy
NEW_ADDR="0xBbBbBbBb22222222222222222222222222222222"   # the current live set
ANVIL_ADDR="0xCcCcCcCc33333333333333333333333333333333" # only ever on a local dev chain

# Build a repo whose ledger has real history: revision 1 publishes $OLD_ADDR, revision 2 supersedes it
# with $NEW_ADDR. That is exactly the shape a redeploy produces, and the shape the old gate went blind
# on. `$1` names the repo; any extra args are files to create carrying $OLD_ADDR.
new_repo() {
  local name="$1"; shift
  local repo="$TEST_ROOT/$name"
  mkdir -p "$repo/scripts" "$repo/contracts/deployments" "$repo/src"
  git -C "$repo" init -q -b main
  git -C "$repo" config user.name 'Dogtag address gate test'
  git -C "$repo" config user.email 'addrgate-test@dogtag.invalid'
  cp "$GATE" "$repo/scripts/check-no-hardcoded-addresses.sh"

  printf '{\n  "Registry": "%s",\n  "chainId": 135\n}\n' "$OLD_ADDR" \
    > "$repo/contracts/deployments/roax.json"
  printf '{"retired": [], "stillHardcoded": {}}\n' > "$repo/scripts/address-debt.json"
  git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: first set'

  printf '{\n  "Registry": "%s",\n  "chainId": 135\n}\n' "$NEW_ADDR" \
    > "$repo/contracts/deployments/roax.json"
  git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: redeploy, superseding the first set'

  local f
  for f in "$@"; do
    mkdir -p "$repo/$(dirname "$f")"
    printf 'export const ADDR = "%s";\n' "$OLD_ADDR" > "$repo/$f"
  done
  if [[ $# -gt 0 ]]; then
    git -C "$repo" add -A && git -C "$repo" commit -q -m 'feat: a consumer carrying the old address'
  fi
  printf '%s\n' "$repo"
}

run_gate() { (cd "$1" && bash scripts/check-no-hardcoded-addresses.sh 2>&1); }

expect_red() {
  local name="$1" repo="$2" needle="$3" out
  if out="$(run_gate "$repo")"; then
    fail "$name (gate reported OK; expected refusal)
--- gate output ---
$out"
  fi
  case "$out" in
    *"$needle"*) pass "$name" ;;
    *) fail "$name (refused, but '$needle' absent from:
$out)" ;;
  esac
}

expect_green() {
  local name="$1" repo="$2" out
  if ! out="$(run_gate "$repo")"; then
    fail "$name (gate refused; expected OK)
--- gate output ---
$out"
  fi
  pass "$name"
}

# ---------------------------------------------------------------------------------------------
# 1. THE HOLE ITSELF. A file carries an address the ledger published and a redeploy superseded. It
#    appears in NO hand-written list. Before the fix this was invisible.
# ---------------------------------------------------------------------------------------------
repo="$(new_repo hole src/consumer.ts)"
expect_red "a superseded ledger address is caught with no hand-written retired entry" \
  "$repo" "src/consumer.ts hardcodes a contract address"

# ---------------------------------------------------------------------------------------------
# 2. THE WIDENING IS LOAD-BEARING. Same tree, but the gate's history read is removed. If the mutated
#    copy still refuses, the history derivation is not what caught case 1 and every claim here is
#    resting on something else.
# ---------------------------------------------------------------------------------------------
repo="$(new_repo mutated src/consumer.ts)"
target="$repo/scripts/check-no-hardcoded-addresses.sh"
scrutinee='"${HISTORICAL[@]}"'
grep -qF -- "$scrutinee" "$target" \
  || fail "INERT MUTATION: scrutinee '$scrutinee' absent from the gate before mutating.
       The gate has changed shape; this test pins nothing until the mutation is re-aimed."
python3 - "$target" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
before = s
# Drop HISTORICAL from the pattern, and neutralise the emptiness guard that would otherwise abort.
s = s.replace('"${LIVE[@]}" "${HISTORICAL[@]}"', '"${LIVE[@]}"')
s = s.replace('if [[ ${#HISTORICAL[@]} -eq 0 ]]; then', 'if false; then')
assert s != before, "mutation produced no change"
open(p, 'w').write(s)
PY
grep -qF -- "$scrutinee" "$target" \
  && fail "INERT MUTATION: scrutinee still present after mutating; the edit did not take."
if out="$(cd "$repo" && bash scripts/check-no-hardcoded-addresses.sh 2>&1)"; then
  pass "the widening is load-bearing (history removed -> gate goes green on the same offending tree)"
else
  fail "the widening is NOT load-bearing: with the history read removed the gate still refused, so
       something other than the ledger-history derivation is catching case 1.
--- mutated gate output ---
$out"
fi

# ---------------------------------------------------------------------------------------------
# 3. THE chainId FILTER. Without it, the earliest anvil-era ledger (chainId 31337) would fold
#    `0x000...00A1` and anvil's deterministic addresses into the pattern and redden 16 files that use
#    them as synthetic test constants. A gate that cries wolf gets switched off, so this is not a
#    nicety - it is what makes the widening affordable.
# ---------------------------------------------------------------------------------------------
repo="$TEST_ROOT/chainfilter"
mkdir -p "$repo/scripts" "$repo/contracts/deployments" "$repo/src"
git -C "$repo" init -q -b main
git -C "$repo" config user.name 'Dogtag address gate test'
git -C "$repo" config user.email 'addrgate-test@dogtag.invalid'
cp "$GATE" "$repo/scripts/check-no-hardcoded-addresses.sh"
printf '{"retired": [], "stillHardcoded": {}}\n' > "$repo/scripts/address-debt.json"
printf '{\n  "Registry": "%s",\n  "chainId": 31337\n}\n' "$ANVIL_ADDR" \
  > "$repo/contracts/deployments/roax.json"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: local anvil ledger'
printf '{\n  "Registry": "%s",\n  "chainId": 135\n}\n' "$NEW_ADDR" \
  > "$repo/contracts/deployments/roax.json"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: live chain'
printf 'const CLONE_A = "%s";\n' "$ANVIL_ADDR" > "$repo/src/fixture.ts"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'test: a synthetic fixture constant'
expect_green "an address from a DIFFERENT chain's ledger revision is not treated as retired" "$repo"

# ---------------------------------------------------------------------------------------------
# 4. A revision with NO chainId is INCLUDED. Skipping it would narrow the pattern silently, which is
#    the failure mode this whole file exists about; including can only over-report, which is loud.
# ---------------------------------------------------------------------------------------------
repo="$TEST_ROOT/nochainid"
mkdir -p "$repo/scripts" "$repo/contracts/deployments" "$repo/src"
git -C "$repo" init -q -b main
git -C "$repo" config user.name 'Dogtag address gate test'
git -C "$repo" config user.email 'addrgate-test@dogtag.invalid'
cp "$GATE" "$repo/scripts/check-no-hardcoded-addresses.sh"
printf '{"retired": [], "stillHardcoded": {}}\n' > "$repo/scripts/address-debt.json"
printf '{\n  "Registry": "%s"\n}\n' "$OLD_ADDR" > "$repo/contracts/deployments/roax.json"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: ledger with no chainId'
printf '{\n  "Registry": "%s",\n  "chainId": 135\n}\n' "$NEW_ADDR" \
  > "$repo/contracts/deployments/roax.json"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'deploy: live chain'
printf 'export const ADDR = "%s";\n' "$OLD_ADDR" > "$repo/src/consumer.ts"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'feat: consumer'
expect_red "a revision with no chainId is included rather than silently skipped" \
  "$repo" "src/consumer.ts hardcodes a contract address"

# ---------------------------------------------------------------------------------------------
# 5. A SHALLOW CLONE truncates `git log`, so the history read would return a narrower pattern with no
#    error - a check that passes by not running. It must refuse instead.
# ---------------------------------------------------------------------------------------------
src="$(new_repo shallowsrc src/consumer.ts)"
shallow="$TEST_ROOT/shallow"
git clone -q --depth 1 "file://$src" "$shallow" 2>/dev/null
cp "$GATE" "$shallow/scripts/check-no-hardcoded-addresses.sh"
if [[ "$(git -C "$shallow" rev-parse --is-shallow-repository)" != "true" ]]; then
  fail "INERT FIXTURE: the clone is not shallow, so this case cannot test the shallow guard."
fi
expect_red "a shallow clone is refused rather than silently checking a narrower pattern" \
  "$shallow" "shallow"

# ---------------------------------------------------------------------------------------------
# 6. BIDIRECTIONAL, unchanged by the widening: a declared file that no longer carries an address is
#    still an error, so the list can only shrink.
# ---------------------------------------------------------------------------------------------
repo="$(new_repo stale)"
printf '{"retired": [], "stillHardcoded": {"src/gone.ts": "reason"}}\n' \
  > "$repo/scripts/address-debt.json"
printf 'export const CLEAN = 1;\n' > "$repo/src/gone.ts"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'chore: cleaned file, stale entry'
expect_red "a declared file that no longer carries an address is still an error" \
  "$repo" "no longer hardcodes an address"

# ---------------------------------------------------------------------------------------------
# 7. Declaring the offender clears it - the escape hatch still works after the widening.
# ---------------------------------------------------------------------------------------------
repo="$(new_repo declared src/consumer.ts)"
printf '{"retired": [], "stillHardcoded": {"src/consumer.ts": "reason"}}\n' \
  > "$repo/scripts/address-debt.json"
git -C "$repo" add -A && git -C "$repo" commit -q -m 'chore: declare the consumer'
expect_green "a declared offender clears the gate" "$repo"

printf '\naddress-gate: all cases passed\n'
