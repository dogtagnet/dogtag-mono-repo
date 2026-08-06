#!/usr/bin/env bash
# Mutation gate for the provider's typed-resolver SELECTION (flows 3 and 4).
#
# WHY THIS EXISTS. A test that cannot fail certifies itself, and this surface is built almost entirely
# out of distinctions - approved against merely listed, none against withdrawn-underneath, standing
# against authority, pending against nothing-approved. Every one of those is a place where collapsing
# two states passes a suite that names them.
#
# WHAT IT REFUSES TO DO. A mutation whose scrutinee is ABSENT, or that leaves the target unable to
# compile, is reported INERT and exits non-zero - it is NOT counted as evidence. This repo has shipped
# that mistake twice: once with a mutation naming a method that did not exist, whose "red" was a build
# failure, and once with a text-only edit that reddened nothing because the test asserted a condition
# rather than the words. Self-tested at the end, in BOTH directions.
#
# IT RESTORES FROM A COPY, NOT FROM GIT. `git checkout --` over a dirty tree destroys uncommitted work
# in every TARGET, and reports the wreckage as a pile of INERT mutations, which reads like a harness
# fault rather than like your work having just been deleted. Commit anyway before running this.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

UI=packages/ui
ENGINE=$UI/src/provider/resolverSelection.ts
LIVE=$UI/src/provider/liveReader.ts
FLOWS=$UI/src/domain/ProviderSelfServiceFlows.tsx
PANEL=$UI/src/domain/ProviderSelfServicePanel.tsx
BUTTON=$UI/src/components/Button.tsx
CORE=contracts/src/ProviderRegistry.sol
TARGETS=("$ENGINE" "$LIVE" "$FLOWS" "$PANEL" "$BUTTON" "$CORE")

BACKUP=$(mktemp -d)
trap 'for f in "${TARGETS[@]}"; do cp "$BACKUP/$(echo "$f" | tr / _)" "$f"; done; rm -rf "$BACKUP"' EXIT
for f in "${TARGETS[@]}"; do cp "$f" "$BACKUP/$(echo "$f" | tr / _)"; done

PASS=0
FAIL=0
INERT=0

restore() { for f in "${TARGETS[@]}"; do cp "$BACKUP/$(echo "$f" | tr / _)" "$f"; done; }

# apply <file> <literal-from> <literal-to>  -> 0 when the scrutinee was present and replaced
apply() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys, pathlib
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
p = pathlib.Path(path)
s = p.read_text()
if old not in s:
    sys.exit(3)
p.write_text(s.replace(old, new, 1))
PY
}

# run_ts <test-file> <test-name-substring>
run_ts() {
  ( cd "$UI" && npx vitest run "$1" -t "$2" --reporter=dot 2>&1 )
}

# run_sol <test-fn>
run_sol() {
  ( cd contracts && forge test --match-test "$1" 2>&1 )
}

# check <label> <file> <from> <to> <runner> <arg1> [arg2]
check() {
  local label="$1" file="$2" from="$3" to="$4" runner="$5" a1="$6" a2="${7:-}"
  restore
  if ! apply "$file" "$from" "$to"; then
    echo "INERT  $label"
    echo "       scrutinee absent from $file - this mutation tested NOTHING"
    INERT=$((INERT + 1))
    return
  fi
  local out
  if [ "$runner" = ts ]; then out=$(run_ts "$a1" "$a2"); else out=$(run_sol "$a1"); fi
  # A mutation that does not COMPILE is inert, not evidence: its red is a build failure.
  if echo "$out" | grep -qiE "error TS[0-9]+|Compiler run failed|Transform failed|Failed to (load|parse)"; then
    echo "INERT  $label"
    echo "       the mutated target does not compile - its red proves nothing"
    INERT=$((INERT + 1))
    restore
    return
  fi
  if echo "$out" | grep -qEi "[0-9]+ failed|FAIL:|\[FAIL|timed out|test timed"; then
    echo "PASS   $label"
    PASS=$((PASS + 1))
  else
    echo "FAIL   $label"
    echo "       the mutation survived - nothing pins this"
    FAIL=$((FAIL + 1))
  fi
  restore
}

