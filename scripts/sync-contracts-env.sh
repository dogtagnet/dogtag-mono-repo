#!/usr/bin/env bash
# Write the terminal's exported environment into contracts/.env - the variables that belong there,
# and only those.
#
#   scripts/sync-contracts-env.sh              # sync the exported shell env into contracts/.env
#   scripts/sync-contracts-env.sh <file>       # same, against another file (tests use this)
#
# WHY THIS EXISTS. The boot scripts read contracts/.env, not your shell. A shell holding the right
# GOVERNANCE_PRIVATE_KEY over a file holding the previous deployment's key boots an admin that owns
# nothing on the new contracts, and every check you run against the SHELL agrees with the ledger
# while the FILE is the thing nobody looked at. This command makes the file say what the terminal
# says. Only EXPORTED variables are visible to it: a bare `NAME=value` assignment is a shell
# variable this process can never see, so the report says `kept: NAME` and the file's old value
# stands - put `export` in front and re-run. Do NOT "fix" that by sourcing contracts/.env itself:
# sourcing the TARGET file loads its OLD values over the value you just set, and the next run then
# truthfully reports `unchanged` on a stale key. (`set -a; source <a file of your own variables>;
# set +a` is fine - just never this script's own target.)
#
# THE ALLOWLIST IS CLOSED, deliberately. Dumping `env` would leak every secret in the shell and
# write a hundred lines nothing reads. The set below is derived from what actually consumes this
# file: demo-up.sh's own reads (ROAX_RPC, GOVERNANCE_PRIVATE_KEY, GOV_SIGNER_KEY, and the
# per-provider clone addresses PROFILE_ISSUER_ADDR / VACCINATION_ISSUER_ADDR /
# TRAVEL_CLEARANCE_ISSUER_ADDR, none of which has a ledger fallback), the backends' DEMO_MODE boot
# guard and government-api's EU_HEALTH_CERT_ISSUER_ADDR (both reach the backends because demo-up.sh
# sources this file with `set -a`), and the GOVERNANCE_ADDRESS line PREREQUISITES.md §2.1 documents
# beside the key. Add a variable here only with a code read to point at.
#
# LEDGER-OWNED ADDRESSES ARE REFUSED, AND REMOVED FROM THE FILE. Every protocol address is resolved
# from contracts/deployments/roax.json only when the environment does not already name one - the
# scripts read `${FACTORY_ADDR:-$(ledger_addr DogTagIssuerFactory)}` and a dozen like it - so such
# a line in this file overrides the ledger SILENTLY and survives a redeploy while naming a contract
# that decides nothing. The refuse list below is the enumeration of those `ledger_addr` fallbacks
# in scripts/*.sh, plus DEPLOYER_PRIVATE_KEY / DEPLOYER_ADDRESS: demo-up.sh resolves its admin as
# `${GOVERNANCE_PRIVATE_KEY:-${DEPLOYER_PRIVATE_KEY:-}}`, so a deployer key in this file silently
# stands in for a missing governance key with a signer that lost WHITELIST_ADMIN in Phase-2.
# Any OTHER exported name shaped `*_ADDR` / `*_ADDRESS` is refused by the same rule; if a new
# per-provider clone variable ever needs syncing, add it to ALLOWED_VARS with its code read.
#
# WHAT IT NEVER DOES: print a value (it reports by NAME - set / unchanged / kept / removed /
# refused), blank an allowlisted variable the shell does not set (the file's value is kept, and
# said), write an empty exported value over a real one, or touch the file when nothing changes
# (running it twice reports no change). Comment lines are preserved. Before any write it copies
# the file to <file>.bak.<utc-timestamp> (mode 600, gitignored by the root `.env.*` rule) and
# prints that path. The file ends at mode 600 - it can hold two private keys.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="${1:-$ROOT/contracts/.env}"

# Synced when exported. Each entry is justified by a code read - see the header.
ALLOWED_VARS="ROAX_RPC DEMO_MODE GOVERNANCE_PRIVATE_KEY GOVERNANCE_ADDRESS GOV_SIGNER_KEY PROFILE_ISSUER_ADDR VACCINATION_ISSUER_ADDR TRAVEL_CLEARANCE_ISSUER_ADDR EU_HEALTH_CERT_ISSUER_ADDR"

