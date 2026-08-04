#!/usr/bin/env bash
# Mutation gate for the issuance-list surface — LAYER 2 of the two-layer issuance requirement.
#
# Every claim this slice makes is broken deliberately here, and each mutation must redden exactly one
# NAMED test. A suite that stays green under a mutation is pinning nothing, and a passing suite is
# not evidence on its own.
#
# ---------------------------------------------------------------------------------------------
# IT RESTORES FROM A CONTENT BACKUP, NOT FROM GIT — and that is deliberate
# ---------------------------------------------------------------------------------------------
#
# `scripts/verify-provider-selfservice-mutations.sh` restores with `git checkout --`, which takes git
# as the pristine baseline. Running it over uncommitted edits reverts every targeted file and then
# reports every subsequent mutation as INERT, which reads exactly like a harness bug rather than like
# your work having just been deleted. It has happened in this repo.
#
# This harness copies each target's bytes to a temp dir before touching it and restores from there,
# so it is safe to run over uncommitted work and cannot consult git at all.
#
# ---------------------------------------------------------------------------------------------
# AN INERT MUTATION IS REPORTED, NEVER COUNTED
# ---------------------------------------------------------------------------------------------
#
# A mutation whose scrutinee is absent (the source moved) or that does not COMPILE proves nothing —
# its "red" is a build failure, not a failing assertion. Both are reported INERT and exit 1.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BACKUP="$(mktemp -d)"
trap 'restore_all; rm -rf "$BACKUP"' EXIT

TARGETS=(
  "stacks/vet/api/src/issuance_allowed.rs"
  "stacks/vet/api/src/routes.rs"
  "stacks/vet/api/src/chain.rs"
  "packages/ui/src/signers/roster.ts"
  "packages/ui/src/domain/IssuerSignersPanel.tsx"
)

back_up_all() {
  for f in "${TARGETS[@]}"; do
    mkdir -p "$BACKUP/$(dirname "$f")"
    cp "$f" "$BACKUP/$f"
  done
}
restore_all() {
  for f in "${TARGETS[@]}"; do
    [ -f "$BACKUP/$f" ] && cp "$BACKUP/$f" "$f"
  done
}

PASS=0; INERT=0; SURVIVED=0

# apply <file> <needle> <replacement> — returns 1 when the needle is absent.
apply() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys, pathlib
path, needle, repl = sys.argv[1], sys.argv[2], sys.argv[3]
p = pathlib.Path(path); s = p.read_text()
if needle not in s:
    sys.exit(1)
p.write_text(s.replace(needle, repl, 1))
PY
}

# --- runners. Each must report a NON-COMPILING mutation as INERT rather than counting its red. ----

run_rust_unit() {  # <test-name-filter>
  cargo test -p vet-api --lib "$1" 2>&1
}
run_rust_it() {    # <target> <test-name-filter>
  cargo test -p vet-api --test "$1" "$2" 2>&1
}
run_ts() {         # <file-filter> <test-name-substring>
  ( cd packages/ui && npx vitest run "$1" -t "$2" 2>&1 )
}

# check <label> <expected-test-name> <runner-output-file> <compile-probe-exit>
verdict() {
  local label="$1" out="$2" compiled="$3"
  if [ "$compiled" != "0" ]; then
    echo "  INERT  (mutation does not compile — its red proves nothing)"
    INERT=$((INERT+1)); return
  fi
  if grep -qE "test result: FAILED|[0-9]+ failed|FAIL " "$out"; then
    echo "  caught"
    PASS=$((PASS+1))
  else
    echo "  SURVIVED — the claim is not pinned"
    SURVIVED=$((SURVIVED+1))
  fi
}

