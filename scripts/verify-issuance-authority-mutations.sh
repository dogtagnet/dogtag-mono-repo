#!/usr/bin/env bash
# Mutation gate for the record-type issuance-authority migration.
#
# Each mutation must redden its OWN named test. A mutation that stays green is reported as a failure
# of THIS script, not waved through: a claim whose test cannot fail is an unpinned claim, and this
# repo has already shipped one inert mutation that read as evidence (see AGENTS.md, "Mutations must
# change behaviour").
#
# Written in bash, never zsh: zsh does not word-split an unquoted "$var", so an array of patterns
# would run as one iteration and every check would silently pass. Same trap AGENTS.md records for
# `swiftc $SRC` and for scripts/check-cutover-consumers.sh.
set -uo pipefail
export LC_ALL=C

cd "$(dirname "$0")/.."
ROOT=$(pwd -P)

TARGETS=(
  "stacks/vet/api/src/routes.rs"
  "stacks/vet/api/src/chain.rs"
  "stacks/government/api/src/chain.rs"
)

BACKUP=$(mktemp -d)
restore() {
  for f in "${TARGETS[@]}"; do
    cp "$BACKUP/$(echo "$f" | tr / _)" "$ROOT/$f"
  done
}
snapshot() {
  for f in "${TARGETS[@]}"; do
    cp "$ROOT/$f" "$BACKUP/$(echo "$f" | tr / _)"
  done
}
trap 'restore; rm -rf "$BACKUP"' EXIT

snapshot

PASS=0
FAIL=0

# apply <file> <old> <new> — asserts the OLD text is present before replacing it. Without that
# assertion a stale pattern silently applies nothing and the run reads as an unpinned claim.
apply() {
  local file="$1" old="$2" new="$3"
  if ! grep -qF -- "$old" "$ROOT/$file"; then
    echo "  !! MUTATION DID NOT APPLY: pattern absent in $file"
    echo "     $old"
    return 1
  fi
  python3 - "$ROOT/$file" "$old" "$new" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
assert s.count(old) == 1, f"expected exactly one occurrence, found {s.count(old)}"
open(path, "w").write(s.replace(old, new))
PY
}

# check <label> <package> <test-target> <test-name> <file> <old> <new>
check() {
  local label="$1" pkg="$2" target="$3" name="$4" file="$5" old="$6" new="$7"
  echo "== $label"
  if ! apply "$file" "$old" "$new"; then
    FAIL=$((FAIL + 1)); restore; return
  fi
  # THE MUTATION MUST STILL COMPILE. A mutation that breaks the build reddens every test in the
  # target, so it would report as evidence while pinning nothing about the named test - the inert
  # mutation AGENTS.md records under "Mutations must change behaviour". Checked rather than trusted,
  # because a typo'd method name looks exactly like a real behavioural mutation in the diff.
  if ! cargo check -p "$pkg" --tests >/dev/null 2>&1; then
    echo "  !! INERT: this mutation does not compile, so its red is a build failure, not evidence"
    FAIL=$((FAIL + 1)); restore; return
  fi
  if cargo test -p "$pkg" --test "$target" "$name" >/dev/null 2>&1; then
    echo "  !! STAYED GREEN — $name does not pin this"
    FAIL=$((FAIL + 1))
  else
    echo "  ok: $name went red"
    PASS=$((PASS + 1))
  fi
  restore
}

# check_lib <label> <package> <test-name> <file> <old> <new>
#
# The same gate against a UNIT test inside `src`. Needed because the Alloy error classification has no
# integration-test seam at all: `MemChain` is a different `ChainClient` and cannot reach that code, so
# the only hermetic pin for it is `chain::tests`. Do not "simplify" these into `check` — `--test <name>`
# addresses an integration target and would silently match nothing.
check_lib() {
  local label="$1" pkg="$2" name="$3" file="$4" old="$5" new="$6"
  echo "== $label"
  if ! apply "$file" "$old" "$new"; then
    FAIL=$((FAIL + 1)); restore; return
  fi
  if ! cargo check -p "$pkg" --tests >/dev/null 2>&1; then
    echo "  !! INERT: this mutation does not compile, so its red is a build failure, not evidence"
    FAIL=$((FAIL + 1)); restore; return
  fi
  if cargo test -p "$pkg" --lib "$name" >/dev/null 2>&1; then
    echo "  !! STAYED GREEN — $name does not pin this"
    FAIL=$((FAIL + 1))
  else
    echo "  ok: $name went red"
    PASS=$((PASS + 1))
  fi
  restore
}