# Each of these names sits to the left of a `$(ledger_addr ...)` fallback in scripts/*.sh, so a
# value in the file overrides contracts/deployments/roax.json silently.
LEDGER_OWNED_VARS="FACTORY_ADDR DOGTAG_ISSUER_FACTORY_ADDR ISSUER_REGISTRY_ADDR ISSUER_REGISTRY PROVIDER_REGISTRY_ADDR VERIFICATION_REGISTRY_CONSENT_ADDR SBT_CONSENT_ADDR PROTOCOL_REGISTRY_ADDR DOGTAG_ISSUER_IMPL_ADDR GROTH16_VERIFIER_CONSENT_ADDR PROVIDER_DIRECTORY_ADDR SERVICE_DOMAIN_RESOLVER_ADDR ISSUER_DOMAIN_REGISTRY_ADDR ADMIN_ADDRESS"

# The demo-up.sh admin fallback trap - see the header.
DEPLOYER_TRAP_VARS="DEPLOYER_PRIVATE_KEY DEPLOYER_ADDRESS"

in_list(){ case " $2 " in *" $1 "*) return 0 ;; esac; return 1; }

# allowed | ledger | deployer | shape | other
classify(){
  local k="$1"
  if in_list "$k" "$ALLOWED_VARS"; then echo allowed; return; fi
  if in_list "$k" "$LEDGER_OWNED_VARS"; then echo ledger; return; fi
  if in_list "$k" "$DEPLOYER_TRAP_VARS"; then echo deployer; return; fi
  case "$k" in *_ADDR|*_ADDRESS) echo shape; return ;; esac
  echo other
}

refuse_reason(){
  case "$1" in
    ledger)   echo "ledger-owned: the scripts fall back to contracts/deployments/roax.json only when this is unset, so a value here overrides the ledger silently" ;;
    deployer) echo "demo-up.sh silently falls back to it as the admin signer when GOVERNANCE_PRIVATE_KEY is absent, and that key lost WHITELIST_ADMIN in Phase-2" ;;
    shape)    echo "matches the contract-address shape this file must not carry (docs/DEMO_CLICKS.md section 0.5); if it is a new per-provider clone variable, add it to ALLOWED_VARS with its code read" ;;
  esac
}

# The shell's side: only EXPORTED variables reach this process.
shell_has(){ [ -n "${!1+x}" ]; }
shell_val(){ printf '%s' "${!1-}"; }

# The file's side: its EFFECTIVE values, read the way every consumer reads them - by sourcing in a
# throwaway process (last assignment wins, quoting resolved). Never sourced into this shell.
# shellcheck disable=SC2016  # $0/$1 must expand in the child, not here
file_val(){ env -i /bin/bash --norc --noprofile -c 'set -a; . "$0" >/dev/null 2>&1; k="$1"; printf "%s" "${!k-}"' "$ENV_FILE" "$1"; }

# Extract the KEY of a `KEY=...` / `export KEY=...` line; empty output for anything else.
line_key(){ printf '%s' "$1" | sed -nE 's/^[[:space:]]*(export[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*)=.*/\2/p'; }

# Values are written source-able: bare when safe, single-quoted otherwise. Never printed to the
# terminal either way.
quote_val(){
  case "$1" in
    "") printf "''" ;;
    *[!A-Za-z0-9_./:@+=-]*) printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")" ;;
    *) printf '%s' "$1" ;;
  esac
}

if [ -e "$ENV_FILE" ] && [ ! -f "$ENV_FILE" ]; then
  echo "ERROR: $ENV_FILE exists and is not a regular file" >&2; exit 1
fi
if [ -f "$ENV_FILE" ]; then
  # shellcheck disable=SC2016  # $0 must expand in the child, not here
  env -i /bin/bash --norc --noprofile -c 'set -a; . "$0"' "$ENV_FILE" >/dev/null 2>&1 \
    || { echo "ERROR: $ENV_FILE does not source cleanly as shell - fix the file first, nothing was written" >&2; exit 1; }
fi

# The file is sourced last-wins, so for a key that appears twice the LAST line is the one whose
# value is in force - that is the occurrence we keep, and earlier ones are the duplicates we drop.
LAST_LINES=""
if [ -f "$ENV_FILE" ]; then
  LAST_LINES=" $(
    n=0
    while IFS= read -r line || [ -n "$line" ]; do
      n=$((n + 1))
      k="$(line_key "$line")"
      [ -n "$k" ] && printf '%s:%s\n' "$k" "$n"
    done < "$ENV_FILE" \
      | awk -F: '{ last[$1] = $2 } END { for (k in last) printf " %s:%s", k, last[k] }'
  ) "
fi
is_last_occurrence(){ case "$LAST_LINES" in *" $1:$2 "*) return 0 ;; esac; return 1; }

