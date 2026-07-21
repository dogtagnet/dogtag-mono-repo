#!/usr/bin/env bash
# DogTag ROAX owner-hidden consent end-to-end test (LIVE, gasless owner).
#
# The harness generates a REAL Groth16 proof from circuits/consent.circom, establishes the tag/root
# preconditions on a freshly deployed owner-hidden stack, and submits the proof through the verifier
# backend. Contract addresses are intentionally env-only: a redeploy must repoint this harness instead
# of silently falling back to the retired deployment.
#
# Required address env (directly or via contracts/.env):
#   ISSUER_REGISTRY_ADDR (or ISSUER_REGISTRY)
#   VERIFICATION_REGISTRY_CONSENT_ADDR
#   SBT_CONSENT_ADDR
#   PROFILE_ISSUER_ADDR
# Required signer env: GOVERNANCE_PRIVATE_KEY (or DEPLOYER_PRIVATE_KEY).
#
# Requires: curl, jq, cast, node, pnpm, cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ -f "$ROOT/contracts/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/contracts/.env"
  set +a
fi

RPC="${ROAX_RPC:-https://devrpc.roax.net}"
CHAIN_ID="${CHAIN_ID:-135}"
IR="${ISSUER_REGISTRY_ADDR:-${ISSUER_REGISTRY:-}}"
VR="${VERIFICATION_REGISTRY_CONSENT_ADDR:-}"
SBT="${SBT_CONSENT_ADDR:-}"
PROFILE_ISSUER="${PROFILE_ISSUER_ADDR:-}"
PK="${GOVERNANCE_PRIVATE_KEY:-${DEPLOYER_PRIVATE_KEY:-}}"

: "${IR:?set ISSUER_REGISTRY_ADDR (fresh shared deployment)}"
: "${VR:?set VERIFICATION_REGISTRY_CONSENT_ADDR (fresh owner-hidden registry)}"
: "${SBT:?set SBT_CONSENT_ADDR (fresh owner-hidden SBT)}"
: "${PROFILE_ISSUER:?set PROFILE_ISSUER_ADDR (fresh DogTagIssuer clone)}"
: "${PK:?set GOVERNANCE_PRIVATE_KEY or DEPLOYER_PRIVATE_KEY for the fresh deployment admin}"

ADMIN_ADDR="$(cast wallet address --private-key "$PK")"
export MONOREPO_ROOT="$ROOT"
export CIRCUITS_BUILD_DIR="$ROOT/circuits/build"

PORT="${E2E_PORT:-43777}"
GROOMER="http://127.0.0.1:$PORT"
OP_PW="operator"
ADMIN_PW="admin"
UNLOCK_PW=consent-e2e-passphrase

