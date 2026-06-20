#!/usr/bin/env bash
# Prepare the on-chain precondition for a REAL phone to run the groomer EXPORT (on-chain ZK verify).
#
# The export verify requires `ownerOf(dogTagId) == subject` where subject = the phone's wallet. The
# mobile app does NOT mint; the central/issuer does. This mints a DOG_PROFILE SBT with a fresh NUMERIC
# dogTagId to the phone's wallet (so ownerOf holds), and — if given the groomer's signer — funds +
# whitelists it for issuance + the VERIFY purposes. Then issue a VACCINATION for the SAME dogTagId in
# the vet portal so the phone can import it and export a proof.
#
#   scripts/demo-prepare-phone.sh <phoneWalletAddr> [groomerSignerAddr]
#
# Prints DOG_TAG_ID=<n> — set that exact numeric value as the dogTagId in the vet Issue form.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
[ $# -ge 1 ] || { echo "usage: $0 <phoneWalletAddr> [groomerSignerAddr]" >&2; exit 1; }
PHONE="$1"
set -a; source contracts/.env; set +a
RPC="${ROAX_RPC:-https://devrpc.roax.net}"
PK="$DEPLOYER_PRIVATE_KEY"
SBT=$(jq -r .DogTagSBT contracts/deployments/roax.json)

# The CANONICAL on-chain dogTagId is field_of_value(Integer(rawNumeric)) — the SAME value the credential
# leaf hashes to, which the circuit compares pub[0] against and the contract feeds to ownerOf. The raw
# numeric is only an off-chain HANDLE (what the operator types into the vaccination Issue form). Build
# the helper that computes the on-chain id.
cargo build -q -p dogtag-standard-rs --bin field-hash
FH="$ROOT/target/debug/field-hash"

# pick a fresh numeric handle whose field-hashed on-chain id is not yet minted (ownerOf reverts -> "none")
DOGTAG=""; ONCHAIN=""
for _ in $(seq 1 10); do
  cand=$(( (RANDOM * 32768 + RANDOM) % 900000000 + 100000000 ))
  oc=$("$FH" "$cand")
  owner=$(cast call "$SBT" 'ownerOf(uint256)(address)' "$oc" --rpc-url "$RPC" 2>/dev/null || echo none)
  if [ "$owner" = "none" ]; then DOGTAG="$cand"; ONCHAIN="$oc"; break; fi
done
[ -n "$DOGTAG" ] || { echo "could not find a free dogTagId" >&2; exit 1; }

# DOG_PROFILE SBT root is metadata only (the verify checks ownerOf, not this root) -> placeholder.
ROOTH=0x0000000000000000000000000000000000000000000000000000000000000001
echo "Minting DOG_PROFILE SBT (handle=$DOGTAG -> on-chain id=$ONCHAIN) to phone $PHONE…"
cast send "$SBT" 'mint(address,uint256,bytes32)' "$PHONE" "$ONCHAIN" "$ROOTH" \
  --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
got=$(cast call "$SBT" 'ownerOf(uint256)(address)' "$ONCHAIN" --rpc-url "$RPC")
echo "  ownerOf($ONCHAIN) = $got"
[ "$(echo "$got" | tr 'A-F' 'a-f')" = "$(echo "$PHONE" | tr 'A-F' 'a-f')" ] || { echo "ownerOf != phone" >&2; exit 1; }

if [ $# -ge 2 ]; then
  echo "Funding + whitelisting groomer relayer $2 (issuance + VERIFY:grooming/boarding/daycare)…"
  scripts/demo-bootstrap.sh "$2"
fi

echo
echo "NEXT — in the VET portal:  Issue → set dogTagId = $DOGTAG (exact) → Fill demo data → Sign & Issue → Create QR."
echo "Then on the phone:  scan the import QR, then scan the groomer EXPORT QR → on-device proof → recorded."
echo "DOG_TAG_ID=$DOGTAG"