changed=0
SEEN_IN_FILE=""     # managed keys the file carries (so the append pass skips them)
REPORTED_REFUSED=""
NEW_CONTENT=""
append(){ NEW_CONTENT="${NEW_CONTENT}${1}
"; }
say(){ echo "  $1"; }

echo "sync-contracts-env: $ENV_FILE"

# ---- walk the existing file, preserving comments and unmanaged lines ----------------------------
if [ -f "$ENV_FILE" ]; then
  lineno=0
  while IFS= read -r line || [ -n "$line" ]; do
    lineno=$((lineno + 1))
    key="$(line_key "$line")"
    if [ -z "$key" ]; then
      append "$line"                      # comment, blank, or not a KEY=value line
      continue
    fi
    kind="$(classify "$key")"
    case "$kind" in
      ledger|deployer|shape)
        say "removed: $key - $(refuse_reason "$kind")"
        changed=1
        continue
        ;;
    esac
    if ! is_last_occurrence "$key" "$lineno"; then
      # An earlier duplicate: the value in force is the last line's, which is kept below.
      say "removed: $key (duplicate line - the file is sourced last-wins, the last one is kept)"
      changed=1
      continue
    fi
    case "$kind" in
      allowed)
        SEEN_IN_FILE="$SEEN_IN_FILE $key"
        if shell_has "$key"; then
          sv="$(shell_val "$key")"
          fv="$(file_val "$key")"
          if [ -z "$sv" ]; then
            append "$line"
            say "kept: $key (exported EMPTY in the shell - an empty value is never written over the file's)"
          elif [ "$sv" = "$fv" ]; then
            append "$line"
            say "unchanged: $key"
          else
            append "$key=$(quote_val "$sv")"
            say "set: $key (the file had a different value)"
            changed=1
          fi
        else
          # Not silently blanked - the file's value stands, and we say so, WITH the remedy: this is
          # the line a reader sees when they assigned without `export` and believe they just set it.
          append "$line"
          say "kept: $key (not exported in this shell - the file's value is left alone; if you just set it, run: export $key=<value>  and re-run)"
        fi
        ;;
      other)
        append "$line"
        say "kept: $key (not a variable this script manages - left as-is)"
        ;;
    esac
  done < "$ENV_FILE"
fi

# ---- add allowlisted variables the shell exports and the file lacks -----------------------------
for k in $ALLOWED_VARS; do
  in_list "$k" "$SEEN_IN_FILE" && continue
  if shell_has "$k"; then
    v="$(shell_val "$k")"
    if [ -z "$v" ]; then
      say "kept: $k (exported EMPTY in the shell - an empty value is never written)"
    else
      append "$k=$(quote_val "$v")"
      say "set: $k (was absent from the file)"
      changed=1
    fi
  fi
done

# ---- refuse, by name, anything ledger-owned or trap-shaped that the shell exports ----------------
for name in $(compgen -e | LC_ALL=C sort); do
  kind="$(classify "$name")"
  case "$kind" in
    ledger|deployer|shape)
      in_list "$name" "$REPORTED_REFUSED" && continue
      REPORTED_REFUSED="$REPORTED_REFUSED $name"
      say "refused: $name - $(refuse_reason "$kind")"
      ;;
  esac
done
say "every other exported variable: ignored (the allowlist in this script's header is closed)"

# ---- write only if something changed ------------------------------------------------------------
if [ "$changed" -eq 0 ]; then
  if [ -f "$ENV_FILE" ]; then
    echo "no change: $ENV_FILE already says what the terminal says - file left untouched"
  else
    echo "no change: nothing allowlisted is exported and $ENV_FILE does not exist - nothing written"
  fi
  exit 0
fi

if [ -f "$ENV_FILE" ]; then
  # Never reuse a backup name: two runs inside one second would otherwise overwrite the first
  # run's backup - the copy holding the pre-run content - with the second's. Found by walking
  # docs/DEMO_CLICKS.md sections 0.5 -> 5.3 -> 7.5 back to back.
  BAK="$ENV_FILE.bak.$(date -u +%Y%m%dT%H%M%SZ)"
  n=1
  while [ -e "$BAK" ]; do BAK="$ENV_FILE.bak.$(date -u +%Y%m%dT%H%M%SZ).$n"; n=$((n + 1)); done
  cp -p "$ENV_FILE" "$BAK"
  chmod 600 "$BAK"
  echo "backup: $BAK"
fi

tmp="$(mktemp "$(dirname "$ENV_FILE")/.env.sync.XXXXXX")"
chmod 600 "$tmp"
printf '%s' "$NEW_CONTENT" > "$tmp"
mv "$tmp" "$ENV_FILE"
chmod 600 "$ENV_FILE"
echo "wrote: $ENV_FILE (mode 600)"
