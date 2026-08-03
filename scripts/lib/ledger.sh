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
  # The repo root, derived from this file rather than from the caller's $0, so a script that lives in
  # a subdirectory resolves the same ledger.
  printf '%s\n' "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/contracts/deployments/roax.json"
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