mutate_rust() { # <label> <file> <needle> <repl> <kind:lib|test> <target-or-filter> [filter]
  local label="$1" file="$2" needle="$3" repl="$4" kind="$5" a="$6" b="${7:-}"
  echo "== $label"
  if ! apply "$file" "$needle" "$repl"; then
    echo "  INERT  (scrutinee absent from $file — the source moved)"
    INERT=$((INERT+1)); return
  fi
  local out; out="$(mktemp)"
  if [ "$kind" = "lib" ]; then run_rust_unit "$a" > "$out"; else run_rust_it "$a" "$b" > "$out"; fi
  # A build failure is not a failing assertion. `error[E` / `error:` from rustc means INERT.
  # A COMPILE failure and a FAILING TEST both print `error:` — cargo says "error: test failed, to
  # rerun pass …" for the second. Keying on that reported every caught mutation as INERT, which is
  # this repo's could-not-check-rendered-as-an-answer defect inside the harness meant to catch it.
  # The discriminator is whether the test binary RAN: no `test result:` line means it never built.
  local compiled=0
  grep -q "test result:" "$out" || compiled=1
  verdict "$label" "$out" "$compiled"
  rm -f "$out"; restore_all
}

mutate_ts() { # <label> <file> <needle> <repl> <spec-filter> <test-name>
  local label="$1" file="$2" needle="$3" repl="$4" spec="$5" name="$6"
  echo "== $label"
  if ! apply "$file" "$needle" "$repl"; then
    echo "  INERT  (scrutinee absent from $file — the source moved)"
    INERT=$((INERT+1)); return
  fi
  local out; out="$(mktemp)"
  run_ts "$spec" "$name" > "$out"
  local compiled=0
  # A transform failure means the mutation does not compile.
  grep -qE "Failed to (parse|load)|Transform failed|error TS[0-9]+" "$out" && compiled=1
  # A `-t` filter that matched NOTHING skips every test and exits 0 — which reads exactly like a
  # surviving mutation. It is a harness fault: the filter names a test that does not exist.
  if grep -qE "Tests +[0-9]+ skipped|No test files found" "$out"; then
    echo "  INERT  (the test filter '$name' matched no test — the harness is wrong, not the code)"
    INERT=$((INERT+1)); restore_all; rm -f "$out"; return
  fi
  verdict "$label" "$out" "$compiled"
  rm -f "$out"; restore_all
}

back_up_all

echo "--- Rust: the roster fold ---"
mutate_rust "withdrawn collapses into never-admitted (ever_named always false)" \
  stacks/vet/api/src/issuance_allowed.rs \
  "let ever_named = named_set.contains(&address);" \
  "let ever_named = false;" \
  lib "issuance_allowed" ""

mutate_rust "our own signer loses its row (the probe is dropped)" \
  stacks/vet/api/src/issuance_allowed.rs \
  "for address in named_set.iter().cloned().chain(also.map(normalize_addr)) {" \
  "for address in named_set.iter().cloned() {" \
  lib "issuance_allowed" ""

mutate_rust "an unavailable read answers a question about an address" \
  stacks/vet/api/src/issuance_allowed.rs \
  "        let entries = self.entries()?;
        let want = normalize_addr(address);" \
  "        let entries = self.entries().unwrap_or(&[]);
        let want = normalize_addr(address);" \
  lib "issuance_allowed" ""

echo "--- Rust: the route ---"
mutate_rust "a failed read renders as an empty list" \
  stacks/vet/api/src/routes.rs \
  "            Err(e) => RosterRead::Unavailable {
                reason: e.to_string(),
            }," \
  "            Err(_e) => RosterRead::Resolved {
                owner: String::new(),
                entries: vec![],
                active_signer_allowed: Some(false),
            }," \
  test "issuance_allowed_route" "an_unreadable_log"

mutate_rust "a locked backend answers false about a signer it cannot derive" \
  stacks/vet/api/src/routes.rs \
  "                let answered = active.as_deref().and_then(|a| read.allowed(a));" \
  "                let answered = Some(active.as_deref().and_then(|a| read.allowed(a)).unwrap_or(false));" \
  test "issuance_allowed_route" "locked_custody"

