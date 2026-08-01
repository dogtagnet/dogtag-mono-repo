#!/bin/bash
# registry-plan S-15: the repeatable mutation gate for the provider self-service engine.
#
# Run it to prove the engine's guards are PINNED rather than merely present. Written in bash, not
# zsh: zsh does not word-split an unquoted "$var", so a list-driven loop there runs once with the
# whole string and every search misses - a check that passes by not running. LC_ALL=C is exported
# for the WHOLE script rather than around one command, so the verdict does not depend on the
# caller's locale.
#
# A mutation that leaves the suite GREEN is reported as this harness's own failure. So is one whose
# search string no longer matches: an unapplied mutation produces a false 'pinned' reading, which is
# the exact mistake scripts/verify-issuance-authority-mutations.sh was written to stop making.
# A mutation must also CHANGE BEHAVIOUR - deleting the explicit lowercase check in validateDomain
# was tried and rejected here, because the charset rule below it refuses uppercase anyway, so only
# the message changed and 'unpinned' would have been the wrong reading. One break at a time; a mutation that leaves the suite GREEN is reported as
# its own failure, and a mutation that fails to APPLY is reported as INERT rather than counted as
# evidence (a mutation whose search string no longer matches produces a false "pinned" reading).
set -u
cd "$(cd "$(dirname "$0")/.." && pwd)"
export LC_ALL=C

TARGETS="packages/ui/src/provider/cloneProvenance.ts packages/ui/src/provider/directoryPlan.ts packages/ui/src/provider/liveReader.ts packages/ui/src/provider/domainClaim.ts packages/ui/src/provider/sendOutcome.ts packages/ui/src/domain/ProviderSelfServiceFlows.tsx"
BACKUP=$(mktemp -d)
for f in $TARGETS; do mkdir -p "$BACKUP/$(dirname "$f")"; cp "$f" "$BACKUP/$f"; done
restore() { for f in $TARGETS; do cp "$BACKUP/$f" "$f"; done; }
trap restore EXIT

fails=0

# $1 label, $2 file, $3 old, $4 new, $5 test file
mutate() {
  local label="$1" file="$2" old="$3" new="$4" spec="$5"
  restore
  if ! grep -qF -- "$old" "$file"; then
    echo "INERT  $label -- search string absent from $file; mutation never applied"
    fails=$((fails+1)); return
  fi
  python3 - "$file" "$old" "$new" <<'PY'
import sys
p,old,new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
assert old in s
open(p,"w").write(s.replace(old, new, 1))
PY
  out=$(pnpm --filter @dogtag/ui exec vitest run "$spec" 2>&1)
  if echo "$out" | grep -qE "Tests +[0-9]+ failed"; then
    red=$(echo "$out" | grep -oE "Tests +[0-9]+ failed" | head -1)
    echo "PINNED $label -- $red"
  else
    echo "GREEN  $label -- MUTATION SURVIVED, the claim is not pinned"
    fails=$((fails+1))
  fi
}

echo "=== S-15 mutation evidence ==="

mutate "provenance refusal deleted (a forged address falls through)" \
  packages/ui/src/provider/cloneProvenance.ts \
  "  if (genuine === false) {" \
  "  if (false) {" \
  test/providerCloneGuard.test.ts

mutate "a failed isClone READ rendered as 'not genuine'" \
  packages/ui/src/provider/cloneProvenance.ts \
  '        "could-not-run",
        "The factory could not be reached, so this address was neither confirmed nor refused.",
        reasonFrom(error, "the isClone read failed"),' \
  '        "fail",
        "The factory could not be reached, so this address was neither confirmed nor refused.",' \
  test/providerCloneGuard.test.ts

mutate "ownership stops being REPORTED (a delegate cannot see whose contract it is)" \
  packages/ui/src/provider/cloneProvenance.ts \
  "    const owned = owner.toLowerCase() === caller.toLowerCase();" \
  "    const owned = true;" \
  test/providerCloneGuard.test.ts

