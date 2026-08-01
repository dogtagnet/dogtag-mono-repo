#!/usr/bin/env bash
# Check the S-13 client-repoint inventory: every tracked file carrying a MOVING address is declared in
# scripts/cutover-consumers.json, and every declared file still carries one.
#
# WHY THIS EXISTS
#
# The cutover's realistic failure mode is not repointing the wrong address. It is repointing 22 of the
# 25 files in the repoint group and shipping the other 3 still aimed at generation 1. Each fails SILENTLY:
# the indexer's anti-spoof gate drops unrecognised emitters with no error (an oversight feed that looks
# merely empty), and a reader still on the generation-1 factory resolves a generation-2 root to
# address(0), which surfaces as an indeterminate issuer-whitelist pillar rather than as a failure.
#
# So the inventory is a CHECKED artifact rather than a list in a document. A hand-maintained list goes
# stale the moment a file is added, and the staleness is invisible until the cutover.
#
# WHY BASH AND NOT ZSH
#
# The repo's default shell is zsh, which does NOT word-split an unquoted "$var". Iterating a
# space-separated address list that way runs ONE iteration with the whole string as the pattern, every
# git grep misses, and the script reports a clean tree. That is a check that passes by not running -
# the same defect class this script exists to catch. bash word-splits, and the arrays below are
# explicit regardless.
#
# WHY FULL ADDRESSES, CASE-INSENSITIVELY
#
# Both halves are load-bearing, and the registry plan's own inventory got both wrong:
#
#   case  - addresses are stored EIP-55-checksummed in some files and lowercased in others (the
#           indexer lowercases). A case-sensitive grep for the checksummed form is blind to every
#           lowercased consumer, which is how the plan's list omitted stacks/indexer/api/src/main.rs -
#           the very file its own analysis of the silent-drop gate is about.
#   full  - an 8-hex-prefix grep matches synthetic addresses that merely share a prefix.
#           packages/ui/test/provenance.test.ts uses 0xED20269E1234567890abcdefABCDEF1234567890, which
#           is not the factory at all.
#
# Elided prose references (0xED20269E...) cannot be matched either way without reintroducing the
# prefix false positives, so they are DECLARED in the manifest and checked for presence instead.
#
# WHY A RETIRED-ADDRESS CHECK TOO
#
# The inventory above is derived by grepping the CURRENT generation-1 address, so it is structurally
# blind to a consumer already holding a SUPERSEDED one - it looks clean because the file carries no
# moving address at all. That blind spot was live: stacks/owner/web/src/lib/config.ts pinned the M5
# VerificationRegistryConsent while calling it "the live" one, so the owner's consent history scanned
# a dead contract and rendered "no consent history" for absent evidence. `retiredAddresses` closes it
# with the SAME declared-allowlist discipline as the inventory: a retired address may only appear in a
# file that says why, and every other carrier is an error.
#
# Usage: scripts/check-cutover-consumers.sh
# Exit:  0 = tree and manifest agree; 1 = they disagree (each disagreement is named).
set -euo pipefail

# LC_ALL=C for the WHOLE script, not just the sorts. `comm` compares with the locale collating
# sequence, so a C-sorted input fed to a UTF-8 `comm` mis-merges: `README.md` sorts after `packages/`
# in en_US.UTF-8 and before `apps/` in C, which makes comm report nearly every file as both undeclared
# AND stale. Setting it here rather than per-command means the gate answers the same thing whatever
# locale the CALLER happens to have - and a gate that emits a dozen bogus errors on its first real
# invocation is a gate that gets distrusted and deleted.
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)"
MANIFEST=scripts/cutover-consumers.json
[ -f "$MANIFEST" ] || { echo "::error:: missing $MANIFEST"; exit 1; }

# Every section below is read through a process substitution, whose failure `set -e` does NOT catch -
# so a manifest missing a key would make that section's loop read nothing and the gate would report a
# clean tree having checked nothing. That is the defect this gate exists to catch, so the sections are
# asserted present ONCE, here, where the failure is loud.
python3 - "$MANIFEST" <<'PY' || { echo "::error:: $MANIFEST is not valid JSON or is missing a required section"; exit 1; }
import json, sys
m = json.load(open(sys.argv[1]))
missing = [k for k in ("movingAddresses", "consumers", "elidedReferences", "doNotMove", "retiredAddresses")
           if k not in m]
if missing:
    print("missing section(s): " + ", ".join(missing), file=sys.stderr)
    raise SystemExit(1)
