#!/usr/bin/env bash
# DogTag testnet demo — on-chain bootstrap for ONE business signer.
# After you genesis a vet/groomer backend (its Setup wizard shows the derived signer address),
# run this with that address to FUND it with PLASMA + WHITELIST it for the demo record types,
# so it can issue against the pre-created clones on ROAX. (The admin portal "Approve" also
# whitelists; this script additionally funds gas, which the portal cannot.)
#
#   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
#
# Uses the deployer key in contracts/.env (registry WHITELIST_ADMIN + PLASMA source + factory owner).
set -euo pipefail
SIGNER="${1:?usage: demo-bootstrap.sh <signerAddress>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
set -a; source "$ROOT/contracts/.env"; set +a
RPC="$ROAX_RPC"; PK="$DEPLOYER_PRIVATE_KEY"
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c   # IssuerRegistry (deployments/roax.json)

echo "Funding $SIGNER with 0.5 PLASMA for gas…"
cast send "$SIGNER" --value 0.5ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
for RT in VACCINATION DOG_PROFILE SERVICE_ATTESTATION; do
  KEY=$(cast keccak "$RT")
  echo "whitelistFor($RT, $SIGNER)…"
  cast send "$IR" "whitelistFor(bytes32,address)" "$KEY" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
done
echo "Done. $SIGNER is funded + whitelisted. Balance: $(cast from-wei "$(cast balance "$SIGNER" --rpc-url "$RPC")") PLASMA"
echo "isWhitelistedFor(VACCINATION): $(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$(cast keccak VACCINATION)" "$SIGNER" --rpc-url "$RPC")"
