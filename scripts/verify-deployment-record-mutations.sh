#!/usr/bin/env bash
# Mutation gate for the deploy-record slice: break each claim, watch its own test go red, revert.
#
# WHY THIS EXISTS AS A SCRIPT. Every claim here is a NEGATIVE - "the page does not say you have
# deployed nothing when it could not read", "it does not blame your key", "it does not print the same
# sentence twice" - and a negative assertion passes just as happily against code that never had the
# behaviour at all. The only way to know a test would catch the regression is to reintroduce it.
#
# It reports an unapplied mutation as INERT and exits non-zero rather than counting its green as
# evidence, and a mutation whose target text has moved is a scrutinee that no longer exists, not a
# claim that turned out to be pinned. Self-tested at the end for both failure shapes.
#
# THE MUTATIONS LIVE IN `deployment-record-mutations.py`, not here. The scrutinees are TypeScript
# containing `$`, backticks and backslashes, all of which bash re-interprets inside a heredoc - two
# mutations were silently reported INERT for exactly that reason before they were moved out, and an
# inert mutation reads exactly like an unpinned claim.
#
# COMMIT FIRST. This restores with `git checkout --`, so uncommitted work inside TARGETS is lost.
set -uo pipefail
cd "$(dirname "$0")/.."

TARGETS=(
  packages/ui/src/provider/deploymentHistory.ts
  packages/ui/src/provider/cloneProvenance.ts
  packages/ui/src/provider/actionAvailability.ts
  packages/ui/src/provider/sendOutcome.ts
  packages/ui/src/provider/domainClaim.ts
  packages/ui/src/provider/directoryPlan.ts
  packages/ui/src/domain/ProviderSelfServiceFlows.tsx
)
MUT=scripts/deployment-record-mutations.py

pass=0; fail=0; inert=0
total=$(python3 "$MUT" count)

for ((i = 0; i < total; i++)); do
  IFS=$'\t' read -r file label suite expect < <(python3 "$MUT" describe "$i")

  if ! python3 "$MUT" apply "$i"; then
    printf '  INERT   %s\n            the scrutinee is absent - the mutation changed nothing\n' "$label"
    inert=$((inert + 1)); git checkout -- "${TARGETS[@]}"; continue
  fi

  out=$(cd packages/ui && npx vitest run "$suite" --reporter=basic 2>&1)
  git checkout -- "${TARGETS[@]}"

  # DELIBERATELY NARROW. A bare `SyntaxError` was in this pattern once and it misclassified a real
  # mutation: removing the digits guard makes `BigInt("2a")` throw a RUNTIME SyntaxError, which is
  # exactly the red the test is meant to produce - and reporting that as "does not compile" turned a
  # pinned claim into an inert one. A module that genuinely fails to build says `Transform failed`
  # and takes the whole suite down as a `Failed Suites`, neither of which a thrown error does.
  if grep -qiE 'error TS|Transform failed|Failed Suites' <<<"$out"; then
    printf '  INERT   %s\n            the mutation does not compile, so its red proves nothing\n' "$label"
    inert=$((inert + 1)); continue
  fi
  if grep -qF "$expect" <<<"$out" && grep -qiE '[0-9]+ failed' <<<"$out"; then
    printf '  ok      %s\n            reddens: %s\n' "$label" "$expect"
    pass=$((pass + 1))
  else
    printf '  UNPINNED %s\n            expected a failure naming: %s\n' "$label" "$expect"
    fail=$((fail + 1))
  fi
done

echo
printf 'pinned %d   unpinned %d   inert %d\n' "$pass" "$fail" "$inert"
# The two self-tests at the end of the list are DELIBERATELY inert, so exactly two inert results are
# expected. Any other count means a real mutation failed to apply and its green proved nothing.
if [ "$fail" -ne 0 ] || [ "$inert" -ne 2 ]; then
  echo "FAILED: expected 0 unpinned and exactly 2 inert (the two self-tests)"
  exit 1
fi
echo "OK: every claim reddens its own test, and the harness reports a broken mutation as inert"
