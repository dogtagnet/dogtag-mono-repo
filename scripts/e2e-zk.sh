#!/usr/bin/env bash
# DogTag testnet (ROAX chainId 135) BACKEND-RELAY *ZK* end-to-end test (LIVE, reproducible).
#
# Proves, against the LIVE new ROAX deployment, the gasless groomer ZK verification path:
#   (a) the RELAYER gaslessly binds the OWNER's consent key via ConsentKeyRegistry.bindConsentKeyFor
#       (the owner only ECDSA-signs an EIP-712 BindConsentKey off-chain; the relayer broadcasts), and
#   (b) a REAL Groth16 proof is recorded on-chain via VerificationRegistry.recordVerificationZK
#       THROUGH the backend RELAY endpoint (POST /v1/verify/consent) — the owner pays ZERO gas.
#
# The proof ORACLE is the host ark-circom Groth16 prover (crates/dogtag-prover-rs, bin `prove-stdin`),
# fed a circuit input bound to THIS run's relayer/subject/purpose/dogTagId by scripts/zk/gen_input.mjs.
# That same {a,b,c,pubSignals} is what an on-device phone prover would emit; here the script stands in
# for the phone. The proof is verified on-chain by the new Groth16Verifier (checked once up front).
#
# The script is fully self-contained + reproducible:
#   - boots its OWN isolated groomer backend (custom port, in-mem store) wired to the LIVE contracts,
#   - genesis'es a fresh relayer signer over the admin HTTP API (no manual wizard),
#   - establishes all on-chain preconditions with the deployer key (contracts/.env),
#   - drives the relay endpoint with the proof + bind block, and asserts the on-chain Verified state.
#
#   scripts/e2e-zk.sh
#
# Requires: curl, jq, cast (foundry), node, python3, cargo. Uses contracts/.env (DEPLOYER_PRIVATE_KEY).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
RPC="${ROAX_RPC:-https://devrpc.roax.net}"
CHAIN_ID=135
export MONOREPO_ROOT="$ROOT"
export CIRCUITS_BUILD_DIR="$ROOT/circuits/build"

# LIVE ROAX addresses (contracts/deployments/roax.json).
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c          # IssuerRegistry
VR=0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1          # VerificationRegistry (ZK-wired)
SBT=0x1FB8986573Ac36d532cF7d5a5352202B094D4233          # DogTagSBT
CKR=0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671          # ConsentKeyRegistry (gasless bindConsentKeyFor)
VACC_CLONE=0x5c703910111f942EE0f47E02214291b5274cDb53   # VACCINATION issuer clone
ZKV=0x138b433071Ad806E841B5AD53623290a9bf21761          # Groth16Verifier

set -a; source "$ROOT/contracts/.env"; set +a   # DEPLOYER_PRIVATE_KEY / DEPLOYER_ADDRESS
PK="$DEPLOYER_PRIVATE_KEY"

R=21888242871839275222246405745257275088548364400416034343698204186575808495617  # BN254 r

# Isolated backend instance for this run.
PORT="${E2E_PORT:-43777}"
GROOMER="http://127.0.0.1:$PORT"
OP_PW=operator
ADMIN_PW=admin
BIND_PW="bind-passphrase-e2e"

# Run-unique values so the test is re-runnable (fresh dogTagId -> fresh Merkle root R; fresh nonce ->
# fresh nullifier, so consumed[nf] is never replayed). SUBJECT = vm.addr(1) (privkey 0x1) — a key we
# control as the "owner"; it NEVER needs gas, it only ECDSA-signs the bind off-chain.
SUBJECT_PK=0x0000000000000000000000000000000000000000000000000000000000000001
SUBJECT=$(cast wallet address --private-key "$SUBJECT_PK" | tr 'A-F' 'a-f')
PURPOSE_LABEL="${PURPOSE_LABEL:-boarding_intake}"
RECORD_TYPE=VACCINATION
NOW=$(date +%s)
DOG_TAG_ID="${DOG_TAG_ID:-$(( (NOW % 9000000) + 800000 ))}"
CONSENT_NONCE="${CONSENT_NONCE:-$NOW}"
# purpose field element = keccak256(label) mod r (circuit/registry convention).
PURPOSE_DEC=$(python3 -c "print(int('$(cast keccak "$PURPOSE_LABEL")',16) % $R)")
PURPOSE_B32=$(cast to-uint256 "$PURPOSE_DEC")