# The authorization regression this slice's review found: the portal re-deriving `owner() == caller`
# instead of composing `canWriteService`, which refuses a delegate the CHAIN admits. Mutating the
# report back into a refusal is the exact shape of it.
mutate "clone-control made able to refuse again (a legitimate delegate is turned away)" \
  packages/ui/src/provider/cloneProvenance.ts \
  '        "Who owns this contract?",
        "pass",' \
  '        "Who owns this contract?",
        owned ? "pass" : "fail",' \
  test/providerCloneGuard.test.ts

mutate "the repoint authority sourced from ownership rather than from the chain" \
  packages/ui/src/provider/cloneProvenance.ts \
  "      const mayRepoint = await reader.canWriteServiceRepoint(candidate, caller);" \
  "      const mayRepoint = owner?.toLowerCase() === caller.toLowerCase();" \
  test/providerCloneGuard.test.ts

mutate "attachment collapsed (deployed treated as attached)" \
  packages/ui/src/provider/cloneProvenance.ts \
  "    attachedHere = !attachedToNobody && service.providerId.toLowerCase() === providerId.toLowerCase();" \
  "    attachedHere = true;" \
  test/providerCloneGuard.test.ts

# Re-anchored: the branch now also requires the listing state to have been READ, because appending
# is only safe when we know there is nothing to replace.
mutate "a placeholder pin emitted for an ABSENT location (the 0,0 bug)" \
  packages/ui/src/provider/directoryPlan.ts \
  '  if (location.kind === "located" && listing) {' \
  '  if (location.kind !== "invalid" && listing) {' \
  test/providerDirectoryPlan.test.ts

mutate "a SECOND pin appended instead of rewriting the one being corrected" \
  packages/ui/src/provider/directoryPlan.ts \
  "    if (listing.pinCount === 0) {" \
  "    if (true) {" \
  test/providerDirectoryPlan.test.ts

mutate "several existing pins silently appended to rather than refused" \
  packages/ui/src/provider/directoryPlan.ts \
  '    if (tooMany && location.kind === "located") {' \
  "    if (false) {" \
  test/providerDirectoryPlan.test.ts

mutate "the anchor re-written for nothing (a registrar address confirmation silently invalidated)" \
  packages/ui/src/provider/directoryPlan.ts \
  "  if (!anchorUnchanged) {" \
  "  if (true) {" \
  test/providerDirectoryPlan.test.ts

mutate "an empty publication reported ready while its own advice says to add something" \
  packages/ui/src/provider/directoryPlan.ts \
  '  if (channelsPublished === 0 && location.kind !== "located") {' \
  "  if (false) {" \
  test/providerDirectoryPlan.test.ts

mutate "withdrawal offered without establishing this key's authority over the record" \
  packages/ui/src/provider/directoryPlan.ts \
  "      directoryLive === true && mayWriteRecord === true && !!listing?.onlyPin," \
  "      !!listing?.onlyPin," \
  test/providerDirectoryPlan.test.ts

mutate "a zero hashAlgorithm on the profile anchor (contract reverts BadProfileAnchor)" \
  packages/ui/src/provider/directoryPlan.ts \
  "export const MULTIHASH_KECCAK_256 = 0x1b;" \
  "export const MULTIHASH_KECCAK_256 = 0;" \
  test/providerDirectoryPlan.test.ts

mutate "coordinate truncated instead of rounded" \
  packages/ui/src/provider/directoryPlan.ts \
  "  return Math.round(degrees * COORDINATE_SCALE);" \
  "  return Math.trunc(degrees * COORDINATE_SCALE);" \
  test/providerDirectoryPlan.test.ts

mutate "canCreateService asked with no 'from' (the msg.sender trap)" \
  packages/ui/src/provider/liveReader.ts \
  "        account: contracts.factory," \
  "" \
  test/providerDomainAndDeploy.test.ts

mutate "an unreadable domain record defaulted to UNSET ('nobody has said')" \
  packages/ui/src/provider/domainClaim.ts \
  "    ...(standing ? { standing } : {})," \
  "    standing: standing ?? { disposition: 0, domain: \"\", lineageRecognizesService: false, registryApprovesThisResolver: false, coreSelectsThisResolver: false, serviceStandingEffective: false }," \
  test/providerDomainAndDeploy.test.ts

