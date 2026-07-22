#!/usr/bin/env bash
# DogTag testnet demo — on-chain bootstrap for ONE business signer.
# After you genesis a vet/groomer backend (its Setup wizard shows the derived signer address), run
# this with that address to FUND it with PLASMA and grant the owner-hidden issuance/verification
# capabilities used by demo-up.sh. (The admin portal "Approve" also grants these capabilities; this
# script is the idempotent fallback and additionally funds gas, which the portal cannot.)
#
#   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
#
# For a verifier relayer, this also whitelists the signer for the VERIFY:<purpose> keys (default:
# grooming_intake boarding_intake daycare_access) so it may relay owner-hidden consent proofs.
# Override the purposes via VERIFY_PURPOSES="a b c".
#
#   VERIFY_PURPOSES="grooming_intake" scripts/demo-bootstrap.sh 0x<signer>
#
# Uses governance signer-1's key in contracts/.env (registry WHITELIST_ADMIN + factory owner + PLASMA
# source). Governance Phase-2 (executed on-chain 2026-07-05, block 123835) moved the governance/admin
# authority off the old deployer EOA 0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96 onto governance signer-1
# 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2; set GOVERNANCE_PRIVATE_KEY (signer-1) in contracts/.env.
set -euo pipefail
SIGNER="${1:?usage: demo-bootstrap.sh <signerAddress>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
set -a
# shellcheck disable=SC1091
source "$ROOT/contracts/.env"
set +a

RPC="${ROAX_RPC:-https://devrpc.roax.net}"
PK="${GOVERNANCE_PRIVATE_KEY:?set GOVERNANCE_PRIVATE_KEY (governance signer-1 0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2) in contracts/.env}"
IR="${ISSUER_REGISTRY_ADDR:-${ISSUER_REGISTRY:-}}"
VR="${VERIFICATION_REGISTRY_CONSENT_ADDR:-}"
SBT="${SBT_CONSENT_ADDR:-}"
PROFILE_ISSUER="${PROFILE_ISSUER_ADDR:-}"
VACCINATION_ISSUER="${VACCINATION_ISSUER_ADDR:-}"

: "${IR:?set ISSUER_REGISTRY_ADDR to the shared deployment used by demo-up.sh}"
: "${VR:?set VERIFICATION_REGISTRY_CONSENT_ADDR to the owner-hidden registry used by demo-up.sh}"
: "${SBT:?set SBT_CONSENT_ADDR to the owner-hidden SBT used by demo-up.sh}"
: "${PROFILE_ISSUER:?set PROFILE_ISSUER_ADDR to the DOG_PROFILE issuer clone used by demo-up.sh}"
: "${VACCINATION_ISSUER:?set VACCINATION_ISSUER_ADDR to the vaccination issuer clone used by demo-up.sh}"

# BN254 r — purpose (bytes32) = keccak256(label) mod r, matching the backend and registry.
R=21888242871839275222246405745257275088548364400416034343698204186575808495617

# Verifier purposes (labels). Overridable; defaults to the three groomer purposes.
VERIFY_PURPOSES="${VERIFY_PURPOSES:-grooming_intake boarding_intake daycare_access}"

lower(){ tr '[:upper:]' '[:lower:]'; }
fail(){ echo "ERROR: $*" >&2; exit 1; }

# Fail before sending governance transactions if the configured owner-hidden contracts are missing
# or split across deployments. demo-up.sh consumes this same address set.
for entry in \
  "IssuerRegistry:$IR" \
  "VerificationRegistryConsent:$VR" \
  "DogTagSBTConsent:$SBT" \
  "DOG_PROFILE issuer:$PROFILE_ISSUER" \
  "VACCINATION issuer:$VACCINATION_ISSUER"; do
  NAME="${entry%%:*}"
  ADDRESS="${entry#*:}"
  [ "$(cast code "$ADDRESS" --rpc-url "$RPC")" != 0x ] || fail "$NAME has no code at $ADDRESS"
done
[ "$(cast call "$VR" 'issuerRegistry()(address)' --rpc-url "$RPC" | lower)" = "$(printf %s "$IR" | lower)" ] \
  || fail "VerificationRegistryConsent is wired to a different IssuerRegistry"
[ "$(cast call "$VR" 'sbt()(address)' --rpc-url "$RPC" | lower)" = "$(printf %s "$SBT" | lower)" ] \
  || fail "VerificationRegistryConsent is wired to a different DogTagSBTConsent"