green(){ printf '\033[32mPASS\033[0m %s\n' "$1"; }
fail(){ printf '\033[31mFAIL\033[0m %s\n' "$1"; exit 1; }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
hex32(){ python3 -c "print('0x%064x' % int('$1'))"; }
addr_of(){ python3 -c "print('0x%040x' % int('$1'))"; }
lower(){ tr '[:upper:]' '[:lower:]'; }

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dogtag-consent-e2e.XXXXXX")"
FIXTURE="$TMP_DIR/consent-fixture.json"
SRV_PID=""
LOG="$ROOT/.demo/e2e-consent-groomer.log"
cleanup(){
  [ -n "${SRV_PID:-}" ] && kill "$SRV_PID" >/dev/null 2>&1 || true
  rm -r -- "$TMP_DIR" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for entry in "IssuerRegistry:$IR" "VerificationRegistryConsent:$VR" "DogTagSBTConsent:$SBT" "ProfileIssuer:$PROFILE_ISSUER"; do
  name="${entry%%:*}"
  address="${entry#*:}"
  [ "$(cast code "$address" --rpc-url "$RPC")" != 0x ] || fail "$name has no code at $address"
done
[ "$(cast call "$VR" 'issuerRegistry()(address)' --rpc-url "$RPC" | lower)" = "$(printf %s "$IR" | lower)" ] \
  || fail "owner-hidden registry is wired to a different IssuerRegistry"
[ "$(cast call "$VR" 'sbt()(address)' --rpc-url "$RPC" | lower)" = "$(printf %s "$SBT" | lower)" ] \
  || fail "owner-hidden registry is wired to a different SBT"
ZKV="$(cast call "$VR" 'zkVerifier()(address)' --rpc-url "$RPC")"
[ "$(cast code "$ZKV" --rpc-url "$RPC")" != 0x ] || fail "registry verifier has no code at $ZKV"

step "0. Build the consent fixture dependencies and isolated verifier backend"
pnpm --filter @dogtag/standard build >/dev/null
cargo build -q --release -p vet-api
mkdir -p "$ROOT/.demo"
DEMO_MODE=1 ADMIN_PASSWORD=$ADMIN_PW OPERATOR_PASSWORD=$OP_PW \
  CENTRAL_HMAC_SECRET=dev-central-hmac-secret ROAX_RPC=$RPC CHAIN_ID=$CHAIN_ID \
  ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VERIFICATION_REGISTRY_CONSENT_ADDR=$VR SBT_ADDR=$SBT SBT_CONSENT_ADDR=$SBT \
  PROFILE_DOCUMENT_STORE=$PROFILE_ISSUER PROFILE_ISSUER_ADDR=$PROFILE_ISSUER \
  ISSUER_NAME="Consent E2E Groomer" ISSUER_DOMAIN=groomer.e2e BUSINESS_ID=biz-e2e \
  BUSINESS_TYPE=groomer CONFIRMATIONS=1 PORT=$PORT DEPLOYMENT_URL=$GROOMER \
  "$ROOT/target/release/vet-api" >"$LOG" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 60); do
  curl -fsS "$GROOMER/health" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -fsS "$GROOMER/health" >/dev/null 2>&1 || fail "backend failed to start (see $LOG)"
green "isolated verifier backend is up on $GROOMER"

step "1. Create and unlock a fresh gas-paying relayer"
ATOK="$(curl -fsS -X POST "$GROOMER/admin/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$ADMIN_PW\"}" | jq -r .token)"
GEN="$(curl -fsS -X POST "$GROOMER/admin/genesis/start" -H "authorization: Bearer $ATOK")"
WORDS_JSON="$(printf %s "$GEN" | jq -c '[.challengeIndices[] as $i | .words[$i]]')"
CONFIRM="$(curl -fsS -X POST "$GROOMER/admin/genesis/confirm" \
  -H "authorization: Bearer $ATOK" -H 'content-type: application/json' \
  -d "{\"words\":$WORDS_JSON,\"passphrase\":\"$UNLOCK_PW\"}")"
RELAYER="$(printf %s "$CONFIRM" | jq -r .address | lower)"
[ -n "$RELAYER" ] && [ "$RELAYER" != null ] || fail "relayer genesis failed: $CONFIRM"
UNLOCK="$(curl -fsS -X POST "$GROOMER/admin/unlock" -H "authorization: Bearer $ATOK" \
  -H 'content-type: application/json' -d "{\"passphrase\":\"$UNLOCK_PW\"}")"
[ "$(printf %s "$UNLOCK" | jq -r .unlocked)" = true ] || fail "relayer unlock failed: $UNLOCK"
green "fresh relayer = $RELAYER"

step "2. Generate and independently verify a real consent.circom proof"
CONSENT_FIXTURE_RELAYER="$RELAYER" CONSENT_FIXTURE_OUT="$FIXTURE" \
  node "$ROOT/circuits/scripts/gen-consent-fixture.mjs" >"$TMP_DIR/generator.log"

PUB_A="$(jq -c '.a' "$FIXTURE")"
PUB_B="$(jq -c '.b' "$FIXTURE")"
PUB_C="$(jq -c '.c' "$FIXTURE")"
PUB="$(jq -c '.pub' "$FIXTURE")"
[ "$(jq '.pub | length' "$FIXTURE")" = 7 ] || fail "consent proof must expose exactly seven signals"
DOG_TAG_ID="$(jq -r '.pub[0]' "$FIXTURE")"
PURPOSE_WORD="$(hex32 "$(jq -r '.pub[1]' "$FIXTURE")")"
P_RELAYER="$(addr_of "$(jq -r '.pub[2]' "$FIXTURE")" | lower)"
NULLIFIER="$(hex32 "$(jq -r '.pub[3]' "$FIXTURE")")"
ROOT_HEX="$(hex32 "$(jq -r '.pub[4]' "$FIXTURE")")"
RECORD_TYPE_WORD="$(hex32 "$(jq -r '.pub[5]' "$FIXTURE")")"
DEADLINE="$(jq -r '.pub[6]' "$FIXTURE")"
HIDDEN_OWNER="$(jq -r '._ownerAddress' "$FIXTURE" | lower)"
[ "$P_RELAYER" = "$RELAYER" ] || fail "proof relayer $P_RELAYER != backend relayer $RELAYER"

