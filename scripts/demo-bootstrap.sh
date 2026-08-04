#!/usr/bin/env bash
# DogTag testnet demo — on-chain bootstrap for ONE business signer.
#
# After you genesis a vet/groomer backend (its Setup wizard shows the derived signer address), run
# this with that address to FUND it with PLASMA and grant the capabilities `demo-up.sh` needs.
#
#   scripts/demo-bootstrap.sh 0x<vetSignerAddress>
#
# For a verifier relayer it also grants the VERIFY capability for each purpose (default:
# grooming_intake boarding_intake daycare_access) so the signer may relay owner-hidden consent
# proofs. Override with VERIFY_PURPOSES="a b c".
#
# Uses the governance signer's key in contracts/.env — the registry `onlyOwner`, the SBT admin and
# the PLASMA source. The deploy ledger records that address as its `admin` key; set
# GOVERNANCE_PRIVATE_KEY to its key in contracts/.env.
#
# ---------------------------------------------------------------------------------------------
# THIS SCRIPT GRANTS LAYER 1 ONLY, AND IT SAYS SO RATHER THAN IMPLYING OTHERWISE
# ---------------------------------------------------------------------------------------------
#
# `DogTagIssuer.issue` requires BOTH:
#
#     registry.canIssue(address(this), msg.sender)   # layer 1 — the authority's grant + lifecycle
#     && issuanceAllowed[msg.sender]                 # layer 2 — THIS contract's own list
#
# Layer 2 is written by `setIssuanceAllowed`, which admits ONLY from the contract's `owner()`. The
# governance key this script signs with is deliberately NOT admitted to that direction: it also
# writes layer 1, so a registrar that could admit would hold both layers at once and reach exactly
# the cross-provider issuance layer 2 exists to prevent.
#
# So this script CANNOT complete the journey, by design. It grants layer 1, then CHECKS layer 2 and
# EXITS NON-ZERO naming the page that writes it — the vet/groomer portal's **Signing keys** page,
# where the contract's owner signs it from their own wallet. It must never report success while the
# signer still cannot issue.
#
# ---------------------------------------------------------------------------------------------
# WHAT THIS SCRIPT USED TO DO, AND WHY IT WAS WRONG
# ---------------------------------------------------------------------------------------------
#
# It granted through `whitelistFor(bytes32,address)` on the registry and preflighted
# `VerificationRegistryConsent.issuerRegistry()`. The launch set implements NEITHER: the issuance
# grant is `setRights(account, RIGHT_ISSUE)` (address-keyed, no record type), the verify grant is
# `setVerifierCapability(purpose, relayer, allowed)` taking the RAW purpose, and the registry
# reference on the consent registry is `providerRegistry()`.
#
# Measured on ROAX with foundry 1.5.1 rather than assumed: both missing selectors revert, `cast
# call` exits 1, and `cast send` exits 1 too because gas estimation fails first. So the old script
# failed LOUDLY rather than silently — but it failed with a FALSE DIAGNOSIS, reporting
# "VerificationRegistryConsent is wired to a different IssuerRegistry" when nothing was mis-wired
# and the truth was that this script spoke a contract language the deployed set does not implement.
# A confident wrong answer is the same defect as a silent one; it just sends you somewhere else.
set -euo pipefail
SIGNER="${1:?usage: demo-bootstrap.sh <signerAddress>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
set -a
# shellcheck disable=SC1091
source "$ROOT/contracts/.env"
set +a

# THE LEDGER IS THE SOURCE - the governance signer named in the message below is read from it rather
# than pinned here, so a redeploy that rotates the admin cannot leave this script naming the old one.
# shellcheck source=scripts/lib/ledger.sh
source "$ROOT/scripts/lib/ledger.sh"

RPC="${ROAX_RPC:-https://devrpc.roax.net}"
# Resolved into a variable BEFORE the message that names it. A `$(...)` inside `${VAR:?word}` works,
# but any apostrophe in that word opens a quote context bash never closes - "the ledger's admin"
# there is a parse error at load time, not a runtime one, so the whole script dies before its first
# line. Same family as the zsh `:r`/backtick hazards recorded in AGENTS.md; keep expansions plain.
ADMIN_SIGNER="$(ledger_addr admin)"
PK="${GOVERNANCE_PRIVATE_KEY:?set GOVERNANCE_PRIVATE_KEY to the key for the ledger admin signer ${ADMIN_SIGNER:-<none recorded>} in contracts/.env}"
PR="${PROVIDER_REGISTRY_ADDR:-${ISSUER_REGISTRY_ADDR:-$(ledger_addr ProviderRegistry)}}"
VR="${VERIFICATION_REGISTRY_CONSENT_ADDR:-$(ledger_addr VerificationRegistryConsent)}"
SBT="${SBT_CONSENT_ADDR:-$(ledger_addr DogTagSBTConsent)}"
# These two are per-provider `DogTagIssuer` CLONES, deployed by a provider rather than by
# Deploy.s.sol, so the ledger holds no key for them and there is nothing to resolve. They stay
# operator-supplied - exactly as demo-up.sh, whose address set this script consumes.
PROFILE_ISSUER="${PROFILE_ISSUER_ADDR:-}"
VACCINATION_ISSUER="${VACCINATION_ISSUER_ADDR:-}"