mutate_rust "the dog-tag profile clone is dropped from the surface" \
  stacks/vet/api/src/routes.rs \
  "    if !st.cfg.profile_issuer_addr.is_empty() {
        configured.push((\"DOG_PROFILE\".to_string(), st.cfg.profile_issuer_addr.clone()));
    }" \
  "" \
  test "issuance_allowed_route" "the_dog_tag_profile_clone"

mutate_rust "a backend write route appears" \
  stacks/vet/api/src/routes.rs \
  "        .route(\"/issuer/issuance-allowed\", get(issuance_allowed_roster))" \
  "        .route(\"/issuer/issuance-allowed\", get(issuance_allowed_roster).post(issuance_allowed_roster))" \
  test "issuance_allowed_route" "there_is_no_backend_route"

echo "--- Rust: the chain seam ---"
mutate_rust "the log-read failure switch is ignored" \
  stacks/vet/api/src/chain.rs \
  "        if g.failing_issuance_allowed_log_reads {
            return Err(ChainError::Rpc(\"issuance-allowed log read failed\".into()));
        }" \
  "" \
  test "issuance_allowed_route" "an_unreadable_log"

echo "--- TypeScript: what a row claims ---"
mutate_ts "withdrawn and never-admitted collapse" \
  packages/ui/src/signers/roster.ts \
  "  return entry.everNamed ? \"withdrawn\" : \"neverAdmitted\";" \
  "  return \"neverAdmitted\";" \
  signerRoster "distinguishes withdrawn"

mutate_ts "the word 'withdrawn' leaves the DOM" \
  packages/ui/src/signers/roster.ts \
  "  withdrawn: \"Withdrawn\"," \
  "  withdrawn: \"Inactive\"," \
  signerRosterRender "carried by a WORD"

mutate_ts "admitting stops being the owner's alone" \
  packages/ui/src/signers/roster.ts \
  "  if (normalizeAddress(ctx.account ?? \"\") !== normalizeAddress(ctx.read.owner)) {
    return {
      kind: \"otherwiseBlocked\",
      why: \`Only this contract's owner can admit a signer" \
  "  if (false) {
    return {
      kind: \"otherwiseBlocked\",
      why: \`Only this contract's owner can admit a signer" \
  signerRoster "refuses a wallet that is not the owner"

echo "--- TypeScript: what the page renders ---"
# REJECTED AS INERT, and worth recording rather than silently replacing: `state === "submitted" ||
# mayContinueAfter(state)` SURVIVED, and not because the claim is unpinned. In the case that would
# have caught it the receipt promise never resolves, so `follow` never returns and that line is never
# reached at all - the mutation targets a state the code cannot be in there. An unreachable mutation
# proves exactly as little as a non-compiling one.
#
# What IS reachable, and what the claim actually says: only an ESTABLISHED SUCCESS re-reads. Dropping
# the guard makes a REVERTED transaction re-read too, which is the same defect one state along - the
# page would show a fresh list as though the write had landed.
mutate_ts "any outcome re-reads, not only an established success" \
  packages/ui/src/domain/IssuerSignersPanel.tsx \
  "        if (mayContinueAfter(state)) {" \
  "        if (true) {" \
  signerRosterRender "REVERTED transaction does not re-read"

mutate_ts "an unreadable list renders the roster anyway" \
  packages/ui/src/domain/IssuerSignersPanel.tsx \
  "        {read.state === \"unavailable\" ? (" \
  "        {false ? (" \
  signerRosterRender "never as an empty list"

echo
echo "=================================================================="
echo "caught: $PASS   SURVIVED: $SURVIVED   INERT: $INERT"
if [ "$SURVIVED" -ne 0 ] || [ "$INERT" -ne 0 ]; then
  echo "FAILED — a surviving mutation pins nothing; an inert one proves nothing."
  exit 1
fi
echo "OK — every claim is pinned by a test that fails when it is broken."