green(){ printf '\033[32mPASS\033[0m %s\n' "$1"; }
fail(){ printf '\033[31mFAIL\033[0m %s\n' "$1"; cleanup; exit 1; }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
hex32(){ python3 -c "print('0x%064x' % int('$1'))"; }
addr_of(){ python3 -c "print('0x%040x' % int('$1'))"; }   # field element -> 0x address

SRV_PID=""
LOG="$ROOT/.demo/e2e-zk-groomer.log"
cleanup(){ [ -n "${SRV_PID:-}" ] && kill "$SRV_PID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "subject(owner)=$SUBJECT  dogTagId=$DOG_TAG_ID  purpose=$PURPOSE_LABEL($PURPOSE_DEC)  nonce=$CONSENT_NONCE"

# --------------------------------------------------------------------------------------------------
step "0. Build prover bin + vet-api, boot an isolated groomer backend wired to the LIVE contracts"
cargo build -q --release -p dogtag-prover-rs --bin prove-stdin
cargo build -q --release -p vet-api
mkdir -p "$ROOT/.demo"
ADMIN_PASSWORD=$ADMIN_PW OPERATOR_PASSWORD=$OP_PW CENTRAL_HMAC_SECRET=dev-central-hmac-secret \
  ROAX_RPC=$RPC CHAIN_ID=$CHAIN_ID ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  CONSENT_KEY_REGISTRY_ADDR=$CKR VACCINATION_ISSUER_ADDR=$VACC_CLONE \
  ISSUER_NAME="E2E Groomer" ISSUER_DOMAIN=groomer.e2e BUSINESS_ID=biz-e2e BUSINESS_TYPE=groomer \
  CONFIRMATIONS=1 PORT=$PORT DEPLOYMENT_URL=$GROOMER CIRCUITS_BUILD_DIR=$CIRCUITS_BUILD_DIR \
  "$ROOT/target/release/vet-api" >"$LOG" 2>&1 &
SRV_PID=$!
for i in $(seq 1 60); do
  curl -fsS "$GROOMER/health" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -fsS "$GROOMER/health" >/dev/null 2>&1 || fail "groomer backend failed to start (see $LOG)"
green "isolated groomer up on $GROOMER (pid $SRV_PID)"

# --------------------------------------------------------------------------------------------------
step "1. Genesis a fresh RELAYER signer over the admin HTTP API (no manual wizard)"
ATOK=$(curl -fsS -X POST "$GROOMER/admin/login" -H 'content-type: application/json' -d "{\"password\":\"$ADMIN_PW\"}" | jq -r .token)
[ -n "$ATOK" ] && [ "$ATOK" != null ] || fail "admin login"
GEN=$(curl -fsS -X POST "$GROOMER/admin/genesis/start" -H "authorization: Bearer $ATOK")
# Re-type the challenge words back, in challenge-index order, to confirm genesis.
WORDS_JSON=$(echo "$GEN" | jq -c '[.challengeIndices[] as $i | .words[$i]]')
CONFIRM=$(curl -fsS -X POST "$GROOMER/admin/genesis/confirm" -H "authorization: Bearer $ATOK" \
  -H 'content-type: application/json' -d "{\"words\":$WORDS_JSON,\"passphrase\":\"$BIND_PW\"}")
RELAYER=$(echo "$CONFIRM" | jq -r .address | tr 'A-F' 'a-f')
[ -n "$RELAYER" ] && [ "$RELAYER" != null ] || fail "genesis confirm: $CONFIRM"
UNLOCK=$(curl -fsS -X POST "$GROOMER/admin/unlock" -H "authorization: Bearer $ATOK" \
  -H 'content-type: application/json' -d "{\"passphrase\":\"$BIND_PW\"}")
[ "$(echo "$UNLOCK" | jq -r .unlocked)" = true ] || fail "unlock: $UNLOCK"
green "relayer (backend signer 0) = $RELAYER"

# --------------------------------------------------------------------------------------------------
step "2. PROOF ORACLE: generate a REAL Groth16 proof bound to (relayer, subject, purpose, dogTagId)"
ZK_RELAYER=$RELAYER ZK_SUBJECT=$SUBJECT ZK_PURPOSE=$PURPOSE_DEC ZK_DOGTAGID=$DOG_TAG_ID ZK_NONCE=$CONSENT_NONCE \
  node "$ROOT/scripts/zk/gen_input.mjs" > /tmp/e2e_geninput.json
echo "  proving (host ark-circom Groth16 — slow)…"
jq -c '.input' /tmp/e2e_geninput.json | "$ROOT/target/release/prove-stdin" > /tmp/e2e_proof.json
PUB_A=$(jq -c '.a' /tmp/e2e_proof.json); PUB_B=$(jq -c '.b' /tmp/e2e_proof.json)
PUB_C=$(jq -c '.c' /tmp/e2e_proof.json); PUB=$(jq -c '.pub' /tmp/e2e_proof.json)
P_DOGTAG=$(jq -r '.pub[0]' /tmp/e2e_proof.json)
P_RELAYER=$(addr_of "$(jq -r '.pub[2]' /tmp/e2e_proof.json)")
P_SUBJECT=$(addr_of "$(jq -r '.pub[3]' /tmp/e2e_proof.json)")
NULLIFIER=$(hex32 "$(jq -r '.pub[4]' /tmp/e2e_proof.json)")
KEY_HASH=$(hex32 "$(jq -r '.pub[5]' /tmp/e2e_proof.json)")
ROOT_HEX=$(hex32 "$(jq -r '.pub[6]' /tmp/e2e_proof.json)")
[ "$P_DOGTAG" = "$DOG_TAG_ID" ] || fail "proof dogTagId $P_DOGTAG != $DOG_TAG_ID"
[ "$P_RELAYER" = "$RELAYER" ] || fail "proof relayer $P_RELAYER != $RELAYER"
[ "$P_SUBJECT" = "$SUBJECT" ] || fail "proof subject $P_SUBJECT != $SUBJECT"
echo "  pub: dogTagId=$P_DOGTAG keyHash=$KEY_HASH nullifier=$NULLIFIER R=$ROOT_HEX"
# Sanity: the LIVE Groth16Verifier must accept this proof before we spend any gas.
A0=$(jq -r '.a[0]' /tmp/e2e_proof.json); A1=$(jq -r '.a[1]' /tmp/e2e_proof.json)
B00=$(jq -r '.b[0][0]' /tmp/e2e_proof.json); B01=$(jq -r '.b[0][1]' /tmp/e2e_proof.json)
B10=$(jq -r '.b[1][0]' /tmp/e2e_proof.json); B11=$(jq -r '.b[1][1]' /tmp/e2e_proof.json)
C0=$(jq -r '.c[0]' /tmp/e2e_proof.json); C1=$(jq -r '.c[1]' /tmp/e2e_proof.json)
PJOIN=$(jq -r '.pub | join(",")' /tmp/e2e_proof.json)
[ "$(cast call "$ZKV" 'verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[7])(bool)' \
    "[$A0,$A1]" "[[$B00,$B01],[$B10,$B11]]" "[$C0,$C1]" "[$PJOIN]" --rpc-url "$RPC")" = true ] \
  || fail "live Groth16Verifier rejected the generated proof"
green "real Groth16 proof generated + accepted by the LIVE Groth16Verifier on-chain"

# --------------------------------------------------------------------------------------------------
step "3. Preconditions (deployer): anchor root R on the VACCINATION clone -> isValid(R)==true"
RT_KEY=$(cast keccak "$RECORD_TYPE")
# The clone's issue(R) is onlyWhitelisted(recordType); whitelist the deployer for VACCINATION first.
if [ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$RT_KEY" "$DEPLOYER_ADDRESS" --rpc-url "$RPC")" != true ]; then
  cast send "$IR" 'whitelistFor(bytes32,address)' "$RT_KEY" "$DEPLOYER_ADDRESS" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
if [ "$(cast call "$VACC_CLONE" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" != true ]; then
  cast send "$VACC_CLONE" 'issue(bytes32)' "$ROOT_HEX" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null \
    || fail "clone.issue(R) reverted (R already taken by another clone? use a fresh DOG_TAG_ID)"
fi
[ "$(cast call "$VACC_CLONE" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" = true ] || fail "isValid(R) != true"
green "credentialRoot R anchored on the clone (isValid(R)==true)"

# --------------------------------------------------------------------------------------------------
step "4. Preconditions (deployer): mint dogTagId SBT to the subject (ownerOf(dogTagId)==subject)"
ISSUER_ROLE=$(cast call "$SBT" 'ISSUER_ROLE()(bytes32)' --rpc-url "$RPC")
if [ "$(cast call "$SBT" 'hasRole(bytes32,address)(bool)' "$ISSUER_ROLE" "$DEPLOYER_ADDRESS" --rpc-url "$RPC")" != true ]; then
  cast send "$SBT" 'grantRole(bytes32,address)' "$ISSUER_ROLE" "$DEPLOYER_ADDRESS" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null 2>&1 || true
fi
CUR_OWNER=$(cast call "$SBT" 'ownerOf(uint256)(address)' "$DOG_TAG_ID" --rpc-url "$RPC" 2>/dev/null | tr 'A-F' 'a-f' || echo none)
if [ "$CUR_OWNER" != "$SUBJECT" ]; then
  cast send "$SBT" 'mint(address,uint256,bytes32)' "$SUBJECT" "$DOG_TAG_ID" "$ROOT_HEX" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null \
    || fail "mint SBT to subject (dogTagId $DOG_TAG_ID taken — pick a fresh DOG_TAG_ID)"
fi
[ "$(cast call "$SBT" 'ownerOf(uint256)(address)' "$DOG_TAG_ID" --rpc-url "$RPC" | tr 'A-F' 'a-f')" = "$SUBJECT" ] \
  || fail "ownerOf(dogTagId) != subject"
green "subject owns dogTagId $DOG_TAG_ID"

# --------------------------------------------------------------------------------------------------
step "5. Preconditions (deployer): whitelist relayer for VERIFY:purpose + fund relayer (NOT subject)"
VERIFY_KEY=$(cast keccak "$(cast abi-encode 'f(string,bytes32)' 'VERIFY:' "$PURPOSE_B32")")
if [ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$RELAYER" --rpc-url "$RPC")" != true ]; then
  cast send "$IR" 'whitelistFor(bytes32,address)' "$VERIFY_KEY" "$RELAYER" --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
fi
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VERIFY_KEY" "$RELAYER" --rpc-url "$RPC")" = true ] \
  || fail "relayer not whitelisted for VERIFY:$PURPOSE_LABEL"
