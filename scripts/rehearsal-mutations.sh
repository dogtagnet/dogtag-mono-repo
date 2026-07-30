#!/usr/bin/env bash
#
# S-12: prove every cutover-rehearsal assertion can actually FAIL.
#
# An assertion that has never failed is a check that never ran. This applies one deliberate break at
# a time to the CUTOVER SEQUENCE or to the contract the assertion rests on, runs the specific test
# that should catch it, and requires that test to go RED. A mutation whose test stays green is
# reported as a FAILURE of this script: it means the assertion is not pinning what it claims to.
#
# Every mutation is reverted, including on interrupt or error (see the trap). The script refuses to
# start on a dirty tree, so a failed revert cannot be mistaken for uncommitted work.
#
# Usage: ROAX_FORK_RPC=<endpoint> scripts/rehearsal-mutations.sh
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR/contracts"

: "${ROAX_FORK_RPC:?ROAX_FORK_RPC must name a ROAX endpoint or a local fork of one}"

SEQ="script/CutoverSequence.sol"
ROUTER="src/CloneProvenanceRouter.sol"
FACTORY2="src/DogTagIssuerFactoryV2.sol"
VREG="src/VerificationRegistryConsent.sol"
CORE="src/ProviderRegistry.sol"
SUITE="rehearsal/CutoverRehearsal.t.sol"
# EVERY file any mutation touches must be here, or `restore` silently leaves the tree mutated.
# This list has already been the source of one such gap; add to it in the SAME change as a mutation.
TARGETS=("$SEQ" "$ROUTER" "$FACTORY2" "$VREG" "$CORE" "$SUITE")

BACKUP="$(mktemp -d)"
restore() {
  for f in "${TARGETS[@]}"; do
    [ -f "$BACKUP/$(basename "$f")" ] && cp "$BACKUP/$(basename "$f")" "$f"
  done
}
trap 'restore; rm -rf "$BACKUP"' EXIT INT TERM

if ! git diff --quiet -- "${TARGETS[@]}"; then
  echo "REFUSING: ${TARGETS[*]} have uncommitted changes; a failed revert would be indistinguishable"
  exit 2
fi
for f in "${TARGETS[@]}"; do cp "$f" "$BACKUP/$(basename "$f")"; done

PASSED=0
FAILED=0

# run_mutation <label> <test-filter> <file> <python-mutation-expression>
# The mutation is a python snippet operating on `s`, the file's contents.
run_mutation() {
  local label="$1" filter="$2" file="$3" mutation="$4"
  printf '\n=== MUTATION: %s\n    expects RED: %s\n' "$label" "$filter"

  MUT_FILE="$file" python3 -c "
import os, sys
p = os.environ['MUT_FILE']
s = open(p).read()
before = s
# Shared anchors for the assertion-7 mutations, defined once so the two agree by construction.
DELIST_CALL = (
    '        vm.prank(gen1.governance);\n'
    '        IIssuerRegistryV1(gen1.issuerRegistry).delistFor(cloneRecordType, REHEARSAL_GEN1_SIGNER);'
)
REVOKE_THEN_DELIST = (
    '        vm.prank(REHEARSAL_GEN1_SIGNER);\n'
    '        DogTagIssuer(gen1Clone).revoke(fixtureRoot);\n'
    + DELIST_CALL
)
$mutation
if s == before:
    sys.exit('mutation did not change the file - the anchor text has drifted')
open(p, 'w').write(s)
" || { echo "    SETUP FAILED: could not apply the mutation"; FAILED=$((FAILED + 1)); restore; return; }

  local out
  out=$(FOUNDRY_PROFILE=rehearsal forge test --match-path 'rehearsal/*' --match-test "$filter" 2>&1)
  local status=$?
  restore

  if [ $status -ne 0 ]; then
    echo "    RED as required."
    echo "$out" | grep -E "^\[FAIL" | sed 's/^/      /' | head -4
    PASSED=$((PASSED + 1))
  else
    echo "    *** STAYED GREEN — the assertion does not pin this. ***"
    echo "$out" | grep -E "^\[PASS|Suite result" | sed 's/^/      /' | head -4
    FAILED=$((FAILED + 1))
  fi
}

