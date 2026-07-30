#!/usr/bin/env bash
#
# S-12: rehearse the whole registry cutover against a FORK of the live chain, and capture the exact
# transaction list the live cutover will replay.
#
# Deploys NOTHING live. It forks ROAX into a local anvil, broadcasts the cutover sequence into that
# fork, and runs the six assertions. A fork is used rather than a testnet deploy because it answers
# what the DEPLOYED BYTECODE PERMITS rather than what the source suggests, and because
# `broadcast/**/run-latest.json` then IS the transaction list.
#
# Preconditions are checked HERE, in the shell, where a failure is a real non-zero exit — the same
# reason `make test-consent-parity` exists rather than a bare cargo invocation. Nothing self-skips.
#
#   ANVIL IS KILLED ONLY AFTER ITS RECORDED IDENTITY IS RE-VERIFIED, AND NEVER BY NAME OR PATTERN.
#   Never `pkill -f anvil` here: many checkouts of this monorepo run their own, and killing by
#   pattern has taken down a live service in this fleet three times. A bare pid file has the same
#   failure mode once the pid is recycled, so the record carries pid + port + process start time and
#   every field is re-checked before anything is signalled - see the identity-record note below.
#
# Usage: scripts/rehearse-cutover.sh [--port N] [--block N] [--rpc URL]
#        scripts/rehearse-cutover.sh --stop        # verify-then-kill a stranded fork
set -euo pipefail

PORT=8929
BLOCK=304000
UPSTREAM_RPC="https://devrpc.roax.net"

STOP_ONLY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --port)  PORT="$2"; shift 2 ;;
    --block) BLOCK="$2"; shift 2 ;;
    --rpc)   UPSTREAM_RPC="$2"; shift 2 ;;
    --stop)  STOP_ONLY=1; shift ;;
    *) echo "unknown argument: $1"; exit 2 ;;
  esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR/contracts"

