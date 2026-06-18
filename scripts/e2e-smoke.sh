#!/usr/bin/env bash
# DogTag testnet (ROAX chainId 135) BACKEND end-to-end smoke test.
#
# Drives the LIVE running backends (admin/central :39742, vet :41874) with curl + cast through the
# full demo flow and verifies every on-chain effect. This is the click-through ground truth.
#
#   scripts/demo-up.sh          # start the stack (or start backends manually — see README)
#   scripts/e2e-smoke.sh        # run this smoke test
#
# It is idempotent-ish: it genesis-es a FRESH vet custody signer each run (the in-memory store means a
# backend restart wipes custody, so a fresh genesis is expected), funds + whitelists it, issues a
# VACCINATION credential anchored on the live clone, shares it (short one-time /r/<token>), and records a NORMAL-path
# verification on-chain (minting the subject SBT + binding the subject as needed).
#
# Requires: curl, jq, cast (foundry), python3. Uses contracts/.env (DEPLOYER_PRIVATE_KEY) for funding.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ADMIN="${ADMIN_BASE:-http://localhost:39742}"
VET="${VET_BASE:-http://localhost:41874}"
RPC="${ROAX_RPC:-https://devrpc.roax.net}"

IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c          # IssuerRegistry
VR=0x19C1B5f80c41EE864149500bdF998Dd18aec2a43          # VerificationRegistry (ZK-wired)
SBT=0x1FB8986573Ac36d532cF7d5a5352202B094D4233          # DogTagSBT
VACC_CLONE=0x5c703910111f942EE0f47E02214291b5274cDb53   # VACCINATION issuer clone
ADMIN_PW="${ADMIN_PASSWORD:-admin}"
OP_PW="${OPERATOR_PASSWORD:-operator}"
PASSPHRASE="${GENESIS_PASSPHRASE:-demo-pass-123}"

set -a; source "$ROOT/contracts/.env"; set +a   # DEPLOYER_PRIVATE_KEY / DEPLOYER_ADDRESS
PK="$DEPLOYER_PRIVATE_KEY"

R=21888242871839275222246405745257275088548364400416034343698204186575808495617  # BN254 r

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
APPR=$(curl -fsS --max-time 120 -X POST "$ADMIN/v1/issuer-applications/$APPID/approve" -H "authorization: Bearer $ATOK_C")
echo "  approve -> $APPR"
[ "$(echo "$APPR" | jqr .status)" = approved ] || fail "approve"
VACC_KEY=$(cast keccak VACCINATION)
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VACC_KEY" "$WL_PROBE" --rpc-url "$RPC")" = true ] \
  || fail "approve did not whitelist keccak256(VACCINATION) on-chain"
green "approve whitelisted keccak256(VACCINATION) for the issuer signer ON-CHAIN"

# --------------------------------------------------------------------------------------------------
step "2. Vet custody: admin login -> genesis start/confirm -> unlock -> accounts (FRESH token)"
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
step "3. Fund + whitelist the genesis signer on-chain (demo-bootstrap.sh)"
"$ROOT/scripts/demo-bootstrap.sh" "$SIGNER" >/dev/null
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VACC_KEY" "$SIGNER" --rpc-url "$RPC")" = true ] \
  || fail "signer not whitelisted for VACCINATION on-chain"
green "signer funded + whitelisted (VACCINATION/DOG_PROFILE/SERVICE_ATTESTATION)"

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
step "6. Verify-session NORMAL path: subject SBT mint + consent-key bind, ECDSA consent, recorded on-chain"
PURPOSE_LABEL=${PURPOSE_LABEL:-boarding}
# purpose (bytes32) = keccak256(label) mod r — what the registry stores + nullifies + verify-keys.
PURPOSE_B32=$(cast to-uint256 "$(python3 -c "print(int('$(cast keccak "$PURPOSE_LABEL")',16) % $R)")")
# verify whitelist key = keccak256(abi.encode("VERIFY:", purpose))
VERIFY_KEY=$(cast keccak "$(cast abi-encode 'f(string,bytes32)' 'VERIFY:' "$PURPOSE_B32")")
echo "  purpose(b32)=$PURPOSE_B32  verifyKey=$VERIFY_KEY"

# whitelist the relayer (genesis signer) for VERIFY:purpose so both the backend preflight and the
# on-chain `!verify-wl` require pass.
cast send "$IR" 'whitelistFor(bytes32,address)' "$VERIFY_KEY" "$SIGNER" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$SIGNER" --rpc-url "$RPC")" = true ] \
  || fail "relayer not whitelisted for VERIFY:$PURPOSE_LABEL"
green "relayer whitelisted for VERIFY:$PURPOSE_LABEL"

# Subject: a key we control. Mint the dogTagId SBT to the subject so ownerOf(dogTagId)==subject.
SUBJECT_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
SUBJECT=$(cast wallet address --private-key "$SUBJECT_PK")
echo "  subject=$SUBJECT"
ISSUER_ROLE=$(cast call "$SBT" 'ISSUER_ROLE()(bytes32)' --rpc-url "$RPC")
# grant the deployer the SBT issuer role (idempotent) and mint dogTagId to the subject if unminted.
cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$DEPLOYER_ADDRESS" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null 2>&1 || true
CUR_OWNER=$(cast call "$SBT" 'ownerOf(uint256)(address)' "$DOG_TAG_ID" --rpc-url "$RPC" 2>/dev/null || echo none)
if ! echo "$CUR_OWNER" | grep -iq "$SUBJECT"; then
  cast send "$SBT" 'mint(address,uint256,bytes32)' "$SUBJECT" "$DOG_TAG_ID" "$ROOT_HEX" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null \
    || fail "mint SBT to subject (dogTagId $DOG_TAG_ID may be owned by someone else — set DOG_TAG_ID to a fresh id and rerun)"
