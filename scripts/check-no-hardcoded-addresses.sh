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
#
# WHY THE RETIRED SET IS DERIVED FROM THE LEDGER'S OWN GIT HISTORY
#
# `retired` was hand-curated, and that inverted the guarantee this gate exists to provide: it was
# STRONGEST while a pasted address was still correct, and went BLIND at the exact moment it became
# wrong. A redeploy REPLACES addresses in the ledger in place, so on the next commit every superseded
# address silently drops out of the pattern - and unless a human remembered to move each one into
# `retired` in the same change, every consumer still carrying it stopped being checked. That is the
# same "decays SILENTLY ... right up until the next redeploy" failure named at the top of this file,
# reproduced inside the guard built to prevent it.
#
# Measured on this repo, not reasoned about: 10 addresses the ledger really had published were
# invisible, and three UNDECLARED files were carrying one - two of them shipped source
# (`packages/ui/src/schema/demoData.ts`, `stacks/admin/web/src/lib/governance.ts`), not documentation.
#
# The ledger is version-controlled, so the superseded set is recoverable rather than remembered: every
# revision of the ledger is read and its addresses folded into the pattern. The list stops depending
# on anybody's diligence at redeploy time.
#
# WHY THAT HISTORY IS FILTERED BY chainId, AND WHY AN ABSENT chainId IS INCLUDED
#
# The earliest ledger revision is a LOCAL ANVIL one (`chainId: 31337`) carrying placeholder and
# deterministic-anvil values - among them `0x000...00A1`, which 16 files legitimately use as a
# synthetic test constant. Folding it in would redden all 16 for addresses that were never deployed
# anywhere real, and a gate that cries wolf gets switched off. So a revision is used only when its
# chain is the chain the CURRENT ledger describes.
#
# A revision with NO `chainId` is INCLUDED, deliberately. Skipping narrows the pattern silently, which
# is the failure mode above; including can only ever over-report, which is loud and correctable. No
# revision in this repo lacks one, so the rule is defensive - state it rather than discover it.
#
# WHAT `retired` IS FOR NOW - it is NOT redundant
#
# It is the addresses the ledger NEVER published: contracts provisioned out of band (a per-record-type
# clone deployed by `scripts/demo-provision-government.sh`, say) which no revision of the ledger can
# testify to. Five of its entries are exactly that. The rest are recoverable from history and are kept
# anyway: it costs nothing, and it means a history problem degrades to the old behaviour rather than
# to no behaviour at all.
#
# WHAT THIS GATE STILL CANNOT SEE - a real limit, named rather than left implicit
#
# It can only recognise an address the LEDGER has published. Addresses the chain produced some other
# way - a clone a provider deployed through the self-service portal, a provider id, an EOA from a live
# walk - are outside it, and no ledger-derived pattern can reach them: they are indistinguishable from
# a synthetic fixture by inspection, so catching them would need a chain oracle rather than a grep.
# `docs/DEMO_CLICKS.md` carries nine such addresses. The success line below therefore states the claim
# actually checked, so an "OK" is not read as broader than what ran.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEDGER_REL="contracts/deployments/roax.json"
LEDGER="$ROOT_DIR/$LEDGER_REL"
DEBT="$ROOT_DIR/scripts/address-debt.json"

[[ -f "$LEDGER" ]] || { echo "::error:: ledger not found: $LEDGER"; exit 1; }
[[ -f "$DEBT" ]] || { echo "::error:: debt list not found: $DEBT"; exit 1; }

# Read a newline-separated stream into an array. This is `mapfile -t`, spelled out, because `mapfile`
# is a bash 4 builtin and stock `/bin/bash` on macOS is 3.2 - where it is not a builtin, not on PATH,
# and under `set -euo pipefail` aborts the run at `command not found`. Whether that bites depends on
# which bash the caller happens to resolve, which is exactly the kind of "the gate did not run"
# accident the header above is about. Empty input yields an empty array, as `mapfile` does.
read_lines() {
  local __name="$1" __line
  eval "$__name=()"
  while IFS= read -r __line; do
    eval "$__name+=(\"\$__line\")"
  done
}

# THE LEDGER IS THE ONLY SOURCE. Every 20-byte address it publishes is an address no source file may
# contain. Keys beginning `_` are prose notes; their addresses are historical references, not the
# live set, so they are deliberately excluded.
read_lines LIVE < <(python3 - "$LEDGER" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k, v in d.items():
    if k.startswith("_"):
        continue
    if isinstance(v, str) and v.startswith("0x") and len(v) == 42:
        print(v.lower())
PY
)

read_lines RETIRED < <(python3 - "$DEBT" <<'PY'
import json, sys
for a in json.load(open(sys.argv[1])).get("retired", []):
    print(a.lower())
PY
)

read_lines DECLARED < <(python3 - "$DEBT" <<'PY'
import json, sys
for f in json.load(open(sys.argv[1])).get("stillHardcoded", {}):
    print(f)
PY
)

cd "$ROOT_DIR"