# ---------------------------------------------------------------------------------------------
# (1) the authority must come off the clone, not off the configured registry
# ---------------------------------------------------------------------------------------------
check "preflight reverts to the configured registry" \
  vet-api issuance_authority_migration \
  the_preflight_asks_the_clones_own_authority_not_the_configured_registry \
  stacks/vet/api/src/routes.rs \
  '        .issuance_capability(&issuer_addr, &rt_key, &signer_addr)' \
  '        .issuance_capability(&st.cfg.issuer_registry_addr, &rt_key, &signer_addr)'

# ---------------------------------------------------------------------------------------------
# (2) the rung: canIssue, not isRecognizedIssuer
# ---------------------------------------------------------------------------------------------
check "preflight widened to isRecognizedIssuer" \
  vet-api issuance_authority_migration \
  the_preflight_uses_the_narrow_rung_so_it_refuses_what_the_write_would_refuse \
  stacks/vet/api/src/chain.rs \
  '            let (_recognized, can_issue) = g' \
  '            let (can_issue, _recognized) = g'

# ---------------------------------------------------------------------------------------------
# (3) could-not-determine is neither verdict
# ---------------------------------------------------------------------------------------------
check "Undetermined collapsed into the FORBIDDEN arm" \
  vet-api issuance_authority_migration \
  an_undeterminable_authority_is_not_reported_as_the_signers_fault \
  stacks/vet/api/src/routes.rs \
  '        Ok(IssuanceCapability::Undetermined) => {
            return err(
                StatusCode::BAD_GATEWAY,
                "preflight: could not determine issuance authority for this issuer contract",
            )
        }' \
  '        Ok(IssuanceCapability::Undetermined) => {
            return err(
                StatusCode::FORBIDDEN,
                "address not approved for this recordType yet",
            )
        }'

check "matrix reverts to unwrap_or(false)" \
  vet-api issuance_authority_migration \
  the_signer_matrix_says_null_rather_than_false_when_it_could_not_check \
  stacks/vet/api/src/routes.rs \
  '            Ok(IssuanceCapability::Undetermined) | Err(_) => None,' \
  '            Ok(IssuanceCapability::Undetermined) | Err(_) => Some(false),'

# ---------------------------------------------------------------------------------------------
# (4) confirm asks the past
# ---------------------------------------------------------------------------------------------
check "confirm reverts to the current-state getter" \
  vet-api issuance_authority_migration \
  a_signer_delisted_after_it_anchored_can_still_confirm_its_own_issuance \
  stacks/vet/api/src/routes.rs \
  '        .whitelisted_at_issuance(&issuer_addr, &rt_key, &signer, &r.root)
        .await
    {
        Ok(GrantAtIssuance::Authorized) => {}
        Ok(GrantAtIssuance::NotAuthorized) => {
            return Err(err(
                StatusCode::FORBIDDEN,
                "signer was not whitelisted when this root was anchored",
            ))
        }
        // Never a pass and never an accusation. The remaining binding checks below still have to
        // hold, so a confirm is not waved through on an unanswerable authority.
        Ok(GrantAtIssuance::Undetermined) => {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "whitelist: could not determine the issuing signer'"'"'s authority at anchoring",
            ))
        }
        Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("whitelist: {e}"))),
    }' \
  '        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &rt_key, &signer)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return Err(err(
                StatusCode::FORBIDDEN,
                "signer not whitelisted at confirm",
            ))
        }
        Err(e) => return Err(err(StatusCode::BAD_GATEWAY, &format!("whitelist: {e}"))),
    }'

# ---------------------------------------------------------------------------------------------
# (5) the pillar's generation guard — BOTH directions, on BOTH backends
# ---------------------------------------------------------------------------------------------
check "vet: generation guard dropped (gen-2 accused of forgery)" \
  vet-api verify_credential_issuer_pillar \
  a_generation_two_authority_is_undetermined_not_a_forgery_verdict \
  stacks/vet/api/src/chain.rs \
  '        if history.is_empty()
            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))
        {
            return Ok(GrantAtIssuance::Undetermined);
        }' \
  '        // mutation: guard removed'

check "vet: generation guard made unconditional (gen-1 refusal softened)" \
  vet-api verify_credential_issuer_pillar \
  an_empty_history_on_a_generation_one_registry_is_still_a_definite_refusal \
  stacks/vet/api/src/chain.rs \
  '        if history.is_empty()
            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))
        {' \
  '        if history.is_empty() {'

check "government: generation guard dropped (gen-2 accused of forgery)" \
  government-api flow_memchain \
  a_generation_two_authority_is_undetermined_not_a_forgery_verdict \
  stacks/government/api/src/chain.rs \
  '        if history.is_empty()
            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))
        {
            return Ok(GrantAtIssuance::Undetermined);
        }' \
  '        // mutation: guard removed'