A0="$(jq -r '.a[0]' "$FIXTURE")"; A1="$(jq -r '.a[1]' "$FIXTURE")"
B00="$(jq -r '.b[0][0]' "$FIXTURE")"; B01="$(jq -r '.b[0][1]' "$FIXTURE")"
B10="$(jq -r '.b[1][0]' "$FIXTURE")"; B11="$(jq -r '.b[1][1]' "$FIXTURE")"
C0="$(jq -r '.c[0]' "$FIXTURE")"; C1="$(jq -r '.c[1]' "$FIXTURE")"
PJOIN="$(jq -r '.pub | join(",")' "$FIXTURE")"
[ "$(cast call "$ZKV" 'verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[7])(bool)' \
    "[$A0,$A1]" "[[$B00,$B01],[$B10,$B11]]" "[$C0,$C1]" "[$PJOIN]" --rpc-url "$RPC")" = true ] \
  || fail "committed ceremony verifier rejected the generated consent proof"
green "real consent proof accepted by verifier $ZKV (no owner public signal)"

step "3. Anchor the proof root and mint the owner-hidden tag"
ANCHOR_RT="$(cast call "$PROFILE_ISSUER" 'recordType()(bytes32)' --rpc-url "$RPC")"
if [ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$ANCHOR_RT" "$ADMIN_ADDR" --rpc-url "$RPC")" != true ]; then
  cast send "$IR" 'whitelistFor(bytes32,address)' "$ANCHOR_RT" "$ADMIN_ADDR" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
if [ "$(cast call "$PROFILE_ISSUER" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" != true ]; then
  cast send "$PROFILE_ISSUER" 'issue(bytes32)' "$ROOT_HEX" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null \
    || fail "profile issuer could not anchor R (use a fresh deployment if this root was revoked)"
fi

ISSUER_ROLE="$(cast call "$SBT" 'ISSUER_ROLE()(bytes32)' --rpc-url "$RPC")"
if [ "$(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$ADMIN_ADDR" --rpc-url "$RPC")" != true ]; then
  cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$ADMIN_ADDR" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
ONCHAIN_ROOT="$(cast call "$SBT" 'profileRoot(uint256)(bytes32)' "$DOG_TAG_ID" --rpc-url "$RPC")"
ZERO_WORD="0x$(printf '%064d' 0)"
if [ "$(printf %s "$ONCHAIN_ROOT" | lower)" = "$ZERO_WORD" ]; then
  cast send "$SBT" 'mintCustodial(uint256,bytes32)' "$DOG_TAG_ID" "$ROOT_HEX" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
elif [ "$(printf %s "$ONCHAIN_ROOT" | lower)" != "$(printf %s "$ROOT_HEX" | lower)" ]; then
  fail "dogTagId is already sealed to a different root"
fi
[ "$(cast call "$SBT" 'profileRoot(uint256)(bytes32)' "$DOG_TAG_ID" --rpc-url "$RPC" | lower)" = "$(printf %s "$ROOT_HEX" | lower)" ] \
  || fail "profileRoot(dogTagId) != proof R"
CUSTODIAN="$(cast call "$SBT" 'custodian()(address)' --rpc-url "$RPC" | lower)"
TAG_HOLDER="$(cast call "$SBT" 'ownerOf(uint256)(address)' "$DOG_TAG_ID" --rpc-url "$RPC" | lower)"
[ "$TAG_HOLDER" = "$CUSTODIAN" ] || fail "tag was not minted to the neutral custodian"
[ "$TAG_HOLDER" != "$HIDDEN_OWNER" ] || fail "hidden owner leaked into ERC-721 ownership"
green "R anchored and dogTagId $DOG_TAG_ID sealed to neutral custodian $CUSTODIAN"