# ---------------------------------------------------------------------------------------------
# Assertion 1 — all historical roots resolve through the router
# Break: the router is deployed without generation 1 in its list. This is the "forgot the historical
# generation" mistake, and on the live chain it is unrecoverable because rootIndex is immutable.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "router built without generation 1" \
  "test_1_every_historical_root_resolves_through_the_router" \
  "$SEQ" \
  "s = s.replace('generations[0] = factoryV1; // index 0 is consulted FIRST — oldest first.', 'generations[0] = address(new DogTagIssuerFactory(address(new DogTagIssuer()), address(1), address(this)));')
s = s.replace('import {ProtocolRegistryV2} from \"../src/ProtocolRegistryV2.sol\";', 'import {ProtocolRegistryV2} from \"../src/ProtocolRegistryV2.sol\";\nimport {DogTagIssuerFactory} from \"../src/DogTagIssuerFactory.sol\";\nimport {DogTagIssuer} from \"../src/DogTagIssuer.sol\";')"

# ---------------------------------------------------------------------------------------------
# Assertion 2 — a generation-1 credential verifies on the generation-2 registry
# Break: the generation-2 registry binds its immutable rootIndex to the generation-2 FACTORY instead
# of the router. This is plan §5's worst case, and the whole reason the router exists.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "registry rootIndex bound to the generation-2 factory instead of the router" \
  "test_2_a_generation_1_credential_verifies_on_the_generation_2_registry" \
  "$SEQ" \
  "s = s.replace('d.registryV2 = c5_deployRegistryV2(d.core, d.router, gen1); // C-5', 'd.registryV2 = new VerificationRegistryConsent(address(d.core), gen1.sbt, gen1.verifier, address(d.factoryV2), gen1.governance); // C-5')"

# ---------------------------------------------------------------------------------------------
# Assertion 3 — a generation-2 credential verifies on the generation-2 registry
# Break: C-4b is skipped. The step the plan does not contain at all.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "C-4b (appendGeneration) omitted" \
  "test_3_a_generation_2_credential_verifies_on_the_generation_2_registry" \
  "$SEQ" \
  "s = s.replace('        c4b_appendGeneration(d.router, d.factoryV2); // C-4b <- not in the plan', '        // c4b_appendGeneration omitted by mutation')"