check "government: generation guard made unconditional (gen-1 refusal softened)" \
  government-api flow_memchain \
  an_empty_history_on_a_generation_one_registry_is_still_a_definite_refusal \
  stacks/government/api/src/chain.rs \
  '        if history.is_empty()
            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))
        {' \
  '        if history.is_empty() {'

# ---------------------------------------------------------------------------------------------
# (6) a REVERT and an UNDELIVERED PROBE are different facts
#
# The guard above turns on a probe, and a probe has THREE outcomes, not two. Reading a transport
# failure as "the contract refused it" is this repo's standing defect class - could-not-check rendered
# as a definite answer - reproduced inside the guard built to remove it.
#
# The first two mutations here revert the classifier to the `is_ok()` it replaced, and go through
# `check_lib` because nothing else can reach that code: `MemChain` is a different `ChainClient`, so its
# tests pin the trait's CONTRACT and never this classification. The remaining four mutate the fakes,
# which is what pins the contract half.
# ---------------------------------------------------------------------------------------------
check_lib "vet: every error read as a revert (the old is_ok semantics)" \
  vet-api \
  a_probe_that_could_not_be_delivered_is_undetermined_never_generation_one \
  stacks/vet/api/src/chain.rs \
  '    match e {
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(p)) => {
            p.code == EXECUTION_REVERTED_CODE || p.message.contains(EXECUTION_REVERTED_MESSAGE)
        }
        _ => false,
    }' \
  '    let _ = e;
    true'

check_lib "vet: no error read as a revert (the generation-1 refusal lost)" \
  vet-api \
  only_an_execution_revert_identifies_generation_one \
  stacks/vet/api/src/chain.rs \
  '    match e {
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(p)) => {
            p.code == EXECUTION_REVERTED_CODE || p.message.contains(EXECUTION_REVERTED_MESSAGE)
        }
        _ => false,
    }' \
  '    let _ = e;
    false'

check_lib "government: every error read as a revert (the old is_ok semantics)" \
  government-api \
  a_probe_that_could_not_be_delivered_is_undetermined_never_generation_one \
  stacks/government/api/src/chain.rs \
  '    match e {
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(p)) => {
            p.code == EXECUTION_REVERTED_CODE || p.message.contains(EXECUTION_REVERTED_MESSAGE)
        }
        _ => false,
    }' \
  '    let _ = e;
    true'

# A NODE-LEVEL ERROR IS NOT A CONTRACT ANSWER. Widening back to "any JSON-RPC error response" reads a
# rate limit or an internal error as a revert, which leaves an empty grant history standing as a
# definite forgery verdict - on government's UNAUTHENTICATED POST /v1/verify.
check_lib "vet: any node error read as a revert (not just an execution revert)" \
  vet-api \
  a_node_error_that_is_not_a_revert_is_undetermined_never_generation_one \
  stacks/vet/api/src/chain.rs \
  '    match e {
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(p)) => {
            p.code == EXECUTION_REVERTED_CODE || p.message.contains(EXECUTION_REVERTED_MESSAGE)
        }
        _ => false,
    }' \
  '    matches!(
        e,
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(_))
    )'

check_lib "government: any node error read as a revert (not just an execution revert)" \
  government-api \
  a_node_error_that_is_not_a_revert_is_undetermined_never_generation_one \
  stacks/government/api/src/chain.rs \
  '    match e {
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(p)) => {
            p.code == EXECUTION_REVERTED_CODE || p.message.contains(EXECUTION_REVERTED_MESSAGE)
        }
        _ => false,
    }' \
  '    matches!(
        e,
        alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(_))
    )'

# The FALL-THROUGH is the sharpest arm: on a transport failure the legacy getter would still answer,
# and on a `ProviderRegistry` it answers a confident wrong `false` off the orthogonal VERIFY axis.
check "vet: undelivered probe falls through to the legacy getter" \
  vet-api issuance_authority_migration \
  an_undelivered_generation_probe_never_falls_through_to_the_legacy_getter \
  stacks/vet/api/src/chain.rs \
  '        if g.unreachable_probe_registries.contains(&governing) {
            return Ok(IssuanceCapability::Undetermined);
        }' \
  '        // mutation: fall through on an undelivered probe'

check "vet: undelivered probe leaves the pillar refusal standing" \
  vet-api verify_credential_issuer_pillar \
  an_undelivered_generation_probe_is_undetermined_not_a_forgery_verdict \
  stacks/vet/api/src/chain.rs \
  '            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))' \
  '            && g.provider_registries.contains(&governing)'

check "government: undelivered probe leaves the pillar refusal standing" \
  government-api flow_memchain \
  an_undelivered_generation_probe_is_undetermined_not_a_forgery_verdict \
  stacks/government/api/src/chain.rs \
  '            && (g.provider_registries.contains(&governing)
                || g.unreachable_probe_registries.contains(&governing))' \
  '            && g.provider_registries.contains(&governing)'

echo
echo "mutations reddening their named test: $PASS"
echo "mutations that stayed green or failed to apply: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1