empty = [k for k in ("movingAddresses", "retiredAddresses") if not m[k]]
if empty:
    print("empty section(s): " + ", ".join(empty) + " - the gate would check nothing", file=sys.stderr)
    print("Emptying a seeded set is the cheapest way to silence a check without a trace.", file=sys.stderr)
    raise SystemExit(1)
# `semanticEquivalence` is the governing condition - an address may be repointed only when the
# successor answers the same question for the same inputs. The manifest's own _doc asserts every entry
# carries a verdict, so assert it here rather than leaving that an unchecked claim: an ABSENT key is an
# error, exactly as for generation2, because absent is not the same as UNVERIFIED. UNVERIFIED is a
# stated verdict; absent is nobody having asked.
unjudged = [a["contract"] for a in m["movingAddresses"] if "semanticEquivalence" not in a]
if unjudged:
    print("movingAddresses entries with no `semanticEquivalence` verdict: " + ", ".join(unjudged),
          file=sys.stderr)
    print("Absent is not UNVERIFIED. Record what the successor answers, and under what caller context.",
          file=sys.stderr)
    raise SystemExit(1)
PY

python3 - "$MANIFEST" <<'PY' > /tmp/.cutover-addrs.$$ || { echo "::error:: $MANIFEST is not valid JSON"; exit 1; }
import json, sys
m = json.load(open(sys.argv[1]))
for a in m["movingAddresses"]:
    # The verdict is TRUNCATED from the manifest's own string, never re-summarized here. A second
    # wording in the script would be free to drift from the first, which is this gate's whole thesis.
    verdict = " ".join(a["semanticEquivalence"].split())
    if len(verdict) > 74:
        verdict = verdict[:73] + "…"
    # An address whose own supersededBy says BLOCKED must say so in the gate's OUTPUT, not only in the
    # file. It is the one fact that decides whether S-14 may touch the address at all.
    step = " ".join(str(a.get("repointStep", "")).split())
    if "BLOCKED" in str(a.get("supersededBy", "")) or "BLOCKED" in step:
        verdict = "BLOCKED - " + verdict
        if len(verdict) > 74:
            verdict = verdict[:73] + "…"
    print(f'{a["contract"]}\t{a["generation1"]}\t{verdict}')
PY

declare -a CONTRACTS=() ADDRS=() VERDICTS=()
while IFS=$'\t' read -r contract addr verdict; do
  CONTRACTS+=("$contract"); ADDRS+=("$addr"); VERDICTS+=("$verdict")
done < /tmp/.cutover-addrs.$$
rm -f /tmp/.cutover-addrs.$$

python3 -c '
import json
m=json.load(open("scripts/cutover-consumers.json"))
print("\n".join(c["path"] for c in m["consumers"]))
' | sort -u > /tmp/.cutover-declared.$$

: > /tmp/.cutover-found.$$
fail=0

# `git grep` searches TRACKED files only, so a file that is about to be committed is invisible to it.
# That is not hypothetical: this manifest went undeclared while it was untracked and the gate reported
# a clean tree the whole time - a check passing by not running, which is the defect this gate exists
# to catch, aimed at itself. So scan the untracked-but-not-ignored set separately.
untracked=$(git ls-files --others --exclude-standard)
if [ -n "$untracked" ]; then
  for i in "${!ADDRS[@]}"; do
    while IFS= read -r f; do
      [ -f "$f" ] || continue
      if grep -qI -i -- "${ADDRS[$i]}" "$f" 2>/dev/null; then
        echo "::error:: UNTRACKED $f carries ${CONTRACTS[$i]}'s address."
        echo "          git grep cannot see it, so committing it would add an undeclared consumer that"
        echo "          this gate would then be blind to. Track and declare it, or remove it."
        fail=1
      fi
    done <<< "$untracked"
  done
fi

echo "Moving addresses (generation 1 -> generation 2), what carries each, and whether the successor"
echo "answers THE SAME QUESTION FOR THE SAME INPUTS - the governing condition for any repoint:"
echo
for i in "${!ADDRS[@]}"; do
  addr="${ADDRS[$i]}"
  # -I skips binaries; -i is the case fix; the pattern is the FULL 40-hex address.
  hits=$(git grep -lI -i -- "$addr" || true)
  n=$(printf '%s' "$hits" | grep -c . || true)
  printf '  %-30s %s  (%s files)\n' "${CONTRACTS[$i]}" "$addr" "$n"
  # The verdict is what decides whether S-14 may touch this address at all, so it is printed here
  # rather than left buried in the manifest - BLOCKED is the most consequential fact in the file.
  printf '  %-30s   %s\n' "" "${VERDICTS[$i]}"
  printf '%s\n' "$hits" | grep . >> /tmp/.cutover-found.$$ || true