# ---------------------------------------------------------------------------------------------
# Assertion 4 — a generation-2 credential does NOT verify on the generation-1 registry
#
# The generation-1 registry is the REAL deployed instance, so its behaviour cannot be mutated on a
# fork. What CAN be wrong is the assertion: `!verify-wl` fires at :153, BEFORE root resolution at
# :187, so if the relayer is not whitelisted on generation 1 the submission reverts for a reason
# that has nothing to do with root resolution — and the test would pass while proving nothing.
#
# Break: drop the generation-1 whitelist precondition. The exact-string assertion must catch it.
# This is the mutation that shows the trap is closed rather than merely commented on.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "generation-1 relayer precondition dropped (assertion must not accept the wrong revert)" \
  "test_4_a_generation_2_credential_does_not_verify_on_the_generation_1_registry" \
  "$SUITE" \
  "s = s.replace('''        _anchorInGeneration2();
        _whitelistRelayerOnGeneration1();''', '''        _anchorInGeneration2();''')"

# ---------------------------------------------------------------------------------------------
# Assertion 5 — an unmigrated relayer fails `!verify-wl`
# Break: the registry's relayer restriction is disabled, so C-6 stops gating anything and the
# capability migration looks optional.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "relayer restriction defaults off, so C-6 gates nothing" \
  "test_5_an_unmigrated_relayer_fails_verify_wl_and_a_migrated_one_passes" \
  "$VREG" \
  "s = s.replace('bool public restrictToWhitelistedRelayers = true;', 'bool public restrictToWhitelistedRelayers = false;')"

# ---------------------------------------------------------------------------------------------
# Assertion 6a — the guarded generation-2 factory refuses a root generation 1 already holds
# Break: the cross-generation write guard is removed (defence in depth).
# ---------------------------------------------------------------------------------------------
run_mutation \
  "cross-generation write guard removed from registerRoot" \
  "test_6a_the_generation_2_factory_refuses_a_root_generation_1_already_holds" \
  "$FACTORY2" \
  "s = s.replace('        require(!priorIndex.isRootAnchored(root), \"root taken upstream\");', '')"

# ---------------------------------------------------------------------------------------------
# Assertion 6b — oldest-first survives an unguarded later generation
# Break: reverse the resolution loop. THE revocation bypass. Nothing errors; a revoked credential
# simply starts verifying again.
# ---------------------------------------------------------------------------------------------
run_mutation \
  "router resolution reversed to NEWEST first" \
  "test_6b_a_revoked_root_survives_re_anchoring_by_an_unguarded_later_generation" \
  "$ROUTER" \
  "s = s.replace('''        uint256 n = _generations.length;
        for (uint256 i; i < n; i++) {
            address clone = IIssuerFactoryGeneration(_generations[i]).rootIssuer(root);
            if (clone != address(0)) return clone;
        }
        return address(0);''', '''        uint256 n = _generations.length;
        for (uint256 i = n; i > 0; i--) {
            address clone = IIssuerFactoryGeneration(_generations[i - 1]).rootIssuer(root);
            if (clone != address(0)) return clone;
        }
        return address(0);''')"

# ---------------------------------------------------------------------------------------------
# Correction 3 — C-2 cannot attach an ownerless generation-1 clone
# Break: accept a service whose owner() cannot be read. That is the change someone reaching for a
# "legacy controller adapter" would make, and it would silently admit the five ownerless clones.
# ---------------------------------------------------------------------------------------------
#
# BOTH edits are needed, and that is itself the finding: `_readServiceMetadata` folds `!ownerOk` into
# a single `metadataOk`, so relaxing only the downstream `liveOwner == address(0)` guard changes
# nothing observable. A one-line mutation stays green while proving nothing — which is exactly the
# inert-mutation trap, and this harness caught it on the first run rather than being believed.
run_mutation \
  "core given a legacy-controller adapter that accepts an ownerless clone" \
  "test_correction_3_c2_cannot_attach_an_ownerless_generation_1_clone" \
  "$CORE" \
  "s = s.replace('        if (!recordTypeOk || recordTypeData.length < 32 || !ownerOk) return (false, bytes32(0), address(0));', '        if (!recordTypeOk || recordTypeData.length < 32) return (false, bytes32(0), address(0));')
s = s.replace('''        if (!metadataOk || recordType == bytes32(0) || liveOwner == address(0)) {
            revert InvalidServiceMetadata();
        }
        if (liveOwner != expectedOwner) revert UnexpectedServiceOwner();''', '''        if (!metadataOk || recordType == bytes32(0)) {
            revert InvalidServiceMetadata();
        }
        if (liveOwner == address(0)) liveOwner = expectedOwner;
        if (liveOwner != expectedOwner) revert UnexpectedServiceOwner();''')"

# ---------------------------------------------------------------------------------------------
# Assertion 7 (C-12) - TWO mutations, one per half, because the assertion makes two claims.
#
# Note what CANNOT be mutated here: part (3) verifies through the REAL DEPLOYED generation-1 clone
# and the REAL deployed registry, so no edit to src/DogTagIssuer.sol can reach it - that bytecode
# comes from the chain, not from this tree. An earlier attempt at exactly that mutation stayed GREEN
# and this harness caught it. Both mutations below therefore target the freeze the test PERFORMS.
# ---------------------------------------------------------------------------------------------

# Half 1 - the freeze is actually performed. This maps directly to the original review finding:
# without the delisting the test would DESCRIBE C-12 rather than exercise it.
run_mutation \
  "the C-12 delisting is not performed (the assertion must not merely describe the freeze)" \
  "test_7_the_generation_1_freeze_stops_new_issuance_and_preserves_verification" \
  "$SUITE" \
  "s = s.replace(DELIST_CALL, '        // delistFor omitted by mutation')"

# Half 2 - the freeze must not break existing credentials. Models a runbook that revokes while it
# freezes; part (3) of the assertion is the only thing that catches it.
run_mutation \
  "the freeze also revokes the anchored root (history must NOT survive that)" \
  "test_7_the_generation_1_freeze_stops_new_issuance_and_preserves_verification" \
  "$SUITE" \
  "s = s.replace(DELIST_CALL, REVOKE_THEN_DELIST)"

printf '\n============================================================\n'
printf 'mutations that correctly reddened their assertion: %d\n' "$PASSED"
printf 'mutations that did NOT:                            %d\n' "$FAILED"
printf '============================================================\n'
[ "$FAILED" -eq 0 ]