cast send "$RELAYER" --value 0.3ether --rpc-url "$RPC" --private-key "$PK" --legacy >/dev/null
# Record the subject's gas balance BEFORE the bind to later prove it never paid (must be 0 throughout).
SUBJECT_BAL_BEFORE=$(cast balance "$SUBJECT" --rpc-url "$RPC")
green "relayer whitelisted + funded ($(cast from-wei "$(cast balance "$RELAYER" --rpc-url "$RPC")") PLASMA); subject bal=$SUBJECT_BAL_BEFORE"

# --------------------------------------------------------------------------------------------------
step "6. Owner (subject) signs the EIP-712 BindConsentKey OFF-CHAIN (no gas) authorizing the bind"
# Digest = keccak256(0x1901 || domainSep(CKR, chainId) || structHash) over
#   BindConsentKey(bytes32 babyJubPubKeyHash, address wallet, uint256 nonce), wallet=subject.
BIND_NONCE=$(cast call "$CKR" 'bindNonce(address)(uint256)' "$SUBJECT" --rpc-url "$RPC")
BIND_TYPEHASH=$(cast keccak "BindConsentKey(bytes32 babyJubPubKeyHash,address wallet,uint256 nonce)")
STRUCT_HASH=$(cast keccak "$(cast abi-encode 'f(bytes32,bytes32,address,uint256)' "$BIND_TYPEHASH" "$KEY_HASH" "$SUBJECT" "$BIND_NONCE")")
DOM_TYPEHASH=$(cast keccak "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)")
DOMAIN_SEP=$(cast keccak "$(cast abi-encode 'f(bytes32,bytes32,bytes32,uint256,address)' "$DOM_TYPEHASH" "$(cast keccak DogTag)" "$(cast keccak 1)" $CHAIN_ID "$CKR")")
BIND_DIGEST=$(cast keccak "0x1901${DOMAIN_SEP#0x}${STRUCT_HASH#0x}")
OWNER_SIG=$(cast wallet sign --private-key "$SUBJECT_PK" --no-hash "$BIND_DIGEST")
green "owner off-chain bind sig produced (nonce=$BIND_NONCE) — owner spent NO gas"