SEL=test/providerResolverSelection.test.ts
REN=test/providerResolverSelectionRender.test.tsx
RDR=test/providerResolverReader.test.ts

echo "== the register list is append-only, so an unfiltered list offers a withdrawn register =="

check "the choice list stops filtering on approval" \
  "$ENGINE" \
  'const approvedChoices = listings?.filter((l) => l.approved).map((l) => l.resolver);' \
  'const approvedChoices = listings?.map((l) => l.resolver);' \
  ts "$SEL" "offers only the APPROVED entries"

# Points at the READER suite, not the engine's: the engine injects a fake, so it cannot see the live
# binding at all. This mutation survived the whole engine suite before that suite existed, which is how
# the gap was found rather than reasoned about.
check "the live reader drops the per-entry approval read and calls everything approved" \
  "$LIVE" \
  'approved: (await client.readContract({' \
  'approved: true || (await client.readContract({' \
  ts "$RDR" "asks isResolverApproved for EVERY listed address"

check "the live reader stops paging after the first page" \
  "$LIVE" \
  '      while (cursor < count) {' \
  '      while (cursor < count && cursor === 0n) {' \
  ts "$RDR" "pages to COMPLETION"

# The mutation is `break`, not `if (false)`: stopping quietly is the realistic wrong fix, and it is
# OBSERVABLE - it returns the partial list. Deleting the guard outright loops forever instead, so its red
# would arrive as a hang, which is a worse signal even though the timeout is now caught above.
check "a non-advancing page is swallowed instead of thrown" \
  "$LIVE" \
  '          throw new Error(
            `resolverPage did not advance past cursor ${cursor} of ${count}; the resolver list could not be read in full`,
          );' \
  '          break;' \
  ts "$RDR" "THROWS rather than looping or truncating"

check "a withdrawn choice is accepted instead of refused" \
  "$ENGINE" \
  'if (entry?.approved) {' \
  'if (entry !== undefined) {' \
  ts "$SEL" "a WITHDRAWN register is refused"

check "the two refusal sentences collapse into one" \
  "$ENGINE" \
  '            ? `${shortHex(chosen)} was approved by DogTag and its approval has been withdrawn, so the registry would refuse it. Pick one from the approved list.`' \
  '            ? `${shortHex(chosen)} is not a ${label} DogTag has approved, so the registry would refuse it. Pick one from the approved list.`' \
  ts "$SEL" "those two refusals are DIFFERENT sentences"

check "the withdrawn entries are no longer reported to the renderer" \
  "$ENGINE" \
  'const withdrawnChoices = listings?.filter((l) => !l.approved).map((l) => l.resolver);' \
  'const withdrawnChoices: readonly Address[] | undefined = undefined;' \
  ts "$REN" "lists the withdrawn one separately"

echo
echo "== three selection states, and a failed read is none of them =="

check "withdrawn-underneath collapses into nothing-selected" \
  "$ENGINE" \
  '  return entry?.approved
    ? { kind: "selected", resolver: stored }
    : { kind: "selectedButPulled", resolver: stored };' \
  '  return entry?.approved ? { kind: "selected", resolver: stored } : { kind: "none" };' \
  ts "$SEL" "a selector whose approval was WITHDRAWN is its own state"

check "an unreadable approval list is reported as plain selected" \
  "$ENGINE" \
  '  if (listings === undefined) return undefined;' \
  '  if (listings === undefined) return { kind: "selected", resolver: stored };' \
  ts "$SEL" "a non-zero selector plus an unreadable list is could-not-run"