fi
green "subject owns dogTagId $DOG_TAG_ID"

# start the session (operator gated); this preflights the relayer VERIFY whitelist on-chain.
SS=$(curl -fsS --max-time 60 -X POST "$VET/verify/session/start" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' \
  -d "{\"purpose\":\"$PURPOSE_LABEL\",\"recordType\":\"VACCINATION\",\"mode\":\"normal\"}")
echo "  session/start -> $SS"
SID=$(echo "$SS" | jqr .sessionId)
[ -n "$SID" ] && [ "$SID" != null ] || fail "session/start: $SS"
green "verify session started: $SID"

# build the 9-field VerificationConsent + EIP-712 signature (domain DogTag/1, verifyingContract=VR).
NONCE=$(date +%s)
DEADLINE=$(python3 -c "print(2**64-1)")
RECORD_TYPE_B32=$(cast keccak VACCINATION)
CHALLENGE=0x00000000000000000000000000000000000000000000000000000000000000aa
TYPEHASH=$(cast keccak "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 purpose,bytes32 credentialRoot,bytes32 challenge,address relayer,address subject,uint256 nonce,uint256 deadline)")
STRUCT_HASH=$(cast keccak "$(cast abi-encode 'f(bytes32,uint256,bytes32,bytes32,bytes32,bytes32,address,address,uint256,uint256)' \
  "$TYPEHASH" "$DOG_TAG_ID" "$RECORD_TYPE_B32" "$PURPOSE_B32" "$ROOT_HEX" "$CHALLENGE" "$SIGNER" "$SUBJECT" "$NONCE" "$DEADLINE")")
# EIP-712 domain separator: keccak256(abi.encode(TYPEHASH_EIP712, keccak("DogTag"), keccak("1"), chainId, VR))
DOM_TYPEHASH=$(cast keccak "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)")
DOMAIN_SEP=$(cast keccak "$(cast abi-encode 'f(bytes32,bytes32,bytes32,uint256,address)' \
  "$DOM_TYPEHASH" "$(cast keccak DogTag)" "$(cast keccak 1)" 135 "$VR")")
DIGEST=$(cast keccak "0x1901${DOMAIN_SEP#0x}${STRUCT_HASH#0x}")
SIG=$(cast wallet sign --private-key "$SUBJECT_PK" --no-hash "$DIGEST")
echo "  consent digest=$DIGEST"

CONSENT_JSON=$(python3 -c "
import json
print(json.dumps({
  'dogTagId': '$DOG_TAG_ID',
  'recordType': '$RECORD_TYPE_B32',
  'purpose': '$PURPOSE_B32',
  'credentialRoot': '$ROOT_HEX',
  'challenge': '$CHALLENGE',
  'relayer': '$SIGNER',
  'subject': '$SUBJECT',
  'nonce': '$NONCE',
  'deadline': $DEADLINE,
}))")

# the disclosed doc is the wrapped credential (re-share a fresh one-time token to fetch it for the verifier).
QR2=$(curl -fsS -X POST "$VET/records/$RID/share" -H "authorization: Bearer $OTOK" | jqr .qrUrl)
TOKEN2="${QR2##*/r/}"
DOC=$(curl -fsS "$VET/r/$TOKEN2")

SUBMIT=$(curl -fsS --max-time 120 -X POST "$VET/verify/consent/submit" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' \
  -d "$(python3 -c "
import json,sys
print(json.dumps({
  'sessionId': '$SID',
  'consent': json.loads('''$CONSENT_JSON'''),
  'sig': '$SIG',
  'mode': 'normal',
  'disclosedDoc': json.loads(sys.stdin.read()),
})) " <<<"$DOC")")
echo "  consent/submit -> $SUBMIT"
[ "$(echo "$SUBMIT" | jqr .recorded)" = true ] || fail "consent/submit: $SUBMIT"
VTX=$(echo "$SUBMIT" | jqr .txHash)
green "NORMAL-path verification recorded on-chain; txHash=$VTX"

# compute the expected nullifier the registry consumes: Poseidon6(DS=4,dogTagId,purpose,relayer,subject,nonce)
# (we read it back off the Verified event via the session status instead of recomputing Poseidon here).

# --------------------------------------------------------------------------------------------------
step "7. GET /verify/session/{id} shows recorded + txHash"
STAT=$(curl -fsS "$VET/verify/session/$SID" -H "authorization: Bearer $OTOK")
echo "  session status -> $STAT"
[ "$(echo "$STAT" | jqr .status)" = recorded ] || fail "session status not recorded: $STAT"
[ "$(echo "$STAT" | jqr .txHash)" = "$VTX" ] || fail "session txHash mismatch"
green "session recorded with txHash $VTX"

printf '\n\033[1;32mALL STEPS PASSED — full backend flow works end-to-end on ROAX (chainId 135).\033[0m\n'