# --------------------------------------------------------------------------------------------------
step "7. Operator login + start EXPORT session (mode=zk) on the relayer backend -> sessionId + one-time export token QR"
OTOK=$(curl -fsS -X POST "$GROOMER/login" -H 'content-type: application/json' -d "{\"password\":\"$OP_PW\"}" | jq -r .token)
[ -n "$OTOK" ] && [ "$OTOK" != null ] || fail "operator login"
SS=$(curl -fsS --max-time 60 -X POST "$GROOMER/verify/session/start" -H "authorization: Bearer $OTOK" \
  -H 'content-type: application/json' -d "{\"purpose\":\"$PURPOSE_LABEL\",\"recordType\":\"$RECORD_TYPE\",\"mode\":\"zk\"}")
SID=$(echo "$SS" | jq -r .sessionId)
[ -n "$SID" ] && [ "$SID" != null ] || fail "session/start: $SS"
# EXPORT QR is a low-density one-time token: <host>/x/<token>?a=<relayerAddr> (no JWT). Parse the
# token (path tail before `?`) — symmetric with the import `/r/<token>` flow.
QR=$(echo "$SS" | jq -r .qrUrl)
echo "$QR" | grep -q "/x/" || fail "export QR must be /x/<token>?a=<relayer>: $QR"
EXPORT_TOKEN=$(echo "$QR" | sed -E 's#.*/x/([0-9a-fA-F]+).*#\1#')
[ "${#EXPORT_TOKEN}" = 32 ] || fail "export token must be 32 hex chars: '$EXPORT_TOKEN' (qr=$QR)"
green "export session started: $SID (export token len ${#EXPORT_TOKEN})"