check "an unreadable choice list is rendered as an empty one" \
  "$ENGINE" \
  '    ...(approvedChoices ? { choices: approvedChoices } : {}),' \
  '    choices: approvedChoices ?? [],' \
  ts "$SEL" "an UNREADABLE list leaves"

echo
echo "== standing before authority, and a refusal never blamed on the key =="

check "a refusal is attributed to the key although standing did not hold" \
  "$ENGINE" \
  '    } else if (standing === true) {' \
  '    } else if (standing !== null) {' \
  ts "$SEL" "the authority row is could-not-run"

check "the domain standing folds hasActiveIssuer, which the chain does not" \
  "$ENGINE" \
  '      e.ownerConfirmed ? null : "this contract'"'"'s owner on file does not match its live owner, so it is held pending confirmation",' \
  '      e.ownerConfirmed ? null : "this contract'"'"'s owner on file does not match its live owner, so it is held pending confirmation",
      e.hasActiveIssuer ? null : "this contract has no active issuer",' \
  ts "$SEL" "does NOT block a register choice"

check "a quarantined contract is called frozen" \
  "$ENGINE" \
  'so it is held pending confirmation"' \
  'so it is frozen"' \
  ts "$SEL" "which is not frozen"

echo
echo "== the no-op the chain refuses =="

check "the no-op preflight is dropped" \
  "$ENGINE" \
  '  if (!sameAddress(current, chosen)) return null;' \
  '  return null;
  if (!sameAddress(current, chosen)) return null;' \
  ts "$SEL" "choosing what is already selected fails"

check "the no-op is asserted from a selection that was never read" \
  "$ENGINE" \
  '  if (!selection) return null;' \
  '  if (!selection) return "nothing is selected";' \
  ts "$SEL" "emits NO no-op row when the current value could not be read"

echo
echo "== the write itself: permission bit, subject, and the selector =="

# Points at the READER suite. `providerWriteAbi.test.ts` pins the CONSTANT against the contract's own
# declaration, which is a different claim from the reader USING it - and swapping the argument leaves the
# constant untouched, so that suite stayed green. Both claims are needed and neither implies the other.
check "the directory-resolver read uses the wrong permission bit" \
  "$LIVE" \
  'args: [providerId, getAddress(caller), PROVIDER_PERMISSION_DIRECTORY_RESOLVER],' \
  'args: [providerId, getAddress(caller), PROVIDER_PERMISSION_RECORD],' \
  ts "$RDR" "asks canWriteProvider with PROVIDER_PERMISSION_DIRECTORY_RESOLVER"

check "the domain-resolver read uses the repoint bit instead" \
  "$LIVE" \
  'args: [getAddress(service), getAddress(caller), SERVICE_PERMISSION_DOMAIN_RESOLVER],' \
  'args: [getAddress(service), getAddress(caller), SERVICE_PERMISSION_REPOINT],' \
  ts "$RDR" "asks canWriteService with SERVICE_PERMISSION_DOMAIN_RESOLVER"

check "setDirectoryResolver takes an address instead of bytes20" \
  "$LIVE" \
  '      { name: "providerId", type: "bytes20" },
      { name: "resolver", type: "address" },' \
  '      { name: "providerId", type: "address" },
      { name: "resolver", type: "address" },' \
  ts test/providerWriteAbi.test.ts "setDirectoryResolver encodes to"

check "the domain send addresses the provider id instead of the contract" \
  "$FLOWS" \
  'args: [domainRegister!.subject, domainRegister!.chosen],' \
  'args: [providerId as `0x${string}`, domainRegister!.chosen],' \
  ts "$REN" "sends setDomainResolver with the CONTRACT address"