# A ledger that parses to zero addresses cannot produce a pattern, and carrying on from here is worse
# than stopping. Measured, rather than reasoned about: `git grep -lIE` ACCEPTS an empty ERE alternative
# (`|0xbeef`, `0xdead||0xbeef`) and it matches every tracked text file. So an omitted element does not
# fail the search - it makes OFFENDERS the whole tree, and the run blames every undeclared file in the
# repo instead of naming the parse. The realistic causes are a moved or renamed ledger and a change to
# its shape, so name both.
if [[ ${#LIVE[@]} -eq 0 ]]; then
  echo "::error:: no contract addresses parsed out of the ledger: $LEDGER"
  echo "          Expected top-level keys whose value is a 42-character 0x address."
  echo "          The gate cannot run without them, and will not guess."
  exit 1
fi

# A SHALLOW CLONE TRUNCATES `git log`, so the history read below would return a NARROWER pattern with
# no error - a check that passes by not running, which is the one outcome this file exists to prevent.
# Refuse instead of quietly checking less than advertised.
if [[ "$(git rev-parse --is-shallow-repository 2>/dev/null || echo unknown)" != "false" ]]; then
  echo "::error:: this is a shallow (or unreadable) git repository, so the ledger's history cannot be read."
  echo "          The retired-address set is derived from that history; without it this gate would"
  echo "          silently check LESS than it claims. Run \`git fetch --unshallow\` and retry."
  exit 1
fi

# EVERY ADDRESS THE LEDGER HAS EVER PUBLISHED ON THIS CHAIN. See the header for why this is derived
# rather than hand-listed, why it is filtered by chainId, and why an absent chainId is included.
HISTORICAL_RAW="$(python3 - "$LEDGER_REL" <<'PY'
import json, re, subprocess, sys

path = sys.argv[1]
ADDR = re.compile(r'0x[0-9a-fA-F]{40}\Z')

def addresses(doc):
    for k, v in doc.items():
        if k.startswith("_"):
            continue
        if isinstance(v, str) and ADDR.match(v):
            yield v.lower()

current = json.load(open(path))
chain = current.get("chainId")

revs = subprocess.run(["git", "log", "--format=%H", "--", path],
                      capture_output=True, text=True).stdout.split()

found, parsed = set(), 0
for rev in revs:
    blob = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True)
    if blob.returncode != 0:
        continue
    try:
        doc = json.loads(blob.stdout)
    except json.JSONDecodeError:
        continue          # a revision mid-edit; the live ledger is parsed separately and guarded
    parsed += 1
    # Absent chainId is INCLUDED: narrowing silently is the failure, over-reporting is loud.
    if "chainId" in doc and doc["chainId"] != chain:
        continue
    found.update(addresses(doc))

# Zero parsed revisions means the history read produced nothing usable. Mirrors the LIVE guard above:
# carrying on would check less than claimed while reporting success.
if parsed == 0:
    sys.stderr.write("::error:: no parseable revision of %s found in git history.\n" % path)
    sys.stderr.write("          The retired-address set is derived from it; refusing to check less than claimed.\n")
    sys.exit(1)

for a in sorted(found):
    print(a)
PY
)"

# `set -e` aborts the assignment above if python exited non-zero, so reaching here means it ran. An
# EMPTY result would still be dangerous rather than merely useless: an empty array element becomes an
# empty ERE alternative, which matches every tracked file (see the LIVE guard's note). Refuse it.
HISTORICAL=()
if [[ -n "$HISTORICAL_RAW" ]]; then read_lines HISTORICAL <<< "$HISTORICAL_RAW"; fi
if [[ ${#HISTORICAL[@]} -eq 0 ]]; then
  echo "::error:: the ledger's git history yielded no addresses for chain id $(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("chainId"))' "$LEDGER")."
  echo "          At least the committed live set was expected. Refusing to check less than claimed."
  exit 1
fi

# RETIRED may legitimately be empty - that is the goal state, every retired address cleared out of the
# tree. `${A[@]+"${A[@]}"}` expands to NOTHING when empty; the `:-` form would inject an empty string.
# The `grep -v '^$'` is not decoration: one empty element becomes an empty ERE alternative, which
# matches EVERY tracked file, so the run would blame the whole repo instead of naming the fault.
PATTERN="$(printf '%s\n' "${LIVE[@]}" "${HISTORICAL[@]}" ${RETIRED[@]+"${RETIRED[@]}"} \
  | tr 'A-F' 'a-f' | grep -v '^$' | sort -u | paste -sd'|' -)"
ADDR_COUNT="$(printf '%s' "$PATTERN" | tr '|' '\n' | grep -c .)"

# Tracked files only, and never the ledger itself - it is where addresses belong.
read_lines OFFENDERS < <(
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

# State the claim that was actually checked. "OK (3 declared)" read far broader than what ran, which
# is how a reader comes to believe the tree carries no stale address at all - see the header's note on
# what a ledger-derived pattern structurally cannot see.
CHAIN_ID="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("chainId"))' "$LEDGER")"
echo "no-hardcoded-addresses: OK - no undeclared tracked file carries any of the ${ADDR_COUNT} addresses"
echo "  the deploy ledger has published on chain ${CHAIN_ID} (${#LIVE[@]} live, ${#HISTORICAL[@]} from its git"
echo "  history, ${#RETIRED[@]} hand-listed as never-published); ${#DECLARED[@]} file(s) declared as debt."
echo "  NOT checked: addresses the ledger never published (provider-deployed clones, walk EOAs)."
