#!/usr/bin/env bash
# DOES NOT WORK AGAINST THE LAUNCH CONTRACT SET, in three independent ways: it calls
# IssuerRegistry.whitelistFor (ProviderRegistry does not implement it on the issuance axis), a
# three-argument createIssuer (the factory takes bytes20,bytes32,uint96), and the factory's owner()
# (the launch factory has none, so the call reverts). Walk docs/DEMO_CLICKS.md section 7.2 instead,
# which does the same job through the product.
# Provision the GOVERNMENT authority to issue TRAVEL_CLEARANCE on LIVE ROAX (chainId 135).
#
# The government stack can always READ the live chain, but anchoring a credential needs three things
# that no other script sets up:
#   1. a government signer EOA        (GOV_SIGNER_KEY)          - funded, so it can pay gas
#   2. that signer whitelisted        (IssuerRegistry.whitelistFor(TRAVEL_CLEARANCE, gov))
#   3. a DogTagIssuer clone to anchor into (TRAVEL_CLEARANCE_ISSUER_ADDR) - DogTagIssuer.issue(R)
#
# Without them government-api runs live-read-only and /issue degrades to dry_run. This script creates
# whatever is missing and is safe to re-run: every step is skipped when already satisfied.
#
#   scripts/demo-provision-government.sh
#
# Writes the resulting GOV_SIGNER_KEY + TRAVEL_CLEARANCE_ISSUER_ADDR into contracts/.env (mode 600) so
# scripts/demo-up.sh picks them up on the next boot. The private key is NEVER printed to stdout.
#
# Requires in contracts/.env: GOVERNANCE_PRIVATE_KEY (holds WHITELIST_ADMIN + owns the factory) and,
# to fund the new signer, either it or DEPLOYER_PRIVATE_KEY with a balance.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ENV_FILE="$ROOT/contracts/.env"
[ -f "$ENV_FILE" ] || { echo "ERROR: $ENV_FILE not found." >&2; exit 1; }
set -a; # shellcheck disable=SC1090
source "$ENV_FILE"; set +a

RPC="${ROAX_RPC:-https://devrpc.roax.net}"
IR="${ISSUER_REGISTRY_ADDR:-${ISSUER_REGISTRY:-}}"
: "${IR:?set ISSUER_REGISTRY_ADDR in contracts/.env}"
: "${GOVERNANCE_PRIVATE_KEY:?set GOVERNANCE_PRIVATE_KEY (must hold WHITELIST_ADMIN and own the factory)}"
# How much to fund the government signer with, in ether-equivalent units of the native token.
GOV_FUND="${GOV_FUND:-0.25}"

die(){ echo; echo "ERROR: $*" >&2; exit 1; }
ledger_addr(){
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\(0x[0-9a-fA-F]\{40\}\)\".*/\1/p" \
    "$ROOT/contracts/deployments/roax.json" 2>/dev/null | head -1
}
lower(){ echo "$1" | tr 'A-Z' 'a-z'; }

# True when the non-negative integer $1 is strictly less than $2. Both operands are normalised through
# `cast --to-dec` first (correct on 0x-hex, a no-op on decimal), then compared by DIGIT COUNT and only
# then lexicographically. That last part is the point: `[ -lt ]` is 64-bit shell arithmetic and aborts
# with "integer expression expected" on anything above 2^63-1 wei (~9.22 ether), so a wei comparison
# cannot use it. Any non-numeric operand is fatal rather than silently false.
dec_lt(){
  local a b
  a="$(cast --to-dec "$1")" || die "cannot normalise '$1' to a decimal integer."
  b="$(cast --to-dec "$2")" || die "cannot normalise '$2' to a decimal integer."
  a="${a//[[:space:]]/}"; b="${b//[[:space:]]/}"
  case "${a}${b}" in ''|*[!0-9]*) die "dec_lt: non-numeric operand ('$1' -> '$a', '$2' -> '$b')." ;; esac
  # strip leading zeros so digit COUNT is a meaningful magnitude test ("0018" vs "25").
  while [ "${#a}" -gt 1 ] && [ "${a:0:1}" = "0" ]; do a="${a:1}"; done
  while [ "${#b}" -gt 1 ] && [ "${b:0:1}" = "0" ]; do b="${b:1}"; done
  if [ "${#a}" -ne "${#b}" ]; then
    [ "${#a}" -lt "${#b}" ]
  else
    [[ "$a" < "$b" ]]
  fi
}