# (phone step) GET /x/<token> resolves the session metadata WITHOUT consuming the token.
META=$(curl -fsS --max-time 30 "$GROOMER/x/$EXPORT_TOKEN")
[ "$(echo "$META" | jq -r .sessionId)" = "$SID" ] || fail "GET /x/<token> sessionId mismatch: $META"
[ "$(echo "$META" | jq -r .mode)" = zk ] || fail "GET /x/<token> mode != zk: $META"
CHALLENGE=$(echo "$META" | jq -r .challenge)
[ -n "$CHALLENGE" ] && [ "$CHALLENGE" != null ] || fail "GET /x/<token> missing challenge: $META"
green "GET /x/<token> resolved: relayer=$(echo "$META" | jq -r .relayer) purpose=$(echo "$META" | jq -r .purpose)"

# --------------------------------------------------------------------------------------------------
step "8. RELAY: POST proof + gasless bind block to /v1/verify/consent (one-time export-token auth)"
DEADLINE=$(( NOW + 3600 ))
# The consent block the relayer cross-checks against the proof's public signals + the session.
CONSENT=$(jq -nc \
  --arg dogTagId "$DOG_TAG_ID" --arg recordType "$RT_KEY" --arg purpose "$PURPOSE_B32" \
  --arg credentialRoot "$ROOT_HEX" --arg challenge "$CHALLENGE" --arg relayer "$RELAYER" \
  --arg subject "$SUBJECT" --arg nonce "$CONSENT_NONCE" --argjson deadline "$DEADLINE" \
  '{dogTagId:$dogTagId,recordType:$recordType,purpose:$purpose,credentialRoot:$credentialRoot,challenge:$challenge,relayer:$relayer,subject:$subject,nonce:$nonce,deadline:$deadline}')