step "4. Whitelist and fund the proof-bound relayer"
VERIFY_KEY="$(cast keccak "$(cast abi-encode 'f(string,bytes32)' 'VERIFY:' "$PURPOSE_WORD")")"
if [ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$RELAYER" --rpc-url "$RPC")" != true ]; then
  cast send "$IR" 'whitelistFor(bytes32,address)' "$VERIFY_KEY" "$RELAYER" \
    --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
cast send "$RELAYER" --value 0.1ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
green "relayer is whitelisted for proof purpose $PURPOSE_WORD and funded"

step "5. Submit through the owner-hidden backend relay"
OTOK="$(curl -fsS -X POST "$GROOMER/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$OP_PW\"}" | jq -r .token)"
PROOF="$(jq -nc --argjson a "$PUB_A" --argjson b "$PUB_B" --argjson c "$PUB_C" --argjson pubSignals "$PUB" \
  '{a:$a,b:$b,c:$c,pubSignals:$pubSignals}')"
ACK="$(curl -sS --max-time 180 -X POST "$GROOMER/verify/consent/levelb" \
  -H "authorization: Bearer $OTOK" -H 'content-type: application/json' \
  -d "$(jq -nc --argjson proof "$PROOF" '{proof:$proof}')")"
[ "$(printf %s "$ACK" | jq -r .status 2>/dev/null || true)" = recording ] || fail "submit failed: $ACK"
SID="$(printf %s "$ACK" | jq -r .sessionId)"

VTX=""
LAST_STATUS='{}'
for _ in $(seq 1 40); do
  LAST_STATUS="$(curl -fsS --max-time 30 "$GROOMER/verify/session/$SID" \
    -H "authorization: Bearer $OTOK" 2>/dev/null || printf '{}')"
  case "$(printf %s "$LAST_STATUS" | jq -r .status 2>/dev/null || true)" in
    recorded) VTX="$(printf %s "$LAST_STATUS" | jq -r .txHash)"; break ;;
    error) fail "on-chain record failed: $LAST_STATUS" ;;
    *) sleep 3 ;;
  esac
done
[ -n "$VTX" ] && [ "$VTX" != null ] || fail "timed out waiting for record: $LAST_STATUS"
green "owner-hidden verification recorded in $VTX"

step "6. Assert nullifier consumption, owner-hidden event shape, and no owner bytes"
[ "$(cast call "$VR" 'consumed(bytes32)(bool)' "$NULLIFIER" --rpc-url "$RPC")" = true ] \
  || fail "consumed(nullifier) != true"
RECEIPT="$(cast receipt "$VTX" --rpc-url "$RPC" --json)"
VR_LOWER="$(printf %s "$VR" | lower)"
EVENT_TOPIC="$(printf %s "$RECEIPT" | jq -r --arg address "$VR_LOWER" \
  'first(.logs[] | select((.address | ascii_downcase) == $address) | .topics[0]) // ""')"
EXPECTED_TOPIC="$(cast keccak 'Verified(uint256,address,bytes32,bytes32,uint256,uint256)')"
[ "$(printf %s "$EVENT_TOPIC" | lower)" = "$(printf %s "$EXPECTED_TOPIC" | lower)" ] \
  || fail "owner-hidden Verified event topic not found"
CHAIN_MATERIAL="$(cast tx "$VTX" --rpc-url "$RPC" --json; printf %s "$RECEIPT")"
if printf %s "$CHAIN_MATERIAL" | lower | grep -Fq "${HIDDEN_OWNER#0x}"; then
  fail "hidden owner bytes appeared in verification calldata or logs"
fi
green "nullifier consumed; Verified has no owner field; hidden owner bytes are absent"

printf '\n\033[1;32mOWNER-HIDDEN CONSENT E2E PASSED on chainId %s\033[0m\n' "$CHAIN_ID"
printf '  registry:       %s\n' "$VR"
printf '  proof signals:  dogTagId, purpose, relayer, nullifier, R, recordType, deadline\n'
printf '  recordType:     %s  deadline=%s\n' "$RECORD_TYPE_WORD" "$DEADLINE"
printf '  record tx:      %s\n' "$VTX"