# Append or replace a KEY=value line in contracts/.env, keeping the file at mode 600.
upsert_env(){
  local key="$1" val="$2"
  touch "$ENV_FILE"; chmod 600 "$ENV_FILE"
  if grep -qE "^[[:space:]]*(export[[:space:]]+)?${key}=" "$ENV_FILE"; then
    # Rewrite in place via a temp file so we never depend on GNU vs BSD sed -i semantics.
    local tmp; tmp="$(mktemp)"; chmod 600 "$tmp"
    grep -vE "^[[:space:]]*(export[[:space:]]+)?${key}=" "$ENV_FILE" > "$tmp"
    printf '%s=%s\n' "$key" "$val" >> "$tmp"
    mv "$tmp" "$ENV_FILE"; chmod 600 "$ENV_FILE"
  else
    printf '%s=%s\n' "$key" "$val" >> "$ENV_FILE"
  fi
}

FACTORY="${FACTORY_ADDR:-$(ledger_addr DogTagIssuerFactory)}"
: "${FACTORY:?set FACTORY_ADDR, or add DogTagIssuerFactory to contracts/deployments/roax.json}"

CHAIN_ID_EXPECTED="${CHAIN_ID:-135}"
ACTUAL="$(cast chain-id --rpc-url "$RPC" 2>/dev/null || true)"
[ "$ACTUAL" = "$CHAIN_ID_EXPECTED" ] \
  || die "$RPC reports chainId ${ACTUAL:-unreachable}, expected $CHAIN_ID_EXPECTED. Refusing to provision against the wrong chain."

GOV_ADMIN_ADDR="$(cast wallet address --private-key "$GOVERNANCE_PRIVATE_KEY")"
TC_KEY="$(cast keccak "TRAVEL_CLEARANCE")"

echo "Provisioning the government authority on chainId $ACTUAL:"
echo "  registry        $IR"
echo "  factory         $FACTORY"
echo "  governance      $GOV_ADMIN_ADDR"

# The factory must be bound to the registry we are whitelisting in, or the clone is unreachable.
FACTORY_REGISTRY="$(cast call "$FACTORY" 'registry()(address)' --rpc-url "$RPC" 2>/dev/null || true)"
[ "$(lower "${FACTORY_REGISTRY:-}")" = "$(lower "$IR")" ] \
  || die "factory $FACTORY is bound to registry ${FACTORY_REGISTRY:-<unreadable>}, not $IR."

# ---------------------------------------------------------------------------------------------
# 1. The government signer. Reuse GOV_SIGNER_KEY when already present so re-runs are idempotent and
#    an operator-supplied key is never silently replaced.
# ---------------------------------------------------------------------------------------------
if [ -n "${GOV_SIGNER_KEY:-}" ]; then
  GOV_ADDR="$(cast wallet address --private-key "$GOV_SIGNER_KEY")"
  echo "  gov signer      $GOV_ADDR  (reusing existing GOV_SIGNER_KEY)"
else
  echo "  gov signer      generating a new dedicated government EOA..."
  # A dedicated signer, NOT the governance key: the demo is supposed to show the government as its own
  # authority whose issuance rights were granted by governance, not as the protocol admin itself.
  NEW_KEY="$(cast wallet new --json | sed -n 's/.*"private_key"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$NEW_KEY" ] || die "cast wallet new did not yield a private key."
  GOV_SIGNER_KEY="$NEW_KEY"
  GOV_ADDR="$(cast wallet address --private-key "$GOV_SIGNER_KEY")"
  upsert_env GOV_SIGNER_KEY "$GOV_SIGNER_KEY"
  echo "                  $GOV_ADDR  (key written to contracts/.env, mode 600)"
fi

# ---------------------------------------------------------------------------------------------
# 2. Fund it. Prefer the deployer (it holds the bulk of the demo balance) and fall back to governance.
# ---------------------------------------------------------------------------------------------
GOV_BAL="$(cast balance "$GOV_ADDR" --rpc-url "$RPC")"
WANT_WEI="$(cast to-wei "$GOV_FUND" ether)"
# Wei values routinely exceed what `[ -lt ]` can hold, so the compare goes through dec_lt (above),
# which is exact at 256 bits and DIES on a malformed operand instead of quietly reading as "funded".
if dec_lt "$GOV_BAL" "$WANT_WEI"; then
  FUND_PK="${DEPLOYER_PRIVATE_KEY:-$GOVERNANCE_PRIVATE_KEY}"
  FUND_ADDR="$(cast wallet address --private-key "$FUND_PK")"
  echo "  funding         $FUND_ADDR -> $GOV_ADDR  ($GOV_FUND)"
  cast send "$GOV_ADDR" --value "$WANT_WEI" --private-key "$FUND_PK" --rpc-url "$RPC" --legacy --json \
    | sed -n 's/.*"transactionHash"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/                  tx \1/p'
  GOV_BAL="$(cast balance "$GOV_ADDR" --rpc-url "$RPC")"
fi
echo "  gov balance     $GOV_BAL wei"

