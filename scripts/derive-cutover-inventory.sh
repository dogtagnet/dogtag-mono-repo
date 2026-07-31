#!/usr/bin/env bash
# Re-derive the cutover migration inventory (plan C-0) from chain events, at a PINNED block.
#
# The inventory is an INPUT to the S-12 rehearsal, so it must be re-derivable rather than
# transcribed: a transcribed root list would assert a fact about the chain that nothing checks.
# Writes contracts/rehearsal/fixtures/historical-roots.json.
#
# WHY THIS SCANS IN CHUNKS RATHER THAN ONE eth_getLogs
#
# A node or proxy that CAPS a log range and truncates instead of erroring returns a short root list,
# and nothing downstream can tell: `rootCount` is computed from the same truncated list, so the fork
# test's `assertEq(roots.length, declared)` is self-consistent by construction and its
# `assertGt(roots.length, 0)` passes. The rehearsal would then report green over fewer roots than the
# chain holds - a check that silently ran over less than it claims, which is precisely the defect
# class this slice exists to eliminate. Today's count is corroborated by the plan's independently
# derived figure, but the NEXT regeneration at a new block has no such corroboration.
#
# So the range is walked in bounded chunks AND cross-checked against a single full-range query. A
# range cap makes the chunked scan complete while the full-range one errors or comes back short: the
# first is reported and tolerated, the second is a REFUSAL. Silence is not one of the outcomes.
#
# Usage: scripts/derive-cutover-inventory.sh [block] [rpc] [chunk-size]
set -euo pipefail

BLOCK="${1:-304000}"
RPC="${2:-https://devrpc.roax.net}"
CHUNK="${3:-25000}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${DERIVE_INVENTORY_OUT:-$ROOT_DIR/contracts/rehearsal/fixtures/historical-roots.json}"

command -v curl >/dev/null 2>&1 || { echo "curl is not on PATH"; exit 1; }

FACTORY=$(python3 -c "import json;print(json.load(open('$ROOT_DIR/contracts/deployments/roax.json'))['DogTagIssuerFactory'])")

echo "deriving from $RPC, factory $FACTORY, blocks 0..$BLOCK in chunks of $CHUNK"

BLOCK="$BLOCK" RPC="$RPC" FACTORY="$FACTORY" OUT="$OUT" CHUNK="$CHUNK" python3 - <<'PY'
import json, os, subprocess, sys

RPC = os.environ["RPC"]
FACTORY = os.environ["FACTORY"]
BLOCK = int(os.environ["BLOCK"])
CHUNK = int(os.environ["CHUNK"])
OUT = os.environ["OUT"]

# keccak256("RootRegistered(bytes32,address)")
TOPIC0 = "0xf2c8e5313899f49c865f2a31f5d51ae837fd0890debcdceea0c47fabde0c519b"


def get_logs(lo, hi):
    """One eth_getLogs. Returns the log list, or raises with the node's own error.

    Transport is `curl`, not `urllib`, and that is not a style preference: devrpc.roax.net answers
    urllib's default `Python-urllib/x.y` user agent with **HTTP 403** while accepting curl's, so a
    urllib rewrite of this script cannot reach the endpoint it exists to read at all.
    """
    payload = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "eth_getLogs",
        "params": [{
            "fromBlock": hex(lo), "toBlock": hex(hi),
            "address": FACTORY, "topics": [TOPIC0],
        }],
    })
    proc = subprocess.run(
        ["curl", "-sS", "--max-time", "120", "-X", "POST", RPC,
         "-H", "content-type: application/json", "-d", payload,
         "-w", "\n%{http_code}"],
        capture_output=True, text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError("curl failed for blocks %d..%d: %s" % (lo, hi, proc.stderr.strip()))
    body, _, code = proc.stdout.rpartition("\n")
    if code.strip() != "200":
        raise RuntimeError("HTTP %s for blocks %d..%d: %s" % (code.strip(), lo, hi, body[:200]))
    try:
        res = json.loads(body)
    except ValueError:
        raise RuntimeError("non-JSON response for blocks %d..%d: %s" % (lo, hi, body[:200]))
    if "result" not in res:
        raise RuntimeError("RPC error for blocks %d..%d: %s" % (lo, hi, res.get("error")))
    return res["result"]


def collect(logs, roots, clones, seen, where):
    for entry in logs:
        root = entry["topics"][1]
        if root in seen:
            # registerRoot is write-once on chain, so a repeat can only be an RPC fault (or an
            # overlapping chunk range) - never a real second registration.
            sys.exit("duplicate root %s in RootRegistered logs (%s) - refusing to write a fixture"
                     % (root, where))
        seen.add(root)
        roots.append(root)
        clones.append("0x" + entry["topics"][2][-40:])


# --- the authoritative scan: bounded chunks, so no single request can be silently capped ----------
roots, clones, seen = [], [], set()
lo = 0
while lo <= BLOCK:
    hi = min(lo + CHUNK - 1, BLOCK)
    collect(get_logs(lo, hi), roots, clones, seen, "blocks %d..%d" % (lo, hi))
    lo = hi + 1
print("chunked scan found %d root(s)" % len(roots))

# --- the independent cross-check: one full-range request over the same span ------------------------
# A truncating endpoint disagrees with the chunked scan here. An endpoint that refuses the range
# outright cannot cross-check, which is stated rather than passed over in silence.
try:
    full = get_logs(0, BLOCK)
except Exception as exc:  # noqa: BLE001 - the endpoint's own refusal is the information
    print("full-range cross-check UNAVAILABLE (%s) - the chunked scan stands alone" % exc)
else:
    full_roots = [entry["topics"][1] for entry in full]
    if full_roots != roots:
        sys.exit(
            "cross-check FAILED: the full-range query returned %d root(s) but the chunked scan "
            "found %d. One of them is truncated - refusing to write a fixture that would be "
            "checked against itself." % (len(full_roots), len(roots))
        )
    print("full-range cross-check agreed: %d root(s)" % len(full_roots))

json.dump({
    "_comment": (
        "Historical anchored roots, derived from RootRegistered on the generation-1 "
        "DogTagIssuerFactory at a pinned block. Regenerate with "
        "scripts/derive-cutover-inventory.sh. NOT hand-edited: the rehearsal asserts each "
        "root resolves through the router to the SAME clone the generation-1 factory names, "
        "so the expected clone is read from the chain rather than trusted from this file."
    ),
    "chainId": 135,
    "rpc": RPC,
    "pinnedBlock": BLOCK,
    "factory": FACTORY,
    "rootCount": len(roots),
    "roots": roots,
    "observedClones": clones,
}, open(OUT, "w"), indent=2)
print("wrote %d roots to %s" % (len(roots), OUT))
PY