: "${PR:?no ProviderRegistry in the ledger; set PROVIDER_REGISTRY_ADDR}"
: "${VR:?no VerificationRegistryConsent in the ledger; set VERIFICATION_REGISTRY_CONSENT_ADDR}"
: "${SBT:?no DogTagSBTConsent in the ledger; set SBT_CONSENT_ADDR}"
: "${PROFILE_ISSUER:?set PROFILE_ISSUER_ADDR to the DOG_PROFILE issuer clone used by demo-up.sh}"
: "${VACCINATION_ISSUER:?set VACCINATION_ISSUER_ADDR to the vaccination issuer clone used by demo-up.sh}"

# BN254 r — purpose (bytes32) = keccak256(label) mod r, matching the backend and the registry.
R=21888242871839275222246405745257275088548364400416034343698204186575808495617

# The ONLY settable bit. `ProviderRegistry.RIGHT_ISSUE = 1 << 0`; every other bit is DERIVED from
# state the core already holds and `setRights` refuses to write one.
RIGHT_ISSUE=1

VERIFY_PURPOSES="${VERIFY_PURPOSES:-grooming_intake boarding_intake daycare_access}"

lower(){ tr '[:upper:]' '[:lower:]'; }
same(){ [ "$(printf %s "$1" | lower)" = "$(printf %s "$2" | lower)" ]; }
fail(){ echo "ERROR: $*" >&2; exit 1; }

# Fail before sending governance transactions if the configured contracts are missing or split
# across deployments. demo-up.sh consumes this same address set.
for entry in \
  "ProviderRegistry:$PR" \
  "VerificationRegistryConsent:$VR" \
  "DogTagSBTConsent:$SBT" \
  "DOG_PROFILE issuer:$PROFILE_ISSUER" \
  "VACCINATION issuer:$VACCINATION_ISSUER"; do
  NAME="${entry%%:*}"
  ADDRESS="${entry#*:}"
  [ "$(cast code "$ADDRESS" --rpc-url "$RPC")" != 0x ] || fail "$NAME has no code at $ADDRESS"
done
# `providerRegistry()`, NOT `issuerRegistry()`. The launch set's consent registry names the provider
# authority; asking for the retired name reverts, and reading that revert as "wired to a different
# registry" was the old script's false diagnosis.
same "$(cast call "$VR" 'providerRegistry()(address)' --rpc-url "$RPC")" "$PR" \
  || fail "VerificationRegistryConsent is wired to a different ProviderRegistry"
same "$(cast call "$VR" 'sbt()(address)' --rpc-url "$RPC")" "$SBT" \
  || fail "VerificationRegistryConsent is wired to a different DogTagSBTConsent"
for ISSUER in "$PROFILE_ISSUER" "$VACCINATION_ISSUER"; do
  # A clone is gated by its OWN `registry()`, written once at `initialize`. A clone bound to an
  # authority nobody reads fails closed on its first issuance and passes every other check until then.
  same "$(cast call "$ISSUER" 'registry()(address)' --rpc-url "$RPC")" "$PR" \
    || fail "issuer clone $ISSUER is bound to a different ProviderRegistry"
done

PROFILE_KEY="$(cast call "$PROFILE_ISSUER" 'recordType()(bytes32)' --rpc-url "$RPC")"
VACCINATION_KEY="$(cast call "$VACCINATION_ISSUER" 'recordType()(bytes32)' --rpc-url "$RPC")"
same "$PROFILE_KEY" "$(cast keccak DOG_PROFILE)" \
  || fail "PROFILE_ISSUER_ADDR is not a DOG_PROFILE issuer clone"
same "$VACCINATION_KEY" "$(cast keccak VACCINATION)" \
  || fail "VACCINATION_ISSUER_ADDR is not a VACCINATION issuer clone"

# Funding is read-before-write like every grant below. It was the ONE unconditional write here, so
# re-running the script topped the signer up again every time - and this script's whole selling point
# is that it is idempotent, which made that a false claim about its own behaviour rather than merely
# a wasted transfer.
GAS_FLOOR_WEI=100000000000000000   # 0.1 PLASMA — enough for the handful of writes a demo signer makes
BALANCE="$(cast balance "$SIGNER" --rpc-url "$RPC")"
if [ "$(python3 -c "print(int('$BALANCE') >= $GAS_FLOOR_WEI)")" = "True" ]; then
  echo "Funding $SIGNER: already holds $(cast from-wei "$BALANCE") PLASMA — skipping"
else
  echo "Funding $SIGNER with 0.5 PLASMA for gas…"
  cast send "$SIGNER" --value 0.5ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi

# --- LAYER 1: the authority's issue right -----------------------------------------------------
# ONE grant, on the ADDRESS. #143 re-keyed the issuance grant off `(service, recordType)` and onto
# the signer alone, so there is no per-record-type loop here any more and no service argument.
# `setRights` reverts NoChange() on a no-op, so an idempotent script must read before it writes.
RIGHTS="$(cast call "$PR" 'rightsOf(address)(uint256)' "$SIGNER" --rpc-url "$RPC")"
if [ $(( RIGHTS & RIGHT_ISSUE )) -ne 0 ]; then
  echo "setRights(RIGHT_ISSUE, $SIGNER): already granted — skipping"
else
  echo "setRights($SIGNER, RIGHT_ISSUE)…"
  cast send "$PR" 'setRights(address,uint256)' "$SIGNER" "$RIGHT_ISSUE" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
RIGHTS="$(cast call "$PR" 'rightsOf(address)(uint256)' "$SIGNER" --rpc-url "$RPC")"
# Read the BIT out of the mask, never compare the whole word. Bit 0 is the only settable bit today,
# so "== 1" and "bit 0 is set" agree on every mask this contract can emit - which is exactly what
# would let a whole-word comparison survive review until a second right is allocated.
[ $(( RIGHTS & RIGHT_ISSUE )) -ne 0 ] || fail "setRights did not take: rightsOf($SIGNER) = $RIGHTS"

# --- DogTagSBTConsent ISSUER_ROLE (owner-hidden DOG_PROFILE minting) ---------------------------
# The vet signer calls mintCustodial(dogTagId, root); no owner address is calldata.
ISSUER_ROLE="$(cast call "$SBT" 'ISSUER_ROLE()(bytes32)' --rpc-url "$RPC")"
if [ "$(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$SIGNER" --rpc-url "$RPC")" = "true" ]; then
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBTConsent: already granted — skipping"
else
  echo "grantRole(ISSUER, $SIGNER) on DogTagSBTConsent…"
  cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$SIGNER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi

# --- VERIFY capability (relayer can record owner-hidden consent) -------------------------------
# `setVerifierCapability` takes the RAW purpose and derives `verificationKey` ITSELF. Handing it an
# already-derived key derives twice and writes a capability `canVerify` never reads - a transaction
# that succeeds, costs gas and grants nothing. It also reverts NoChange() on a no-op.
for LABEL in $VERIFY_PURPOSES; do
  PURPOSE_B32=$(cast to-uint256 "$(python3 -c "print(int('$(cast keccak "$LABEL")',16) % $R)")")
  if [ "$(cast call "$PR" 'canVerify(bytes32,address)(bool)' "$PURPOSE_B32" "$SIGNER" --rpc-url "$RPC")" = "true" ]; then
    echo "setVerifierCapability($LABEL, $SIGNER): already granted — skipping"
  else
    echo "setVerifierCapability($LABEL, $SIGNER)  [purpose=$PURPOSE_B32]…"
    cast send "$PR" 'setVerifierCapability(bytes32,address,bool)' "$PURPOSE_B32" "$SIGNER" true \
      --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
  fi
  echo "  canVerify($LABEL): $(cast call "$PR" 'canVerify(bytes32,address)(bool)' "$PURPOSE_B32" "$SIGNER" --rpc-url "$RPC")"
done

echo
echo "Layer 1 done. $SIGNER is funded and holds RIGHT_ISSUE. Balance: $(cast from-wei "$(cast balance "$SIGNER" --rpc-url "$RPC")") PLASMA"

# --- LAYER 2: this script cannot write it, so it CHECKS it and refuses to claim success ---------
MISSING=""
for entry in "DOG_PROFILE:$PROFILE_ISSUER" "VACCINATION:$VACCINATION_ISSUER"; do
  NAME="${entry%%:*}"
  CLONE="${entry#*:}"
  ALLOWED="$(cast call "$CLONE" 'issuanceAllowed(address)(bool)' "$SIGNER" --rpc-url "$RPC")"
  CAN="$(cast call "$PR" 'canIssue(address,address)(bool)' "$CLONE" "$SIGNER" --rpc-url "$RPC")"
  echo "$NAME  $CLONE  canIssue=$CAN  issuanceAllowed=$ALLOWED"
  [ "$ALLOWED" = "true" ] || MISSING="$MISSING $NAME"
done

if [ -n "$MISSING" ]; then
  cat >&2 <<EOF

ERROR: layer 2 is missing for:$MISSING

  $SIGNER is not on those contracts' own issuance lists, so every attempt to issue through them
  will be refused however layer 1 is set. This script CANNOT fix that and must not pretend it can:
  \`setIssuanceAllowed\` admits only from each contract's owner(), and the governance key used here
  is deliberately excluded from that direction - it also writes layer 1, and one key holding both
  is the cross-provider issuance layer 2 exists to prevent.

  Fix it from the product: open the vet or groomer portal, go to "Signing keys", connect the wallet
  that owns the contract, and admit $SIGNER. The page shows this shop's signing key and whether each
  contract admits it.
EOF
  exit 1
fi

echo
echo "Both layers hold. $SIGNER can issue through both clones."
echo "DogTagSBTConsent: $SBT"
echo "VerificationRegistryConsent: $VR"