BIND=$(jq -nc --arg subject "$SUBJECT" --arg keyHash "$KEY_HASH" --arg ownerSig "$OWNER_SIG" \
  '{subject:$subject,keyHash:$keyHash,ownerSig:$ownerSig}')
PROOF=$(jq -nc --argjson a "$PUB_A" --argjson b "$PUB_B" --argjson c "$PUB_C" --argjson pubSignals "$PUB" \
  '{a:$a,b:$b,c:$c,pubSignals:$pubSignals}')
BODY=$(jq -nc --arg sessionId "$SID" --arg exportToken "$EXPORT_TOKEN" \
  --argjson consent "$CONSENT" --argjson proof "$PROOF" --argjson bind "$BIND" \
  '{sessionId:$sessionId,exportToken:$exportToken,consent:$consent,sig:"0x",mode:"zk",proof:$proof,bind:$bind}')
RELAY=$(curl -sS --max-time 180 -X POST "$GROOMER/v1/verify/consent" \
  -H "authorization: Bearer $EXPORT_TOKEN" -H 'content-type: application/json' -d "$BODY")
# The consent POST is now ASYNC: it returns 200 {status:"recording", sessionId} immediately and
# records on-chain in a background task. We must POLL the session status to completion.
RSTATUS=$(echo "$RELAY" | jq -r .status 2>/dev/null || echo null)
if [ "$RSTATUS" != recording ]; then
  printf 'relay response: %s\n--- last 30 backend log lines ---\n%s\n' "$RELAY" "$(tail -30 "$LOG")"
  fail "consent POST did not return status=recording (got: $RSTATUS)"
fi
green "consent accepted (async): status=recording sid=$SID — polling for on-chain record…"
# Poll the session via the OPERATOR bearer token — the one-time export token is consumed on a
# successful record, so it can't be used to read status here. ~40 tries × 3s = 120s budget.
VTX=null
for i in $(seq 1 40); do
  PS=$(curl -fsS --max-time 30 "$GROOMER/verify/session/$SID" -H "authorization: Bearer $OTOK" 2>/dev/null || echo '{}')
  PSTATUS=$(echo "$PS" | jq -r .status 2>/dev/null || echo null)
  case "$PSTATUS" in
    recorded)
      VTX=$(echo "$PS" | jq -r .txHash)
      [ -n "$VTX" ] && [ "$VTX" != null ] || fail "session recorded but missing txHash: $PS"
      break ;;
    error)
      printf 'session: %s\n--- last 30 backend log lines ---\n%s\n' "$PS" "$(tail -30 "$LOG")"
      fail "session went to error while recording" ;;
    *) sleep 3 ;;
  esac
done
if [ "$VTX" = null ] || [ -z "$VTX" ]; then
  printf 'last session: %s\n--- last 30 backend log lines ---\n%s\n' "$PS" "$(tail -30 "$LOG")"
  fail "timed out waiting for session to reach status=recorded (120s)"
