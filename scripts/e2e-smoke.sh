#!/usr/bin/env bash
# DogTag testnet (ROAX chainId 135) BACKEND end-to-end smoke test.
#
# Drives the LIVE running backends (admin/central :39742, vet :41874) with curl + cast through the
# generic credential lifecycle and verifies every on-chain effect. This is the click-through
# ground truth for issuance, one-time sharing, direct credential checks, and revocation.
#
#   scripts/demo-up.sh          # start the stack (or start backends manually — see README)
#   scripts/e2e-smoke.sh        # run this smoke test
#
# It is idempotent-ish: it genesis-es a FRESH vet custody signer each run (the in-memory store means a
# backend restart wipes custody, so a fresh genesis is expected), funds + whitelists it, issues a
# VACCINATION credential anchored on the live clone, shares it (short one-time /r/<token>), checks it,
# and revokes it. Owner-hidden consent verification needs a real holder witness + DogTagConsent
# Groth16 proof, which this harness does not have; scripts/e2e-zk.sh exercises that canonical path.
#
# Requires: curl, jq, cast (foundry), python3. Uses contracts/.env (GOVERNANCE_PRIVATE_KEY) for
# funding and whitelist administration.
#
# STALE AGAINST THE LAUNCH SET, and resolving its addresses from the ledger does not change that.
# The whitelist steps below call `whitelistFor`/`isWhitelistedFor`, which `ProviderRegistry` does not
# implement on the issuance axis - onboarding is now the registrar SEQUENCE (`registerProvider` ->
# `setProviderStanding` -> `setServiceCreationApproval` -> the provider deploys its clone ->
# `attachService` -> `setServiceStanding` -> `setIssuanceCapability`). Rewriting the flow onto that
# sequence is C-2 work, deliberately not done inside a configuration change. What IS fixed here is
# that the addresses are no longer literals.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
set -a
# shellcheck disable=SC1091
source "$ROOT/contracts/.env"
set +a
ADMIN="${ADMIN_BASE:-http://localhost:39742}"
VET="${VET_BASE:-http://localhost:41874}"
RPC="${ROAX_RPC:-https://devrpc.roax.net}"
BOOTSTRAP_PK="${GOVERNANCE_PRIVATE_KEY:?set GOVERNANCE_PRIVATE_KEY in contracts/.env}"

# THE LEDGER IS THE SOURCE. Resolved by ledger KEY NAME with an env override for a one-off
# deployment - never a literal pinned here, which is the shape that keeps working after a redeploy
# while driving contracts that decide nothing.
# shellcheck source=scripts/lib/ledger.sh
source "$ROOT/scripts/lib/ledger.sh"

# The provider authority. This script still spells it `ISSUER_REGISTRY_ADDR`, as the backends do;
# what it NAMES on the launch set is `ProviderRegistry`, which is also what
# `DogTagIssuerFactory.registry()` answers.
IR="${ISSUER_REGISTRY_ADDR:-${ISSUER_REGISTRY:-$(ledger_addr ProviderRegistry)}}"
# A per-provider `DogTagIssuer` CLONE, deployed by a provider rather than by Deploy.s.sol, so the
# ledger holds no key for it and there is nothing to resolve. It stays operator-supplied.
VACC_CLONE="${VACCINATION_ISSUER_ADDR:-}"
: "${IR:?no ProviderRegistry in the ledger; set ISSUER_REGISTRY_ADDR}"
: "${VACC_CLONE:?set VACCINATION_ISSUER_ADDR to a factory clone this provider deployed}"
ADMIN_PW="${ADMIN_PASSWORD:-admin}"
OP_PW="${OPERATOR_PASSWORD:-operator}"
PASSPHRASE="${GENESIS_PASSPHRASE:-demo-pass-123}"

green(){ printf '\033[32mPASS\033[0m %s\n' "$1"; }
fail(){ printf '\033[31mFAIL\033[0m %s\n' "$1"; exit 1; }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$1"; }

jqr(){ jq -r "$1"; }

# --------------------------------------------------------------------------------------------------
step "1. Admin (central): login -> register business -> issuer-application -> approve (whitelistFor on-chain)"
ATOK_C=$(curl -fsS -X POST "$ADMIN/v1/admin/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$ADMIN_PW\"}" | jqr .token)
[ -n "$ATOK_C" ] && [ "$ATOK_C" != null ] || fail "admin login"
green "central admin login -> $ATOK_C"

curl -fsS -X POST "$ADMIN/v1/businesses" -H "authorization: Bearer $ATOK_C" -H 'content-type: application/json' \
  -d "{\"type\":\"vet\",\"name\":\"Seaport Vet\",\"lat\":1.28,\"lng\":103.85,\"services\":[\"vaccination\"],\"apiBaseUrl\":\"$VET\",\"domain\":\"vet.local\",\"documentStores\":[\"$VACC_CLONE\"]}" >/dev/null
green "registered vet business"

