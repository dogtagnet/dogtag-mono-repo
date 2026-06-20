#!/usr/bin/env bash
# DogTag testnet demo — on-chain bootstrap for ONE business signer.
# After you genesis a vet/groomer backend (its Setup wizard shows the derived signer address),
# run this with that address to FUND it with PLASMA + WHITELIST it for the demo record types,
# so it can issue against the pre-created clones on ROAX. (The admin portal "Approve" also
# whitelists; this script additionally funds gas, which the portal cannot.)
#
#   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
#
# For a GROOMER relayer doing on-chain ZK verification, this ALSO whitelists the signer for the
# VERIFY:<purpose> keys (default: grooming_intake boarding_intake daycare_access) so it may relay
# `recordVerificationZK` on-chain (registry `!verify-wl` require). Override the purposes via
# VERIFY_PURPOSES="a b c".
#
# Optionally fund the OWNER's mobile wallet (for the one-time on-chain consent-key bind) by setting
# OWNER_WALLET=0x... — backward compatible: if unset, that step is skipped.
#
#   VERIFY_PURPOSES="grooming_intake" OWNER_WALLET=0x<ownerWallet> scripts/demo-bootstrap.sh 0x<signer>
#
# Uses the deployer key in contracts/.env (registry WHITELIST_ADMIN + PLASMA source + factory owner).
set -euo pipefail
SIGNER="${1:?usage: demo-bootstrap.sh <signerAddress>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
set -a; source "$ROOT/contracts/.env"; set +a
RPC="$ROAX_RPC"; PK="$DEPLOYER_PRIVATE_KEY"
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c   # IssuerRegistry (deployments/roax.json)
SBT=0x1FB8986573Ac36d532cF7d5a5352202B094D4233  # DogTagSBT (the vet mints DOG_PROFILE here)

# BN254 r — purpose (bytes32) = keccak256(label) mod r, matching e2e-smoke.sh:144 + the registry.
R=21888242871839275222246405745257275088548364400416034343698204186575808495617

# Optional second wallet (the owner's mobile wallet) gets a small amount of PLASMA so it can pay gas
# for the one-time ConsentKeyRegistry.bindConsentKey self-bind. Unset -> skip.
OWNER_WALLET="${OWNER_WALLET:-}"

# Groomer VERIFY purposes (labels). Overridable; defaults to the three groomer purposes.
VERIFY_PURPOSES="${VERIFY_PURPOSES:-grooming_intake boarding_intake daycare_access}"

echo "Funding $SIGNER with 0.5 PLASMA for gas…"
cast send "$SIGNER" --value 0.5ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
for RT in VACCINATION DOG_PROFILE SERVICE_ATTESTATION; do
  KEY=$(cast keccak "$RT")
  echo "whitelistFor($RT, $SIGNER)…"
  cast send "$IR" "whitelistFor(bytes32,address)" "$KEY" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
done

# --- DogTagSBT ISSUER_ROLE grant (the VET now ISSUES DOG_PROFILE SBTs) ------------------------------
# CANONICAL grant is now the ADMIN PORTAL's "Approve application" flow: approving an issuer-application
# whose recordTypes include DOG_PROFILE grants DogTagSBT.ISSUER_ROLE to the signer (see
# stacks/admin/api/src/routes.rs::approve_application). This cast remains as an idempotent FALLBACK so
# the demo still works if you bootstrap without click-through. Skipped if the signer already holds it.
# The vet signer mints DogTagSBT.mint(to,id,root), which is onlyRole(ISSUER_ROLE).
ISSUER_ROLE=$(cast keccak ISSUER)
if [ "$(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$SIGNER" --rpc-url "$RPC")" = "true" ]; then
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBT: already granted — skipping"
else
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBT…"
  cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  echo "  hasRole(ISSUER): $(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$SIGNER" --rpc-url "$RPC")"
fi

# --- VERIFY-key whitelist (groomer relayer can record `recordVerificationZK`) -----------------------
# verify whitelist key = keccak256(abi.encode("VERIFY:", purpose)) where purpose = keccak256(label) mod r.
# Derivation matches e2e-smoke.sh:144-146 byte-for-byte (and backend verify::verify_key).
for LABEL in $VERIFY_PURPOSES; do
  PURPOSE_B32=$(cast to-uint256 "$(python3 -c "print(int('$(cast keccak "$LABEL")',16) % $R)")")
  VERIFY_KEY=$(cast keccak "$(cast abi-encode 'f(string,bytes32)' 'VERIFY:' "$PURPOSE_B32")")
  echo "whitelistFor(VERIFY:$LABEL, $SIGNER)  [purpose=$PURPOSE_B32 key=$VERIFY_KEY]…"
  cast send "$IR" 'whitelistFor(bytes32,address)' "$VERIFY_KEY" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  echo "  isWhitelistedFor(VERIFY:$LABEL): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$SIGNER" --rpc-url "$RPC")"
done

# --- Optional: fund the owner's mobile wallet for the one-time consent-key bind ---------------------
if [ -n "$OWNER_WALLET" ]; then
  echo "Funding owner wallet $OWNER_WALLET with 0.1 PLASMA for the one-time consent-key bind…"
  cast send "$OWNER_WALLET" --value 0.1ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  echo "  owner wallet balance: $(cast from-wei "$(cast balance "$OWNER_WALLET" --rpc-url "$RPC")") PLASMA"
fi

echo "Done. $SIGNER is funded + whitelisted. Balance: $(cast from-wei "$(cast balance "$SIGNER" --rpc-url "$RPC")") PLASMA"
echo "isWhitelistedFor(VACCINATION): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$(cast keccak VACCINATION)" "$SIGNER" --rpc-url "$RPC")"
