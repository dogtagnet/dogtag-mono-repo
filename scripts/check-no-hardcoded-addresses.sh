#!/usr/bin/env bash
# Assert that no consumer HARDCODES a deployed contract address: addresses come from the deploy
# ledger, through configuration, and never from a literal pasted into source.
#
# WHY THIS EXISTS
#
# The captain's requirement is a setup guide you can follow from a clean slate with no hardcoded
# contract address anywhere. That property decays the first time somebody pastes an address back in,
# and it decays SILENTLY: a pasted address is valid-looking, compiles, and keeps working right up
# until the next redeploy, when it points at a contract that decides nothing. This gate is what keeps
# the property true instead of leaving it as a promise in a document.
#
# It is the repurposed `check-cutover-consumers.sh`. That script's cutover framing is dead - there is
# one launch set and nothing to cut over to - but its FUNCTION (assert the tree agrees with a declared
# inventory of address-bearing files, in both directions) is exactly the permanent guard the
# addresses-as-configuration work needs. The reasoning below is inherited, and every part of it was
# paid for by a real failure.
#
# WHY BASH AND NOT ZSH
#
# The repo's default shell is zsh, which does NOT word-split an unquoted "$var". Iterating a
# space-separated address list that way runs ONE iteration with the whole string as the pattern, every
# grep misses, and the script reports a clean tree. That is a check that passes by not running - the
# same defect class this script exists to catch. bash word-splits, and the arrays below are explicit
# regardless.
#
# WHY FULL ADDRESSES, CASE-INSENSITIVELY
#
# Both halves are load-bearing, and a previous hand-built inventory got both wrong:
#
#   case  - addresses are stored EIP-55-checksummed in some files and lowercased in others (the
#           indexer and the government tests lowercase). A case-sensitive grep for the checksummed
#           form is blind to every lowercased consumer.
#   full  - an 8-hex-prefix grep matches synthetic addresses that merely share a prefix, inventing
#           consumers that do not exist.
#
# WHY THE CHECK IS BIDIRECTIONAL
#
# A one-way check ("no undeclared file carries an address") lets the debt list rot: an entry stays
# after its file is cleaned, and the list stops describing the tree. So a declared file that no longer
# carries an address is ALSO an error, naming the entry to delete. The list can then only shrink.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEDGER="$ROOT_DIR/contracts/deployments/roax.json"
DEBT="$ROOT_DIR/scripts/address-debt.json"

[[ -f "$LEDGER" ]] || { echo "::error:: ledger not found: $LEDGER"; exit 1; }
[[ -f "$DEBT" ]] || { echo "::error:: debt list not found: $DEBT"; exit 1; }

# THE LEDGER IS THE ONLY SOURCE. Every 20-byte address it publishes is an address no source file may
# contain. Keys beginning `_` are prose notes; their addresses are historical references, not the
# live set, so they are deliberately excluded.
mapfile -t LIVE < <(python3 - "$LEDGER" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k, v in d.items():
    if k.startswith("_"):
        continue
    if isinstance(v, str) and v.startswith("0x") and len(v) == 42:
        print(v.lower())
PY
)

mapfile -t RETIRED < <(python3 - "$DEBT" <<'PY'
import json, sys
for a in json.load(open(sys.argv[1])).get("retired", []):
    print(a.lower())
PY
)

mapfile -t DECLARED < <(python3 - "$DEBT" <<'PY'
import json, sys
for f in json.load(open(sys.argv[1])).get("stillHardcoded", {}):
    print(f)
PY
)

cd "$ROOT_DIR"
PATTERN="$(printf '%s\n' "${LIVE[@]}" "${RETIRED[@]}" | paste -sd'|' -)"

# Tracked files only, and never the ledger itself - it is where addresses belong.
mapfile -t OFFENDERS < <(
  git grep -lIE -i "$PATTERN" -- \
    ':!contracts/deployments/roax.json' \
    ':!scripts/address-debt.json' \
    ':!scripts/check-no-hardcoded-addresses.sh' \
    2>/dev/null | sort || true
)

fail=0

# 1. Undeclared file carrying an address.
for f in "${OFFENDERS[@]:-}"; do
  [[ -z "$f" ]] && continue
  declared=0
  for d in "${DECLARED[@]:-}"; do [[ "$f" == "$d" ]] && declared=1 && break; done
  if [[ $declared -eq 0 ]]; then
    echo "::error:: $f hardcodes a contract address."
    echo "          Read it from configuration instead; the ledger is the only source."
    echo "          If it genuinely must carry one, declare it in scripts/address-debt.json with a reason."
    fail=1
  fi
done

# 2. Declared file that no longer carries one - the entry is stale and must go, or the list stops
#    describing the tree and the gate starts certifying a shape that has moved on.
for d in "${DECLARED[@]:-}"; do
  [[ -z "$d" ]] && continue
  if [[ ! -e "$d" ]]; then
    echo "::error:: scripts/address-debt.json declares $d, which does not exist. Delete the entry."
    fail=1
    continue
  fi
  found=0
  for f in "${OFFENDERS[@]:-}"; do [[ "$f" == "$d" ]] && found=1 && break; done
  if [[ $found -eq 0 ]]; then
    echo "::error:: scripts/address-debt.json declares $d, which no longer hardcodes an address."
    echo "          Delete the entry - this list may only shrink."
    fail=1
  fi
done

if [[ $fail -ne 0 ]]; then
  echo
  echo "Addresses come from contracts/deployments/roax.json, through configuration."
  exit 1
fi

echo "no-hardcoded-addresses: OK (${#DECLARED[@]} file(s) still declared as debt)"