# Use a throwaway address to PROVE approve->whitelistFor lands on-chain (idempotent: re-whitelisting is a no-op).
WL_PROBE=0x000000000000000000000000000000000000bEEF
APPID=$(curl -fsS -X POST "$ADMIN/v1/issuer-applications" -H 'content-type: application/json' \
  -d "{\"issuerEntityId\":\"seaport-vet\",\"addresses\":[\"$WL_PROBE\"],\"recordTypes\":[\"VACCINATION\"],\"domain\":\"vet.local\",\"documentStore\":\"$VACC_CLONE\"}" | jqr .applicationId)
# The DNS legitimacy check is ADVISORY, not blocking: a non-verified observation is answered with a 409
# and needs the admin's EXPLICIT proceedWithoutDns. `vet.local` can NEVER publish the TXT record, so this
# path is the one the demo always takes — send the confirmation rather than reintroduce a bypass switch.
APPR=$(curl -fsS --max-time 120 -X POST "$ADMIN/v1/issuer-applications/$APPID/approve" \
  -H "authorization: Bearer $ATOK_C" -H 'content-type: application/json' \
  -d '{"proceedWithoutDns":true}')
echo "  approve -> $APPR"
[ "$(echo "$APPR" | jqr .status)" = approved ] || fail "approve"
# EXERCISE the advisory path rather than merely surviving it: the override must be RECORDED, or it is
# fail-open with extra steps. `dnsState` itself is left unpinned — a `.local` name resolves to notListed
# or couldNotCheck depending on the resolver, and both are honest.
APPR_DNS=$(echo "$APPR" | jqr .dnsState)
[ "$(echo "$APPR" | jqr .dnsProceededUnverified)" = true ] \
  || fail "approve did not record that it proceeded without a verified DNS observation (dnsState=$APPR_DNS)"
case "$APPR_DNS" in
  notListed|couldNotCheck) ;;
  *) fail "approve reported dnsState=$APPR_DNS for vet.local, which cannot publish the record" ;;
esac
green "approve recorded the unverified DNS observation ($APPR_DNS) instead of fabricating a pass"
VACC_KEY=$(cast keccak VACCINATION)
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VACC_KEY" "$WL_PROBE" --rpc-url "$RPC")" = true ] \
  || fail "approve did not whitelist keccak256(VACCINATION) on-chain"
green "approve whitelisted keccak256(VACCINATION) for the issuer signer ON-CHAIN"

# --------------------------------------------------------------------------------------------------
step "2. Vet custody: admin login -> genesis start/confirm -> unlock (FRESH token)"
ATOK_V=$(curl -fsS -X POST "$VET/admin/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$ADMIN_PW\"}" | jqr .token)
[ -n "$ATOK_V" ] && [ "$ATOK_V" != null ] || fail "vet admin login"
green "vet admin login (fresh token) -> $ATOK_V"

GEN=$(curl -fsS -X POST "$VET/admin/genesis/start" -H "authorization: Bearer $ATOK_V")
if echo "$GEN" | jq -e '.error' >/dev/null 2>&1; then
  fail "genesis/start: $GEN (already initialized? restart vet-api for a fresh run)"
fi
CW=$(echo "$GEN" | python3 -c "import json,sys; g=json.load(sys.stdin); print(json.dumps([g['words'][i] for i in g['challengeIndices']]))")
CONF=$(curl -fsS -X POST "$VET/admin/genesis/confirm" -H "authorization: Bearer $ATOK_V" -H 'content-type: application/json' \
  -d "{\"words\":$CW,\"passphrase\":\"$PASSPHRASE\"}")
SIGNER=$(echo "$CONF" | jqr .address)
[ -n "$SIGNER" ] && [ "$SIGNER" != null ] || fail "genesis/confirm: $CONF"
green "genesis confirmed; signer = $SIGNER"
curl -fsS -X POST "$VET/admin/unlock" -H "authorization: Bearer $ATOK_V" -H 'content-type: application/json' \
  -d "{\"passphrase\":\"$PASSPHRASE\"}" >/dev/null || fail "unlock"
green "custody unlocked (signer wired into chain client)"

# --------------------------------------------------------------------------------------------------
step "3. Fund + whitelist the genesis signer on-chain"
cast send "$SIGNER" --value 0.5ether --rpc-url "$RPC" --private-key "$BOOTSTRAP_PK" --legacy >/dev/null
cast send "$IR" 'whitelistFor(bytes32,address)' "$VACC_KEY" "$SIGNER" \
  --rpc-url "$RPC" --private-key "$BOOTSTRAP_PK" --legacy >/dev/null
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VACC_KEY" "$SIGNER" --rpc-url "$RPC")" = true ] \
  || fail "signer not whitelisted for VACCINATION on-chain"
green "signer funded + whitelisted for VACCINATION"

# --------------------------------------------------------------------------------------------------
step "4. Operator: login -> backend-mode -> prepare VACCINATION -> anchor issue(root) on the clone"
OTOK=$(curl -fsS -X POST "$VET/login" -H 'content-type: application/json' -d "{\"password\":\"$OP_PW\"}" | jqr .token)
[ -n "$OTOK" ] && [ "$OTOK" != null ] || fail "operator login"
curl -fsS -X PUT "$VET/settings/signing-mode" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' \
  -d '{"mode":"backend"}' >/dev/null
