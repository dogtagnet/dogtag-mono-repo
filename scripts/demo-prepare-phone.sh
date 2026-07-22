#!/usr/bin/env bash
# Prepare a REAL phone for the owner-hidden dog-tag issuance and consent demo.
#
# There is deliberately no scripted pre-mint: the phone derives the profile root R from its owner
# secret, and the vet learns only R when the phone redeems the one-time issuance QR. The issuance
# session allocates the dogTagId, so neither a phone wallet address nor a preselected id belongs here.
#
# Optionally bootstrap the groomer relayer that will submit the later consent proof:
#
#   scripts/demo-prepare-phone.sh
#   scripts/demo-prepare-phone.sh --groomer-relayer 0x<groomerSignerAddress>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

GROOMER_RELAYER=""
case $# in
  0) ;;
  2)
    if [ "$1" != "--groomer-relayer" ]; then
      echo "usage: $0 [--groomer-relayer 0x<groomerSignerAddress>]" >&2
      exit 1
    fi
    GROOMER_RELAYER="$2"
    ;;
  *)
    echo "usage: $0 [--groomer-relayer 0x<groomerSignerAddress>]" >&2
    echo "A phone wallet address is not needed: ownership is hidden in the device-computed root." >&2
    exit 1
    ;;
esac

if [ -n "$GROOMER_RELAYER" ]; then
  if [[ ! "$GROOMER_RELAYER" =~ ^0x[[:xdigit:]]{40}$ ]]; then
    echo "invalid groomer relayer address: $GROOMER_RELAYER" >&2
    exit 1
  fi
  echo "Bootstrapping groomer relayer $GROOMER_RELAYER..."
  "$ROOT/scripts/demo-bootstrap.sh" "$GROOMER_RELAYER"
  echo
fi

cat <<'EOF'
PHONE PREPARATION

1. On the phone, create or restore the owner's wallet/owner secret. The phone needs no gas.
2. In the VET portal, open Register pet, enter the owner identity and pet details, then Start.
   The backend allocates a fresh dogTagId and displays a one-time /p/<token> QR.
3. Scan that QR with the phone. The phone folds its owner secret into profile root R and submits only
   {token, root} to POST /profiles/issue/custodial-bind. No owner address enters the request or chain.
4. Keep the portal open until the issuance session reports bound and shows its transaction proof.
5. Issue the pet's VACCINATION record for that session's dogTagId, then scan its import QR on the phone.
6. In the groomer portal, start a consent verification and scan its /x/<token> QR with the phone.
   The phone creates the owner-hidden consent proof; the groomer relays and records it.

The device-owned root cannot be precomputed or pre-minted by this script. That is the privacy boundary.
EOF