# ---------------------------------------------------------------------------------------------
# 3. Whitelist the government signer for TRAVEL_CLEARANCE (needs WHITELIST_ADMIN).
# ---------------------------------------------------------------------------------------------
WL_ROLE="$(cast keccak "WHITELIST_ADMIN")"
HAS_WL="$(cast call "$IR" 'hasRole(bytes32,address)(bool)' "$WL_ROLE" "$GOV_ADMIN_ADDR" --rpc-url "$RPC")"
[ "$HAS_WL" = "true" ] \
  || die "GOVERNANCE_PRIVATE_KEY ($GOV_ADMIN_ADDR) does not hold WHITELIST_ADMIN on $IR - it cannot grant issuance rights."

ALREADY="$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$TC_KEY" "$GOV_ADDR" --rpc-url "$RPC")"
if [ "$ALREADY" = "true" ]; then
  echo "  whitelist       already whitelisted for TRAVEL_CLEARANCE"
else
  echo "  whitelist       whitelistFor(TRAVEL_CLEARANCE, ${GOV_ADDR})..."
  cast send "$IR" 'whitelistFor(bytes32,address)' "$TC_KEY" "$GOV_ADDR" \
    --private-key "$GOVERNANCE_PRIVATE_KEY" --rpc-url "$RPC" --legacy --json \
    | sed -n 's/.*"transactionHash"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/                  tx \1/p'
  [ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$TC_KEY" "$GOV_ADDR" --rpc-url "$RPC")" = "true" ] \
    || die "whitelistFor did not take effect."
fi

# ---------------------------------------------------------------------------------------------
# 4. The TRAVEL_CLEARANCE DogTagIssuer clone. Deterministic address = predictIssuer(recordType,
#    business); `business` is the governance address, matching how the other demo clones were salted.
# ---------------------------------------------------------------------------------------------
CLONE_BUSINESS="${TRAVEL_CLEARANCE_BUSINESS:-$GOV_ADMIN_ADDR}"
PREDICTED="$(cast call "$FACTORY" 'predictIssuer(bytes32,address)(address)' "$TC_KEY" "$CLONE_BUSINESS" --rpc-url "$RPC")"
CODE="$(cast code "$PREDICTED" --rpc-url "$RPC")"
if [ "$CODE" != "0x" ] && [ -n "$CODE" ]; then
  echo "  clone           $PREDICTED  (already deployed)"
else
  FACTORY_OWNER="$(cast call "$FACTORY" 'owner()(address)' --rpc-url "$RPC")"
  [ "$(lower "$FACTORY_OWNER")" = "$(lower "$GOV_ADMIN_ADDR")" ] \
    || die "factory owner is $FACTORY_OWNER, not $GOV_ADMIN_ADDR - createIssuer is onlyOwner."
  echo "  clone           createIssuer(TRAVEL_CLEARANCE) -> ${PREDICTED}..."
  cast send "$FACTORY" 'createIssuer(string,bytes32,address)' \
    "Example Competent Authority" "$TC_KEY" "$CLONE_BUSINESS" \
    --private-key "$GOVERNANCE_PRIVATE_KEY" --rpc-url "$RPC" --legacy --json \
    | sed -n 's/.*"transactionHash"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/                  tx \1/p'
  [ "$(cast code "$PREDICTED" --rpc-url "$RPC")" != "0x" ] || die "createIssuer did not deploy code at $PREDICTED."
  [ "$(cast call "$FACTORY" 'isClone(address)(bool)' "$PREDICTED" --rpc-url "$RPC")" = "true" ] \
    || die "$PREDICTED is not registered as a clone of $FACTORY."
fi
upsert_env TRAVEL_CLEARANCE_ISSUER_ADDR "$PREDICTED"

# The clone reads its whitelist from its own recordType + registry - verify the pairing end to end
# rather than trusting that the two writes above targeted the same registry.
CLONE_REGISTRY="$(cast call "$PREDICTED" 'registry()(address)' --rpc-url "$RPC")"
CLONE_RT="$(cast call "$PREDICTED" 'recordType()(bytes32)' --rpc-url "$RPC")"
[ "$(lower "$CLONE_REGISTRY")" = "$(lower "$IR")" ] || die "clone registry $CLONE_REGISTRY != $IR."
[ "$(lower "$CLONE_RT")" = "$(lower "$TC_KEY")" ] || die "clone recordType $CLONE_RT != TRAVEL_CLEARANCE $TC_KEY."

echo
echo "Government authority provisioned on chainId $ACTUAL:"
echo "  GOV_SIGNER_KEY                 -> $GOV_ADDR (in contracts/.env, not printed)"
echo "  TRAVEL_CLEARANCE_ISSUER_ADDR   -> $PREDICTED"
echo "  whitelisted for TRAVEL_CLEARANCE, clone bound to registry $IR"
echo
echo "Restart the government service to pick these up (scripts/demo-down.sh && scripts/demo-up.sh),"
echo "then GET :44832/health should report backend=\"live\", chainId $ACTUAL, canSign=true."