# DELIBERATELY NOT MUTATED: swapping `directoryRegister!.chosen` for the live `directoryRegisterChoice`
# is BEHAVIOUR-PRESERVING, and counting its survival as an unpinned claim would be the inert-mutation
# reading this harness exists to avoid. The two can only differ while the plan is stale, and a stale plan
# disables the send - so no reachable state observes the difference. That is the same redundant-by-
# construction pair AGENTS.md records for flow 2, and the sanctioned coverage is what is done instead:
# mutate the INVALIDATION (below, "the chosen register leaves the plan key"), and assert the plan's
# captured inputs directly in the engine suite ("captures the subject and the chosen register"). The
# SUBJECT half of the same rule IS observable and IS mutated - see the domain send above, where the
# provider id and the contract address are both populated at send time and differ.

echo
echo "== the renderer's own honesty rules =="

check "the chosen register leaves the plan key, so a switch does not retire the plan" \
  "$FLOWS" \
  'const directoryRegisterKey = `${identity}|${directoryRegisterChoice}`;' \
  'const directoryRegisterKey = `${identity}`;' \
  ts "$REN" "disables the send and says the answers describe what was picked before"

check "the pending read renders as an empty dropdown" \
  "$FLOWS" \
  "        {options.state === \"pending\" ? (" \
  "        {false ? (" \
  ts "$REN" "says the list is STILL BEING READ"

check "a failed list read is reported as nothing approved" \
  "$FLOWS" \
  '        ) : options.state === "unavailable" ? (' \
  '        ) : false ? (' \
  ts "$REN" "says the read FAILED"

check "the stop-using option is offered with nothing selected" \
  "$FLOWS" \
  '            {allowStop ? (' \
  '            {true ? (' \
  ts "$REN" "does not offer STOP USING when nothing is selected"

check "a superseded plan's card is hidden instead of marked" \
  "$FLOWS" \
  '      {plan ? <ResolverSelectionCard plan={plan} retired={retired} /> : null}' \
  '      {plan && !retired ? <ResolverSelectionCard plan={plan} retired={retired} /> : null}' \
  ts "$REN" "keeps the checked answers ON SCREEN"

check "an unread selection renders as nothing-selected" \
  "$PANEL" \
  '        {plan.description ?? unreadableNotice}' \
  '        {plan.description ?? "No provider directory is selected."}' \
  ts "$REN" "says the selection could not be READ"

echo
echo "== flow 1's Deploy control across the life of a send (the captain's report) =="

LIFE=test/providerDeployButtonLifecycle.test.tsx

# The STATE half. `fresh()` returning null on a spent plan is the whole of what stops a second send.
check "a spent plan still authorizes its button" \
  "$FLOWS" \
  'return held && !held.spent && held.key === key ? held.plan : null;' \
  'return held && held.key === key ? held.plan : null;' \
  ts "$LIFE" "is NOT pressable after the deploy has MINED"

check "the deploy send forgets to retire its plan" \
  "$FLOWS" \
  '                        () => setDeployHeld(spend),' \
  '                        () => {},' \
  ts "$LIFE" "is NOT pressable after the deploy has MINED"