green "operator login + backend signing mode"

DOG_TAG_ID=${DOG_TAG_ID:-42}
PREP=$(curl -fsS --max-time 120 -X POST "$VET/credentials/prepare" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d "{
  \"recordType\":\"VACCINATION\",
  \"dogTagId\":\"$DOG_TAG_ID\",
  \"fields\":{
    \"credentialSubject\":{\"name\":{\"tag\":2,\"value\":\"Rex\"},\"microchip\":{\"code\":{\"tag\":2,\"value\":\"985141006580319\"}}},
    \"vaccineProductName\":{\"tag\":2,\"value\":\"Rabvac 3\"},
    \"vaccinationDate\":{\"tag\":2,\"value\":\"2026-01-11\"}
  }}")
echo "  prepare -> $PREP"
RID=$(echo "$PREP" | jqr .recordId)
ROOT_HEX=$(echo "$PREP" | jqr .merkleRoot)
[ "$(echo "$PREP" | jqr .mode)" = backend ] || fail "prepare not backend mode: $PREP"
[ -n "$ROOT_HEX" ] && [ "$ROOT_HEX" != null ] || fail "prepare: $PREP"
[ "$(cast call "$VACC_CLONE" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" = true ] \
  || fail "root NOT anchored on the VACCINATION clone (isValid=false)"
green "issue(root) anchored ON-CHAIN; isValid($ROOT_HEX)=true; confirm re-verified RootIssued"

# --------------------------------------------------------------------------------------------------
step "5. Share -> SHORT one-time token QR -> GET /r/<token> returns the doc -> second GET = 404 (one-time)"
QR=$(curl -fsS -X POST "$VET/records/$RID/share" -H "authorization: Bearer $OTOK" | jqr .qrUrl)
echo "  qrUrl=$QR"
case "$QR" in
  *"/r/"*) : ;;
  *) fail "share qrUrl is not a /r/<token> URL: $QR" ;;
esac
case "$QR" in
  *"t="*) fail "share qrUrl must NOT carry a JWT query string: $QR" ;;
esac
TOKEN="${QR##*/r/}"
[ "${#TOKEN}" = 32 ] || fail "share token must be 32 hex chars, got '${TOKEN}' (len ${#TOKEN})"
green "share returned a short one-time token: /r/$TOKEN"
G1=$(curl -fsS "$VET/r/$TOKEN")
[ "$(echo "$G1" | jqr .signature.merkleRoot)" = "$ROOT_HEX" ] || fail "GET 1 did not return the wrapped doc: $G1"
green "GET /r/<token> returned the wrapped doc"
CODE2=$(curl -s -o /dev/null -w '%{http_code}' "$VET/r/$TOKEN")
[ "$CODE2" = 404 ] || fail "GET 2 should be 404 (one-time token consumed), got $CODE2"
green "GET 2 = 404 (one-time token consumed)"

# --------------------------------------------------------------------------------------------------
step "6. Direct credential check reports the anchored credential as valid"
VERIFY_BODY=$(jq -n --argjson doc "$G1" --arg signer "$SIGNER" \
  '{wrappedDoc:$doc, signerAddr:$signer}')
VALID=$(curl -fsS --max-time 60 -X POST "$VET/verify/credential" \
  -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d "$VERIFY_BODY")
echo "  verify/credential -> $VALID"
[ "$(echo "$VALID" | jqr .verdict)" = true ] || fail "credential verdict is not valid: $VALID"
[ "$(echo "$VALID" | jqr .status)" = valid ] || fail "credential status is not valid: $VALID"
green "credential integrity + anchor + issuer whitelist are valid"

# --------------------------------------------------------------------------------------------------
step "7. Revoke the record -> on-chain invalid -> direct check reports revoked"
REV=$(curl -fsS --max-time 120 -X POST "$VET/records/$RID/revoke" \
  -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d '{}')
echo "  revoke -> $REV"
[ "$(echo "$REV" | jqr .status)" = revoked ] || fail "revoke did not succeed: $REV"
[ "$(cast call "$VACC_CLONE" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" = false ] \
  || fail "revoked root still reports valid on-chain"

REVOKED=$(curl -fsS --max-time 60 -X POST "$VET/verify/credential" \
  -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d "$VERIFY_BODY")
echo "  verify/credential after revoke -> $REVOKED"
[ "$(echo "$REVOKED" | jqr .verdict)" = false ] || fail "revoked credential verdict is not false: $REVOKED"
[ "$(echo "$REVOKED" | jqr .status)" = revoked ] || fail "credential status is not revoked: $REVOKED"
[ "$(echo "$REVOKED" | jqr .fragments.revoked)" = true ] || fail "revocation fragment missing: $REVOKED"
green "revocation recorded on-chain and reflected by direct verification"

printf '\n\033[1;32mALL STEPS PASSED — credential lifecycle works end-to-end on ROAX (chainId 135).\033[0m\n'
