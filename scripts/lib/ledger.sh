#!/usr/bin/env bash
# The deploy ledger is the ONE source of a contract address. Sourced by every script that needs one.
#
# WHY THIS EXISTS
#
# The captain's requirement is a setup guide you can follow from a clean slate with no hardcoded
# contract address anywhere. Scripts were the quietest place that property decayed: a literal at the
# top of an e2e script is invisible to a reader who is looking at the flow, it compiles into nothing
# that can complain, and it keeps working right up until the set is redeployed - at which point the
# script drives contracts that decide nothing and reports whatever they say.
#
# `contracts/deployments/roax.json` is written BY the deploy. Resolving from it by KEY NAME means a
# redeploy repoints every caller at once, and a key that stops existing fails loudly instead of
# silently naming a dead address.
#
# WHY BASH RATHER THAN THE CALLER'S SHELL
#
# The repo's default shell is zsh, which does not word-split an unquoted "$var" - a hazard already
# recorded in AGENTS.md for exactly this class of helper. Every caller here is a `#!/usr/bin/env bash`
# script, and nothing below relies on splitting.
#
# BUT IT IS ALSO SOURCED BY A HUMAN, AT A ZSH PROMPT, AND THAT USED TO ANSWER EMPTY FOR EVERY KEY.
#
# `docs/DEMO_CLICKS.md` tells a reader to `source scripts/lib/ledger.sh` and then resolve addresses
# with `$(ledger_addr …)`. `BASH_SOURCE` is a bash-only array, so under zsh it is unset,
# `_dogtag_ledger_path` resolved a path two directories above the caller's cwd, and `sed` failed into
# its own `2>/dev/null`. Every key answered empty. What a reader saw was not a diagnosis but
# `cast`'s own `invalid value '' for '[TO]': invalid string length` - measured, not assumed - with
# nothing anywhere naming the ledger. Hence the fallback below.
#
# WHY NOT jq
#
# `jq` is not a declared dependency of this repo and several of these scripts run in environments
# that do not have it. The sed pattern below requires a 40-hex `0x…` VALUE, so it only ever matches a
# real top-level address entry and never surrounding prose.

# shellcheck disable=SC2034
DOGTAG_LEDGER_LIB_LOADED=1

# Absolute path to the ledger. Overridable so a script can be pointed at another chain's ledger.
: "${DOGTAG_LEDGER:=}"

_dogtag_ledger_path() {
  if [ -n "${DOGTAG_LEDGER}" ]; then
    printf '%s\n' "${DOGTAG_LEDGER}"
    return
  fi
  # PREFERRED: the repo root derived from THIS FILE rather than from the caller's $0, so a script that
  # lives in a subdirectory resolves the same ledger wherever it is run from.
  local self="${BASH_SOURCE[0]:-}"
  if [ -n "$self" ]; then
    printf '%s\n' "$(cd "$(dirname "$self")/../.." && pwd)/contracts/deployments/roax.json"
    return
  fi
  # FALLBACK, for a shell with no BASH_SOURCE (zsh at a human prompt): walk up from the caller's cwd.
  # This is deliberately SECOND. A cwd walk that ran first would resolve whichever checkout the caller
  # happens to be standing in - and this monorepo is checked out many times over - so a script would
  # silently read another worktree's ledger. Reached only when the file-relative answer is unavailable,
  # where the caller's own checkout is the only sensible meaning of "the ledger".
  local d="$PWD"
  while [ -n "$d" ] && [ "$d" != "/" ]; do
    if [ -f "$d/contracts/deployments/roax.json" ]; then
      printf '%s\n' "$d/contracts/deployments/roax.json"
      return
    fi
    d="$(dirname "$d")"
  done
  # Nothing found. Print a path that cannot exist rather than an empty string, so `ledger_require`
  # reports the missing KEY and `ledger_addr` answers empty exactly as it does for an absent key -
  # never a silently different failure mode depending on which shell sourced this file.
  printf '%s\n' "/nonexistent/contracts/deployments/roax.json"
}

# ledger_addr <Key> -> the address, or empty when the key is absent.
#
# Keys beginning `_` are prose notes whose addresses are HISTORICAL references rather than the live
# set; the value pattern below cannot match them anyway, because they are nested rather than
# top-level string values, but do not rely on that - ask for a real key.
ledger_addr() {
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\(0x[0-9a-fA-F]\{40\}\)\".*/\1/p" \
    "$(_dogtag_ledger_path)" 2>/dev/null | head -1
}

# ledger_require <Key> [<override>] -> the address, or EXIT 1 naming what is missing.
#
# An explicit override (an env var the operator set) wins, so a script can still be pointed at a
# one-off deployment. What it must never do is fall back to a literal: a wrong address here drives
# real transactions at a contract nobody chose.
ledger_require() {
  local key="$1" override="${2:-}" addr
  addr="${override:-$(ledger_addr "$key")}"
  if [ -z "$addr" ]; then
    echo "ERROR: no address for '$key'." >&2
    echo "       Deploy it and record it in $(_dogtag_ledger_path), or set it explicitly." >&2
    echo "       There is no built-in default: a literal here would name a contract nobody chose." >&2
    exit 1
  fi
  printf '%s\n' "$addr"
}

# ledger_chain_id -> the ledger's chainId, or empty.
ledger_chain_id() {
  sed -n 's/.*"chainId"[[:space:]]*:[[:space:]]*\([0-9]\{1,\}\).*/\1/p' \
    "$(_dogtag_ledger_path)" 2>/dev/null | head -1
}
