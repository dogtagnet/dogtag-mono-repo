#!/bin/bash
# The repeatable mutation gate for the clone's CREATION SEED (`DogTagIssuer.initialize`).
#
# Run it to prove the seed's four claims are PINNED rather than merely present: the seed exists, it
# is reported through the ordinary event, it is a removable VALUE rather than an ownership RULE, and
# it happens once at creation.
#
# Written in bash, not zsh, for the reason `scripts/verify-provider-selfservice-mutations.sh`
# records: zsh does not word-split an unquoted "$var", so a list-driven loop there runs once with
# the whole string and every search misses - a check that passes by not running.
#
# Restore is by FILE COPY, never `git checkout --`: this repo has already lost uncommitted work to a
# harness that took git as its pristine baseline, and a cp-based restore is correct whatever the
# working tree contains.
#
# Three verdicts, and the third is why this script exists at all:
#   PINNED - the mutation applied, compiled, and reddened the named test.
#   GREEN  - the mutation applied and the suite still passed. The claim is NOT pinned.
#   INERT  - the mutation never applied, or applied and failed to COMPILE. Its red proves nothing,
#            so it is counted as this harness's own failure rather than as evidence.
set -u
cd "$(cd "$(dirname "$0")/.." && pwd)/contracts"
export LC_ALL=C

TARGETS="src/DogTagIssuer.sol"
BACKUP=$(mktemp -d)
for f in $TARGETS; do mkdir -p "$BACKUP/$(dirname "$f")"; cp "$f" "$BACKUP/$f"; done
restore() { for f in $TARGETS; do cp "$BACKUP/$f" "$f"; done; }
trap restore EXIT

fails=0

# $1 label, $2 file, $3 old, $4 new, $5 test name the mutation must redden
mutate() {
  local label="$1" file="$2" old="$3" new="$4" want="$5"
  restore
  if ! grep -qF -- "$old" "$file"; then
    echo "INERT  $label -- search string absent from $file; mutation never applied"
    fails=$((fails+1)); return
  fi
  python3 - "$file" "$old" "$new" <<'PY'
import sys
p, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
assert old in s
open(p, "w").write(s.replace(old, new, 1))
PY
  out=$(forge test 2>&1)
  # A mutation that does not COMPILE is inert, not evidence: solc's failure is not the assertion's.
  if echo "$out" | grep -qE "Compiler run failed|Error \(|error\["; then
    echo "INERT  $label -- mutation does not compile; its red proves nothing"
    fails=$((fails+1)); return
  fi
  if ! echo "$out" | grep -q "\[FAIL: .*\] $want"; then
    echo "GREEN  $label -- $want SURVIVED the mutation; the claim is not pinned"
    fails=$((fails+1)); return
  fi
  red=$(echo "$out" | grep -cE "^\[FAIL")
  echo "PINNED $label -- $want reddened ($red failing test(s) total)"
}

echo "=== creation-seed mutation evidence ==="

# 1. The seed itself. The captain's ask is that the deployer can use what it deployed; without the
#    write, the real-core journey ends where it used to - refused by the contract's own creator.
mutate "the seed is deleted (a provider cannot use the clone it just deployed)" \
  src/DogTagIssuer.sol \
  "        issuanceAllowed[owner_] = true;
        emit IssuanceAllowedSet(owner_, true, msg.sender);" \
  "" \
  "test_a_provider_anchors_through_the_clone_it_just_deployed_without_admitting_itself"

# 2. The event, alone. A silent storage write leaves every off-chain reconstruction of the list
#    missing its most likely entry, with the chain and the decoder disagreeing and nothing saying so.
mutate "the seed writes storage but emits nothing (decoders miss the entry)" \
  src/DogTagIssuer.sol \
  "        emit IssuanceAllowedSet(owner_, true, msg.sender);" \
  "" \
  "test_the_creator_is_seeded_onto_its_own_clones_list_at_creation"

# 3. THE WRONG READING OF THE ASK: ownership as a RULE in the issuance path rather than a seeded
#    VALUE in the list. It satisfies "the deployer can issue" and silently makes removal
#    unenforceable, which disarms the withdrawal lever the contract doc argues at length.
mutate "ownership becomes an arm of the issuance check (removal stops working)" \
  src/DogTagIssuer.sol \
  "        if (!issuanceAllowed[msg.sender]) revert NotLocallyAllowed();" \
  "        if (!issuanceAllowed[msg.sender] && msg.sender != owner()) revert NotLocallyAllowed();" \
  "test_the_seeded_creator_is_an_ordinary_list_entry_that_removal_costs_no_ownership"

# 4. Seeding on every ownership change rather than once at creation. Ongoing, it makes ownership
#    imply the right after all, and it silently re-admits an address a previous owner removed.
mutate "the seed follows ownership instead of happening once at creation" \
  src/DogTagIssuer.sol \
  "    function acceptOwnership() public override {
        if (msg.sender == address(0)) revert OwnerCannotBeZero();
        super.acceptOwnership();" \
  "    function acceptOwnership() public override {
        if (msg.sender == address(0)) revert OwnerCannotBeZero();
        super.acceptOwnership();
        issuanceAllowed[msg.sender] = true;" \
  "test_a_handover_does_not_seed_the_new_owner"

# 5. A plausible slip: seeding the initializing CALLER (the factory) rather than the owner it was
#    handed. The factory can never sign, so the seed would be inert and silently so.
mutate "the seed names the initializing caller instead of the owner" \
  src/DogTagIssuer.sol \
  "        issuanceAllowed[owner_] = true;
        emit IssuanceAllowedSet(owner_, true, msg.sender);" \
  "        issuanceAllowed[msg.sender] = true;
        emit IssuanceAllowedSet(msg.sender, true, msg.sender);" \
  "test_the_creator_is_seeded_onto_its_own_clones_list_at_creation"

restore
echo
if [ "$fails" -eq 0 ]; then
  echo "OK - every mutation applied, compiled, and reddened its named test."
else
  echo "FAILED - $fails mutation(s) survived or were inert."
fi
exit "$fails"