# The MEANINGFUL version of the case mutation. Merely deleting the explicit lowercase check is
# behaviour-preserving - the charset rule below it refuses uppercase anyway, so only the message
# changes - and reporting that as "unpinned" would be the inert-mutation reading this harness exists
# to avoid. The real defect class is NORMALIZING, so the mutation normalizes.
mutate "domain silently NORMALIZED instead of refused" \
  packages/ui/src/provider/domainClaim.ts \
  "  const domain = input.trim();" \
  "  const domain = input.trim().toLowerCase();" \
  test/providerDomainAndDeploy.test.ts

# The two claims with no chain behind them: the capability split (verified once in a browser, which
# is not repeatable) and the write ABIs (whose send path has never been executed anywhere, so a
# mistyped argument would first surface as an on-chain revert reading like an authorization fault).
mutate "issuance flows rendered for an operator that issues nothing" \
  packages/ui/src/domain/ProviderSelfServiceFlows.tsx \
  "      {capabilities.issuance ? (" \
  "      {true ? (" \
  test/providerCapabilitySplit.test.tsx

mutate "a write ABI argument type mistyped (wrong selector, on-chain revert)" \
  packages/ui/src/provider/liveReader.ts \
  '      { name: "cloneNonce", type: "uint96" },
    ],
    outputs: [{ name: "clone", type: "address" }],' \
  '      { name: "cloneNonce", type: "uint256" },
    ],
    outputs: [{ name: "clone", type: "address" }],' \
  test/providerWriteAbi.test.ts

mutate "the repoint permission bit swapped for its neighbour (a confident false, not a revert)" \
  packages/ui/src/provider/liveReader.ts \
  "export const SERVICE_PERMISSION_REPOINT = 4;" \
  "export const SERVICE_PERMISSION_REPOINT = 1;" \
  test/providerWriteAbi.test.ts

# The two send-time guards. Both live in the component and the outcome module rather than in the pure
# engine, which is why their catchers are the mounted suites.
#
# NOTE ON WHAT IS *NOT* MUTATED HERE, deliberately. Rule 1 in the flows' header is enforced twice -
# a plan is invalidated when its inputs change AND every send addresses the plan's own captured
# values - and the two are redundant BY CONSTRUCTION: every input a send reads is in that plan's
# key, so while invalidation holds a send from current form state produces identical arguments. A
# mutation swapping one captured value for its field would therefore survive, and counting a
# behaviour-preserving mutation as evidence is the reading this harness exists to avoid. The
# invalidation half is mutated below; the captured half is asserted on the plan objects themselves in
# providerDomainAndDeploy / providerDirectoryPlan / providerCloneGuard, where it IS observable.
# Re-anchored: `fresh` now folds a SECOND retirement reason, so the old search string is absent and
# would have reported INERT. The two halves are mutated separately because they retire a plan for
# different reasons - the form moved, versus the chain moved - and a fix to one says nothing about
# the other.
mutate "a stale plan keeps gating the button (check one address, send another)" \
  packages/ui/src/domain/ProviderSelfServiceFlows.tsx \
  "  return held && !held.spent && held.key === key ? held.plan : null;" \
  "  return held && !held.spent ? held.plan : null;" \
  test/providerSendGuards.test.tsx

mutate "a plan outlives its own transaction (press Send twice, send twice)" \
  packages/ui/src/domain/ProviderSelfServiceFlows.tsx \
  "  return held && !held.spent ? { ...held, spent: true } : held;" \
  "  return held;" \
  test/providerSendGuards.test.tsx

mutate "a reverted transaction reported as a completed one" \
  packages/ui/src/provider/sendOutcome.ts \
  '  return status === "success" ? "succeeded" : "reverted";' \
  '  return "succeeded";' \
  test/providerSendOutcome.test.ts

mutate "an unfollowable receipt collapsed into success rather than staying unestablished" \
  packages/ui/src/provider/sendOutcome.ts \
  '  return state === "succeeded";' \
  '  return state !== "reverted";' \
  test/providerSendOutcome.test.ts

echo "=== $fails mutation(s) unaccounted for ==="
restore
exit $((fails > 0 ? 1 : 0))