done
sort -u /tmp/.cutover-found.$$ -o /tmp/.cutover-found.$$

echo
undeclared=$(comm -23 /tmp/.cutover-found.$$ /tmp/.cutover-declared.$$ || true)
if [ -n "$undeclared" ]; then
  echo "::error:: file(s) carry a moving address but are NOT declared in $MANIFEST."
  echo "          Each is a consumer that a cutover would silently leave on generation 1."
  echo "          Add it with a class (see _consumerClassDoc) - do not delete this check."
  printf '            %s\n' $undeclared
  fail=1
fi

stale=$(comm -13 /tmp/.cutover-found.$$ /tmp/.cutover-declared.$$ || true)
if [ -n "$stale" ]; then
  echo "::error:: file(s) declared in $MANIFEST no longer carry any moving address."
  echo "          Either they were repointed (drop the entry) or the manifest is stale."
  printf '            %s\n' $stale
  fail=1
fi

# Elided prose references: declared rather than matched, so verify each still says what it claims.
while IFS=$'\t' read -r path contract; do
  prefix=$(python3 -c '
import json,sys
m=json.load(open("scripts/cutover-consumers.json"))
print(next(a["generation1"] for a in m["movingAddresses"] if a["contract"]==sys.argv[1])[:10])
' "$contract")
  if ! grep -qi -- "$prefix" "$path" 2>/dev/null; then
    echo "::error:: $path is declared as an elided reference to $contract but no longer mentions $prefix"
    fail=1
  fi
done < <(python3 -c '
import json
m=json.load(open("scripts/cutover-consumers.json"))
for e in m["elidedReferences"]:
    print(e["path"] + "\t" + e["contract"])
')

# A do-not-move address must never be listed as moving. Reusing the SBT and the consent verifier is
# what keeps every existing tag verifiable; moving either is unrecoverable, so it is checked, not noted.
while read -r contract addr; do
  for a in "${ADDRS[@]}"; do
    if [ "$(printf '%s' "$a" | tr 'A-Z' 'a-z')" = "$(printf '%s' "$addr" | tr 'A-Z' 'a-z')" ]; then
      echo "::error:: $contract ($addr) is declared do-not-move AND moving. Reusing it is load-bearing."
      fail=1
    fi
  done
done < <(python3 -c '
import json
m=json.load(open("scripts/cutover-consumers.json"))
for d in m["doNotMove"]:
    print(d["contract"], d["address"])
')

rm -f /tmp/.cutover-found.$$ /tmp/.cutover-declared.$$

# ---- RETIRED ADDRESSES ------------------------------------------------------------------------
# A retired address is one this repo has already superseded, so grepping the CURRENT value can never
# find a file still holding it. Each is declared with an explicit per-address allowlist: an UNDECLARED
# carrier is an error, and a declared carrier that no longer holds it is informational (the allowlist
# has one entry too many, which is not a repointing hazard). The allowlist is scoped PER ADDRESS, so a
# file cleared to carry one retired address is not thereby cleared to carry another.
declare -a RET_ADDRS=() RET_CONTRACTS=()
while IFS=$'\t' read -r contract addr; do
  RET_CONTRACTS+=("$contract"); RET_ADDRS+=("$addr")
done < <(python3 -c '
import json
m=json.load(open("scripts/cutover-consumers.json"))
for r in m["retiredAddresses"]:
    print(r["contract"] + "\t" + r["address"])
')

if [ "${#RET_ADDRS[@]}" -gt 0 ]; then
  echo
  echo "Retired addresses (superseded; may only appear where declared):"
  for i in "${!RET_ADDRS[@]}"; do
    addr="${RET_ADDRS[$i]}"
    python3 -c '
import json,sys
m=json.load(open("scripts/cutover-consumers.json"))
r=next(x for x in m["retiredAddresses"] if x["address"].lower()==sys.argv[1].lower())
print("\n".join(c["path"] for c in r["allowedCarriers"]))
' "$addr" | sort -u > /tmp/.cutover-retallow.$$
    git grep -lI -i -- "$addr" | sort -u > /tmp/.cutover-retfound.$$ || true
    n=$(grep -c . /tmp/.cutover-retfound.$$ || true)
    printf '  %-38s %s  (%s carriers)\n' "${RET_CONTRACTS[$i]}" "$addr" "$n"

    bad=$(comm -23 /tmp/.cutover-retfound.$$ /tmp/.cutover-retallow.$$ || true)
    if [ -n "$bad" ]; then
      echo "::error:: file(s) carry the RETIRED ${RET_CONTRACTS[$i]} address and are not declared as"
      echo "          allowed carriers of it. A retired address reads live, so a consumer holding one"
      echo "          queries a dead contract and reports absence of evidence as evidence of absence."
      echo "          Repoint it, or declare it under retiredAddresses[].allowedCarriers with a reason."
      printf '            %s\n' $bad
      fail=1
    fi
    gone=$(comm -13 /tmp/.cutover-retfound.$$ /tmp/.cutover-retallow.$$ || true)
    if [ -n "$gone" ]; then
      echo "  note: declared carrier(s) no longer hold it - drop the allowlist entry:"
      printf '            %s\n' $gone
    fi

    # Same untracked blind spot as above: git grep cannot see a file about to be committed.
    if [ -n "$untracked" ]; then
      while IFS= read -r f; do
        [ -f "$f" ] || continue
        if grep -qI -i -- "$addr" "$f" 2>/dev/null; then
          echo "::error:: UNTRACKED $f carries the RETIRED ${RET_CONTRACTS[$i]} address."
          fail=1
        fi
      done <<< "$untracked"
    fi
    rm -f /tmp/.cutover-retallow.$$ /tmp/.cutover-retfound.$$
  done
fi

# Surface every per-consumer target that DIVERGES from its address's default. S-14 drives from this
# manifest, and the factory is the one address whose target depends on what the caller does with it -
# readers take the router, writers and the indexer's emitter allowlist take the factory. Printing the
# divergences means that split cannot be missed by reading only the top-level `supersededBy`.
python3 -c '
import json
m=json.load(open("scripts/cutover-consumers.json"))
d=[c for c in m["consumers"] if "supersededBy" in c]
if d:
    print()
    print("Per-consumer targets that DIVERGE from their address default:")
    for c in d:
        print("  " + c["path"])
        print("      -> " + c["supersededBy"])
'

# Report the generation-2 column by READING it. This line used to assert "all null" without the script
# ever touching the field, so a placeholder address would have been announced as absent - an unearned
# claim inside the one artifact whose job is catching unearned claims. A filled value is INFORMATIONAL,
# not a failure: S-14 filling them in is the expected end state. A MISSING key is an error, because a
# property that was never stated must not be reported as a stated null.
#
# `generation2Provenance` is required and PRINTED for the same reason `semanticEquivalence` is. Once
# S-14 filled some entries and not others, a null stopped meaning one thing - not yet deployed,
# deployed with no single successor, or deployed but BLOCKED - and a bare "N of M are FILLED" cannot
# tell those apart. An absent key is an error rather than a silent pass, because a null nobody
# explained reads exactly like a null nobody has got to yet.
python3 - "$MANIFEST" <<'PY' || fail=1
import json, sys, textwrap
m = json.load(open(sys.argv[1]))
missing = [a["contract"] for a in m["movingAddresses"] if "generation2" not in a]
if missing:
    print("::error:: movingAddresses entries with no `generation2` key at all: " + ", ".join(missing))
    print("          Absent is not null. Record it explicitly as null until S-14 deploys.")
    raise SystemExit(1)
unexplained = [a["contract"] for a in m["movingAddresses"] if "generation2Provenance" not in a]
if unexplained:
    print("::error:: movingAddresses entries with no `generation2Provenance`: " + ", ".join(unexplained))
    print("          A null must say WHICH null it is (not deployed / no single successor / BLOCKED),")
    print("          and a filled one must say where the address came from and that it repoints nothing.")
    raise SystemExit(1)
entries = m["movingAddresses"]
filled = [a for a in entries if a["generation2"] is not None]
print()
if not filled:
    print(f"Generation-2 addresses: read {len(entries)}, all null - S-13 repoints, S-14 deploys.")
else:
    print(f"Generation-2 addresses: {len(filled)} of {len(entries)} are FILLED (recorded, NOT repointed):")
    for a in filled:
        print(f"  {a['contract']:<30} {a['generation2']}")
nulls = [a for a in entries if a["generation2"] is None]
if nulls:
    # Printed IN FULL, deliberately. This reason is the ONLY artifact separating the three kinds of
    # null, so a line cap here would cut the distinguishing clause off some entry and - with no marker
    # - the partial output would read as the whole of it. Capping and marking would still hide the
    # clause; there is no length at which this is safe to elide, so it is not elided.
    print(f"Still null ({len(nulls)}), each for its own stated reason:")
    for a in nulls:
        why = " ".join(a["generation2Provenance"].split())
        print(f"  {a['contract']}")
        for line in textwrap.wrap(why, 92, initial_indent="    ", subsequent_indent="    "):
            print(line)
PY

if [ "$fail" -eq 0 ]; then
  echo
  echo "OK: the tree and $MANIFEST agree."
fi
exit "$fail"