fail() { echo "REHEARSAL PRECONDITION FAILED: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------------------------
# The anvil identity record, and why it is a RECORD rather than a pid
#
# A file holding a bare pid GOES STALE the moment that process exits, and the OS reuses pids. An
# operator who then follows the file's own instructions kills whichever unrelated process inherited
# that number - the wrong-process-kill hazard AGENTS.md records as having taken down live services in
# this fleet three times, arriving through a stale record instead of a pattern match.
#
# So the file records IDENTITY, not a number: pid, port, and the kernel's start time for that pid.
# `stop_recorded_anvil` re-verifies EVERY field against the live process before it signals anything,
# and refuses loudly on any disagreement. The file is also removed on exit - but the removal is the
# convenience and the re-verification is the actual fix, because removal cannot happen on an
# abnormal exit, which is precisely the case the file exists for.
#
# Nothing here ever matches on a binary name or path.
# ---------------------------------------------------------------------------------------------
IDENTITY_FILE="$ROOT_DIR/.anvil-rehearsal.pid"

process_start_time() { ps -o lstart= -p "$1" 2>/dev/null | tr -s ' ' | sed 's/^ *//;s/ *$//'; }

record_anvil_identity() {
  local pid="$1" port="$2"
  printf 'pid=%s\nport=%s\nstarted=%s\n' "$pid" "$port" "$(process_start_time "$pid")" \
    > "$IDENTITY_FILE"
}

# Verify the recorded identity still describes a live anvil, then kill it. Refuses rather than
# guesses. Returns non-zero without signalling anything when the record cannot be trusted.
stop_recorded_anvil() {
  [ -f "$IDENTITY_FILE" ] || { echo "no anvil identity record at $IDENTITY_FILE"; return 1; }

  local pid port started
  pid=$(sed -n 's/^pid=//p'     "$IDENTITY_FILE")
  port=$(sed -n 's/^port=//p'    "$IDENTITY_FILE")
  started=$(sed -n 's/^started=//p' "$IDENTITY_FILE")

  if [ -z "$pid" ] || [ -z "$port" ] || [ -z "$started" ]; then
    echo "REFUSING to kill: identity record is incomplete - not signalling anything"; return 1
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "recorded pid $pid is not running (already stopped); removing the stale record"
    rm -f "$IDENTITY_FILE"; return 0
  fi

  # The decisive check: a recycled pid has a different start time, so this is what makes the record
  # unable to mislead. Never skip it.
  local now_started
  now_started=$(process_start_time "$pid")
  if [ "$now_started" != "$started" ]; then
    echo "REFUSING to kill pid $pid: start time is '$now_started' but the record says '$started'."
    echo "That pid has been REUSED by a different process. Not signalling anything."
    return 1
  fi
  # And it must still be the listener we started, on the port we started it on.
  if ! lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null | grep -qx "$pid"; then
    echo "REFUSING to kill pid $pid: it is not the process listening on port $port."
    return 1
  fi

  kill "$pid" 2>/dev/null || true
  for _ in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
  kill -0 "$pid" 2>/dev/null && kill -KILL "$pid" 2>/dev/null || true
  rm -f "$IDENTITY_FILE"
  echo "stopped the verified anvil this script started (pid $pid, port $port)"
}

if [ "$STOP_ONLY" -eq 1 ]; then
  stop_recorded_anvil
  exit $?
fi

for bin in anvil forge cast; do
  command -v "$bin" >/dev/null 2>&1 || fail "$bin is not on PATH (install Foundry)"
done
[ -d lib/forge-std ] && [ -n "$(ls -A lib/forge-std 2>/dev/null)" ] \
  || fail "contracts/lib/forge-std is empty - run: git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts"

# The fixture and the fork must describe the same chain state, or assertion 1 checks a different
# block than the one it names.
FIXTURE=rehearsal/fixtures/historical-roots.json
[ -f "$FIXTURE" ] || fail "$FIXTURE is missing - run scripts/derive-cutover-inventory.sh"
PINNED=$(python3 -c "import json;print(json.load(open('$FIXTURE'))['pinnedBlock'])")
[ "$PINNED" = "$BLOCK" ] \
  || fail "historical-root fixture is pinned to block $PINNED but the fork is at $BLOCK - regenerate with: scripts/derive-cutover-inventory.sh $BLOCK"

if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  fail "port $PORT is already in use - pass --port to pick your own, and do not assume a listener is yours"
fi

LOG="$(mktemp)"
ANVIL_PID=""
cleanup() {
  # Goes through the SAME identity-verified path an operator would use, so the normal exit cannot
  # take a shortcut the recovery path does not have. No pattern matching, ever.
  if [ -n "$ANVIL_PID" ]; then
    stop_recorded_anvil || echo "left pid $ANVIL_PID alone - its identity record could not be trusted"
  fi
  rm -f "$LOG" "$IDENTITY_FILE"
}
trap cleanup EXIT INT TERM

echo "forking $UPSTREAM_RPC at block $BLOCK onto 127.0.0.1:$PORT"
# --auto-impersonate is what lets `forge script --unlocked --sender <governance>` sign as the real
# governance key holder without that key. It is the whole reason a fork can rehearse a governed step.
anvil --fork-url "$UPSTREAM_RPC" --fork-block-number "$BLOCK" --port "$PORT" \
      --auto-impersonate --silent >"$LOG" 2>&1 &
ANVIL_PID=$!
record_anvil_identity "$ANVIL_PID" "$PORT"

FORK_RPC="http://127.0.0.1:$PORT"
for _ in $(seq 1 40); do
  if cast chain-id --rpc-url "$FORK_RPC" >/dev/null 2>&1; then break; fi
  sleep 0.5
done
cast chain-id --rpc-url "$FORK_RPC" >/dev/null 2>&1 || { cat "$LOG"; fail "anvil never came up on $FORK_RPC"; }
[ "$(cast chain-id --rpc-url "$FORK_RPC")" = "135" ] || fail "fork is not chain 135"

GOV=$(python3 -c "import json;print(json.load(open('deployments/roax.json'))['admin'])")
echo "governance signer (impersonated on the fork): $GOV"
cast rpc anvil_setBalance "$GOV" 0x56BC75E2D63100000 --rpc-url "$FORK_RPC" >/dev/null

echo
echo "=== the assertions, against a fork of the UNMODIFIED chain at block $BLOCK ==="
#
# Deliberately forked from UPSTREAM rather than from the anvil below, and the order matters too.
#
# The broadcast MUTATES that anvil: it deploys the whole generation-2 set into it, and the issuer
# implementation lands on exactly the address the test's own `new DogTagIssuerV2()` would take.
# Asserting against the mutated node asks whether the cutover works on a chain where the cutover has
# already happened, which is not the question — and it fails in the confusing direction, as a
# CREATE-collision `ImplementationCodeMismatch` rather than as anything about the cutover.
#
# So: assertions first, against pristine state; the broadcast afterwards, purely to capture the list.
ROAX_FORK_RPC="$UPSTREAM_RPC" FOUNDRY_PROFILE=rehearsal forge test --match-path 'rehearsal/*' \
  || fail "a cutover assertion failed"

echo
echo "=== broadcasting the cutover sequence (produces the transaction list) ==="
#
# `--skip-simulation` is REQUIRED here and is not a way of ignoring a failure.
#
# OBSERVED, on this Foundry (1.5.1) with `--unlocked --sender <governance>`: forge runs the script
# twice. The simulation attributes every CREATE to the governance account — its addresses are exactly
# `governance@nonce` (checked with `cast compute-address`). The second, on-chain re-execution
# produces DIFFERENT CREATE addresses, and the factory constructor is handed the SIMULATION's
# implementation address, which holds no code in that re-execution — so it reverts
# `ImplementationCodeMismatch` while the transactions themselves are perfectly valid.
#
# The exact attribution rule for that second phase was NOT established (the divergent address is
# neither a governance-nonce address nor the script contract's first few), so this comment states
# what was measured and stops there. Skipping the redundant re-execution broadcasts the simulated,
# correctly-attributed list.
#
# Because that removes a check, the receipts and the resulting state are verified below instead —
# every status, the immutable rootIndex binding, and all historical roots. The success banner is not
# taken as evidence of anything.
FOUNDRY_PROFILE=default forge script script/RehearseCutover.s.sol:RehearseCutover \
  --rpc-url "$FORK_RPC" --broadcast --unlocked --sender "$GOV" --legacy --skip-simulation \
  || fail "the cutover sequence did not broadcast cleanly"

LIST="broadcast/RehearseCutover.s.sol/135/run-latest.json"
[ -f "$LIST" ] || fail "no transaction list was written"

echo
echo "=== verifying the BROADCAST result, not the banner ==="
python3 - "$LIST" <<'PY' || fail "a broadcast transaction did not succeed"
import json, sys
d = json.load(open(sys.argv[1]))
txs, receipts = d["transactions"], d["receipts"]
if len(txs) != len(receipts):
    sys.exit("transaction/receipt count mismatch - cannot say every step landed")
bad = [(t.get("contractName"), r["status"]) for t, r in zip(txs, receipts) if r["status"] != "0x1"]
if bad:
    sys.exit("transactions did not succeed: %s" % bad)
print("all %d broadcast transactions succeeded" % len(txs))
PY

ROUTER=$(python3 -c "
import json;d=json.load(open('$LIST'))
print(next(t['contractAddress'] for t in d['transactions'] if t.get('contractName')=='CloneProvenanceRouter'))")
VREG=$(python3 -c "
import json;d=json.load(open('$LIST'))
print(next(t['contractAddress'] for t in d['transactions'] if t.get('contractName')=='VerificationRegistryConsent'))")
FACTORY1=$(python3 -c "import json;print(json.load(open('deployments/roax.json'))['DogTagIssuerFactory'])")

BOUND=$(cast call "$VREG" 'rootIndex()(address)' --rpc-url "$FORK_RPC")
[ "$(echo "$BOUND" | tr 'A-Z' 'a-z')" = "$(echo "$ROUTER" | tr 'A-Z' 'a-z')" ] \
  || fail "the broadcast registry's immutable rootIndex is $BOUND, not the router $ROUTER"
echo "registry rootIndex is the router: $BOUND"

# The property the whole cutover exists to preserve, checked against the ACTUALLY BROADCAST router.
MISMATCH=0
for root in $(python3 -c "
import json;print('\n'.join(json.load(open('$FIXTURE'))['roots']))"); do
  VIA_ROUTER=$(cast call "$ROUTER" 'rootIssuer(bytes32)(address)' "$root" --rpc-url "$FORK_RPC")
  VIA_GEN1=$(cast call "$FACTORY1" 'rootIssuer(bytes32)(address)' "$root" --rpc-url "$FORK_RPC")
  if [ "$VIA_ROUTER" != "$VIA_GEN1" ] || [ "$VIA_ROUTER" = "0x0000000000000000000000000000000000000000" ]; then
    echo "  MISMATCH $root: router=$VIA_ROUTER generation1=$VIA_GEN1"
    MISMATCH=$((MISMATCH + 1))
  fi
done
[ "$MISMATCH" -eq 0 ] || fail "$MISMATCH historical root(s) do not resolve through the broadcast router"
echo "all $(python3 -c "import json;print(json.load(open('$FIXTURE'))['rootCount'])") historical roots resolve through the broadcast router to their original clone"

echo
echo "transaction list: contracts/broadcast/RehearseCutover.s.sol/135/run-latest.json"
echo "render it with:   scripts/render-cutover-txlist.py"
echo "if a fork is ever stranded, stop it with: scripts/rehearse-cutover.sh --stop"
