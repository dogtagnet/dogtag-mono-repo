#!/usr/bin/env bash
# Stop what demo-up.sh started - all of it, or one role at a time.
#
#   scripts/demo-down.sh              # every role
#   scripts/demo-down.sh vet          # just the vet's backend and portal
#   scripts/demo-down.sh vet groomer
#
# Kills ONLY the pids demo-up.sh recorded. Never by binary name or path: this monorepo is checked out
# many times over and every checkout builds the same binary to the same relative path, so a pattern
# kill reaches whichever instance it happens to hit - including a live one somebody else is using.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ALL_ROLES="indexer admin vet groomer prover government owner"

if [ "$#" -eq 0 ]; then
  ROLES="$ALL_ROLES"
else
  ROLES=""
  for r in "$@"; do
    case " $ALL_ROLES " in
      *" $r "*) ROLES="$ROLES $r" ;;
      *) echo "ERROR: '$r' is not a role. Roles: $ALL_ROLES" >&2; exit 1 ;;
    esac
  done
fi

stopped=0
for r in $ROLES; do
  f="$ROOT/.demo/pids.$r"
  [ -f "$f" ] || continue
  n=0
  while read -r pid; do
    [ -n "$pid" ] || continue
    kill "$pid" 2>/dev/null || true
    n=$((n + 1))
  done < "$f"
  rm -f "$f"
  echo "  $r: stopped $n"
  stopped=$((stopped + n))
done

# A stack started by a demo-up.sh from before per-role pid files recorded everything in one `.demo/pids`.
# Honour it on a full stop, so an older running stack is still killable by its own recorded pids rather
# than by the pattern kill this repo forbids. Only on a full stop: that file says nothing about which
# role a pid belongs to, so a per-role request cannot be answered from it and guessing would kill roles
# the caller did not name.
if [ -f "$ROOT/.demo/pids" ]; then
  if [ "$#" -eq 0 ]; then
    n=0
    while read -r pid; do
      [ -n "$pid" ] || continue
      kill "$pid" 2>/dev/null || true
      n=$((n + 1))
    done < "$ROOT/.demo/pids"
    rm -f "$ROOT/.demo/pids"
    echo "  (pre-role-split .demo/pids: stopped $n)"
    stopped=$((stopped + n))
  else
    echo "  NOTE: .demo/pids exists - a stack started before per-role pid files, which does not record" >&2
    echo "        which role each pid is. Stop that one with no arguments." >&2
  fi
fi

if [ "$stopped" -gt 0 ]; then
  echo "demo stopped."
else
  echo "nothing recorded as running for:$ROLES"
fi