fi
green "relay recorded on-chain: recordVerificationZK txHash=$VTX"

# --------------------------------------------------------------------------------------------------
step "9. Assert on-chain + gasless invariants on the LIVE new contracts"
# (a) the relayer's bind landed: keyOf(subject) == keyHash.
[ "$(cast call "$CKR" 'keyOf(address)(bytes32)' "$SUBJECT" --rpc-url "$RPC" | tr 'A-F' 'a-f')" = "$(echo "$KEY_HASH" | tr 'A-F' 'a-f')" ] \
  || fail "keyOf(subject) != keyHash after relayer bind"
green "(a) gasless bindConsentKeyFor: keyOf(subject)==keyHash"
# (b) the nullifier was consumed -> the Verified record landed on the new VR.
[ "$(cast call "$VR" 'consumed(bytes32)(bool)' "$NULLIFIER" --rpc-url "$RPC")" = true ] \
  || fail "consumed(nullifier) != true (record didn't land)"
green "(b) recordVerificationZK: consumed(nullifier)==true on the new VR"
# (b') the Verified event fired in the record tx.
VRLOW=$(echo "$VR" | tr 'A-F' 'a-f')
VLOG=$(cast receipt "$VTX" --rpc-url "$RPC" --json | jq -r '.logs[] | select((.address|ascii_downcase)=="'"$VRLOW"'") | .topics[0]' | head -1)
VTOPIC=$(cast keccak "Verified(uint256,address,address,bytes32,bytes32,uint256)")
[ "$(echo "$VLOG" | tr 'A-F' 'a-f')" = "$(echo "$VTOPIC" | tr 'A-F' 'a-f')" ] \
  || fail "Verified event topic not found in record tx logs"
green "(b') Verified event emitted by the new VR in tx $VTX"
# (c) the OWNER paid ZERO gas — its balance is unchanged (and still 0).
SUBJECT_BAL_AFTER=$(cast balance "$SUBJECT" --rpc-url "$RPC")
[ "$SUBJECT_BAL_AFTER" = "$SUBJECT_BAL_BEFORE" ] || fail "subject balance changed ($SUBJECT_BAL_BEFORE -> $SUBJECT_BAL_AFTER) — owner paid gas!"
[ "$SUBJECT_BAL_AFTER" = "0" ] || fail "subject balance is not zero ($SUBJECT_BAL_AFTER) — not a clean gasless owner"
green "(c) owner gasless: subject PLASMA balance == 0 (unchanged); relayer paid all gas"

# (d) session status reflects recorded + txHash via the dual-gated status endpoint.
# The one-time export token was CONSUMED on submit, so poll status via the operator session (the
# portal path). The phone polls with `?token=` BEFORE submitting; post-submit it relies on its own
# success response. Here we assert via the operator gate.
STAT=$(curl -fsS "$GROOMER/verify/session/$SID" -H "authorization: Bearer $OTOK")
[ "$(echo "$STAT" | jq -r .status)" = recorded ] || fail "session status != recorded: $STAT"
[ "$(echo "$STAT" | jq -r .txHash)" = "$VTX" ] || fail "session txHash mismatch: $STAT"
green "(d) GET /verify/session/$SID -> recorded, txHash=$VTX"

printf '\n\033[1;32mE2E ZK GASLESS RELAY PASSED on LIVE ROAX (chainId %s)\033[0m\n' "$CHAIN_ID"
printf '  oracle:        host ark-circom Groth16 (dogtag-prover-rs prove-stdin)\n'
printf '  bind:          gasless bindConsentKeyFor by relayer %s (keyOf(subject)==keyHash)\n' "$RELAYER"
printf '  record tx:     %s  (recordVerificationZK; consumed(nullifier)==true; Verified event)\n' "$VTX"
printf '  owner gas:     subject %s balance == 0 (paid nothing)\n' "$SUBJECT"
printf '  session:       %s -> recorded\n' "$SID"