check "a rejected signature retires the plan, stranding a provider who fat-fingered Reject" \
  "$FLOWS" \
  '        if (!timedOut) {' \
  '        retire();
        if (!timedOut) {' \
  ts "$LIFE" "BECOMES pressable again after a rejected signature"

check "the in-flight window leaves the button live" \
  "$FLOWS" \
  '  const ready = !busy && !!reader && isConnected && !!caller;' \
  '  const ready = !!reader && isConnected && !!caller;' \
  ts "$LIFE" "while the wallet has been asked and has not answered"

# The APPEARANCE half - the part the captain actually saw, and the part no state test can see.
check "a disabled filled button keeps its fill (the reported defect, reintroduced)" \
  "$BUTTON" \
  '          inert && FILLED_VARIANTS.has(variant ?? "primary") && INERT_FILL,' \
  '          false && FILLED_VARIANTS.has(variant ?? "primary") && INERT_FILL,' \
  ts "$LIFE" "drops the primary fill once it is disabled"

check "the inert look is applied as a disabled: prefix, so twMerge never strips the fill" \
  "$BUTTON" \
  'const INERT_FILL =
  "border border-border bg-surface-muted text-onSurface shadow-none hover:bg-surface-muted";' \
  'const INERT_FILL =
  "disabled:border disabled:bg-surface-muted disabled:text-onSurface";' \
  ts "$LIFE" "does not rest on opacity alone"

# The RECORD half - PR #155's re-read, which this branch confirms still holds rather than assuming it.
# INVERTED, and the direction matters. Widening the trigger to fire on `submitted` too is HARMLESS - an
# extra re-read costs a request and finds the same answer - so it survives, and counting that as unpinned
# would be the inert-mutation reading this harness exists to avoid. The defect PR #155 closed is the
# opposite: re-read ONLY at submission, where the read comes back before the deploy has mined and, being
# the last trigger, leaves the page stale exactly after the deploy the operator is watching for.
check "the chain is re-read ONLY at submission, so the page stays stale after the deploy" \
  "$FLOWS" \
  '        if (record.state !== "submitted" && record.state !== "awaitingWallet") {' \
  '        if (record.state === "submitted") {' \
  ts "$LIFE" "AFTER the receipt, not at submission"

echo
echo "== the contract claims the portal rests on =="

check "the resolver page removes a deapproved address" \
  "$CORE" \
  '        if (!_resolverListed[kind][resolver]) {
            _resolverListed[kind][resolver] = true;
            _resolverAddresses[kind].push(resolver);
        }' \
  '        if (approved && !_resolverListed[kind][resolver]) {
            _resolverListed[kind][resolver] = true;
            _resolverAddresses[kind].push(resolver);
        }' \
  sol test_the_resolver_page_keeps_an_address_whose_approval_was_withdrawn

check "the registrar can select a resolver on a provider's behalf" \
  "$CORE" \
  '        if (!canWriteProvider(providerId, msg.sender, PROVIDER_PERMISSION_DIRECTORY_RESOLVER)) {
            revert Unauthorized();
        }' \
  '        if (msg.sender != owner() && !canWriteProvider(providerId, msg.sender, PROVIDER_PERMISSION_DIRECTORY_RESOLVER)) {
            revert Unauthorized();
        }' \
  sol test_the_registrar_cannot_select_a_resolver_on_a_providers_behalf

check "a repeat selection is accepted as a no-op" \
  "$CORE" \
  '        address oldResolver = p.directoryResolver;
        if (oldResolver == resolver) revert NoChange();' \
  '        address oldResolver = p.directoryResolver;' \
  sol test_selecting_what_is_already_selected_is_refused_as_a_no_op

echo
echo "== self-test: the harness must refuse a mutation that tested nothing =="
BEFORE_INERT=$INERT
check "SELF-TEST an absent scrutinee is reported INERT" \
  "$ENGINE" 'this string is deliberately not in the source' 'x' ts "$SEL" "offers only the APPROVED entries"
check "SELF-TEST a non-compiling mutation is reported INERT" \
  "$ENGINE" 'const checks: ProviderCheck[] = [];' 'const checks: ProviderCheck[] = [ ;' ts "$SEL" "offers only the APPROVED entries"
SELFTEST_INERT=$((INERT - BEFORE_INERT))

restore
echo
echo "----------------------------------------------------------------------"
echo "pinned: $PASS    unpinned: $FAIL    inert: $INERT (2 expected, from the self-test)"
if [ "$SELFTEST_INERT" -ne 2 ]; then
  echo "SELF-TEST FAILED: expected both self-test mutations to be reported INERT, got $SELFTEST_INERT"
  exit 1
fi
if [ "$FAIL" -ne 0 ] || [ "$INERT" -ne 2 ]; then
  echo "RESULT: not every claim is pinned"
  exit 1
fi
echo "RESULT: every mutation reddened its own named test"