for ISSUER in "$PROFILE_ISSUER" "$VACCINATION_ISSUER"; do
  [ "$(cast call "$ISSUER" 'registry()(address)' --rpc-url "$RPC" | lower)" = "$(printf %s "$IR" | lower)" ] \
    || fail "issuer clone $ISSUER is wired to a different IssuerRegistry"
done

PROFILE_KEY="$(cast call "$PROFILE_ISSUER" 'recordType()(bytes32)' --rpc-url "$RPC")"
VACCINATION_KEY="$(cast call "$VACCINATION_ISSUER" 'recordType()(bytes32)' --rpc-url "$RPC")"
[ "$(printf %s "$PROFILE_KEY" | lower)" = "$(cast keccak DOG_PROFILE | lower)" ] \
  || fail "PROFILE_ISSUER_ADDR is not a DOG_PROFILE issuer clone"
[ "$(printf %s "$VACCINATION_KEY" | lower)" = "$(cast keccak VACCINATION | lower)" ] \
  || fail "VACCINATION_ISSUER_ADDR is not a VACCINATION issuer clone"

echo "Funding $SIGNER with 0.5 PLASMA for gas…"
cast send "$SIGNER" --value 0.5ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
for ENTRY in "VACCINATION:$VACCINATION_KEY" "DOG_PROFILE:$PROFILE_KEY"; do
  RT="${ENTRY%%:*}"
  KEY="${ENTRY#*:}"
  echo "whitelistFor($RT, $SIGNER)…"
  cast send "$IR" "whitelistFor(bytes32,address)" "$KEY" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
done

# --- DogTagSBTConsent ISSUER_ROLE grant (owner-hidden DOG_PROFILE issuance) -------------------------
# CANONICAL grant is now the ADMIN PORTAL's "Approve application" flow: approving an issuer-application
# whose recordTypes include DOG_PROFILE grants DogTagSBTConsent.ISSUER_ROLE to the signer (see
# stacks/admin/api/src/routes.rs::approve_application). This cast remains as an idempotent FALLBACK so
# the demo still works if you bootstrap without click-through. Skipped if the signer already holds it.
# The vet signer calls mintCustodial(dogTagId, root); no owner address is calldata.
ISSUER_ROLE="$(cast call "$SBT" 'ISSUER_ROLE()(bytes32)' --rpc-url "$RPC")"
if [ "$(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$SIGNER" --rpc-url "$RPC")" = "true" ]; then
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBTConsent: already granted — skipping"
else
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBTConsent…"
  cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  echo "  hasRole(ISSUER): $(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$SIGNER" --rpc-url "$RPC")"
fi

# --- VERIFY-key whitelist (relayer can record owner-hidden consent) ---------------------------------
# verify whitelist key = keccak256(abi.encode("VERIFY:", purpose)) where purpose = keccak256(label) mod r.
# Derivation matches VerificationRegistryConsent._verifyKey and backend verify_key.
for LABEL in $VERIFY_PURPOSES; do
  PURPOSE_B32=$(cast to-uint256 "$(python3 -c "print(int('$(cast keccak "$LABEL")',16) % $R)")")
  VERIFY_KEY=$(cast keccak "$(cast abi-encode 'f(string,bytes32)' 'VERIFY:' "$PURPOSE_B32")")
  echo "whitelistFor(VERIFY:$LABEL, $SIGNER)  [purpose=$PURPOSE_B32 key=$VERIFY_KEY]…"
  cast send "$IR" 'whitelistFor(bytes32,address)' "$VERIFY_KEY" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  echo "  isWhitelistedFor(VERIFY:$LABEL): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$SIGNER" --rpc-url "$RPC")"
done

echo "Done. $SIGNER is funded for owner-hidden issuance + verification. Balance: $(cast from-wei "$(cast balance "$SIGNER" --rpc-url "$RPC")") PLASMA"
echo "isWhitelistedFor(VACCINATION): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$(cast keccak VACCINATION)" "$SIGNER" --rpc-url "$RPC")"
echo "isWhitelistedFor(DOG_PROFILE): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$(cast keccak DOG_PROFILE)" "$SIGNER" --rpc-url "$RPC")"
echo "DogTagSBTConsent: $SBT"
echo "VerificationRegistryConsent: $VR"
