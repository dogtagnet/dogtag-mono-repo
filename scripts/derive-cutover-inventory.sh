#!/usr/bin/env bash
# Re-derive the cutover migration inventory (plan C-0) from chain events, at a PINNED block.
#
# The inventory is an INPUT to the S-12 rehearsal, so it must be re-derivable rather than
# transcribed: a transcribed root list would assert a fact about the chain that nothing checks.
# Writes contracts/rehearsal/fixtures/historical-roots.json.
#
# Usage: scripts/derive-cutover-inventory.sh [block] [rpc]
set -euo pipefail

BLOCK="${1:-304000}"
RPC="${2:-https://devrpc.roax.net}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT_DIR/contracts/rehearsal/fixtures/historical-roots.json"

FACTORY=$(python3 -c "import json;print(json.load(open('$ROOT_DIR/contracts/deployments/roax.json'))['DogTagIssuerFactory'])")
# keccak256("RootRegistered(bytes32,address)")
TOPIC0=0xf2c8e5313899f49c865f2a31f5d51ae837fd0890debcdceea0c47fabde0c519b

echo "deriving from $RPC, factory $FACTORY, blocks 0..$BLOCK"

curl -sS -X POST "$RPC" -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"fromBlock\":\"0x0\",\"toBlock\":\"$(printf '0x%x' "$BLOCK")\",\"address\":\"$FACTORY\",\"topics\":[\"$TOPIC0\"]}]}" \
  | BLOCK="$BLOCK" RPC="$RPC" FACTORY="$FACTORY" OUT="$OUT" python3 -c '
import json, os, sys
res = json.load(sys.stdin)
if "result" not in res:
    sys.exit("RPC error: %s" % res.get("error"))
logs = res["result"]
roots, clones, seen = [], [], set()
for l in logs:
    r = l["topics"][1]
    if r in seen:            # registerRoot is write-once, so a repeat would be an RPC fault
        sys.exit("duplicate root %s in RootRegistered logs - refusing to write a fixture" % r)
    seen.add(r)
    roots.append(r)
    clones.append("0x" + l["topics"][2][-40:])
json.dump({
    "_comment": (
        "Historical anchored roots, derived from RootRegistered on the generation-1 "
        "DogTagIssuerFactory at a pinned block. Regenerate with "
        "scripts/derive-cutover-inventory.sh. NOT hand-edited: the rehearsal asserts each "
        "root resolves through the router to the SAME clone the generation-1 factory names, "
        "so the expected clone is read from the chain rather than trusted from this file."
    ),
    "chainId": 135,
    "rpc": os.environ["RPC"],
    "pinnedBlock": int(os.environ["BLOCK"]),
    "factory": os.environ["FACTORY"],
    "rootCount": len(roots),
    "roots": roots,
    "observedClones": clones,
}, open(os.environ["OUT"], "w"), indent=2)
print("wrote %d roots to %s" % (len(roots), os.environ["OUT"]))
'
