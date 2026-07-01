#!/usr/bin/env bash
# DogTag THREE-ROLE showcase — vet ISSUES -> government/groomer VERIFY -> government ISSUES.
#
# Demonstrates the three role stacks as genuinely SEPARATE running deployables (each its own binary,
# port, Mongo, on-chain wiring) driving one credential across roles. See docs/ROLE_APPS.md.
#
# Two modes:
#   scripts/e2e-roles.sh            # DEMO (default): boots an ephemeral government-api in
#                                   # GOV_DEMO_MODE and runs its ISSUE->VERIFY->records->audit
#                                   # round-trip. Zero external deps (no node, no gas, no Mongo).
#   scripts/e2e-roles.sh --live     # LIVE cross-stack: drives the SEPARATE running stacks from
#                                   # scripts/demo-up.sh (vet :41874 + government :44832) against
#                                   # ROAX. VET issues a VACCINATION -> GOVERNMENT verifies it
#                                   # (gasless) -> GOVERNMENT issues a TRAVEL_CLEARANCE. Requires
#                                   # demo-up running + contracts/.env (funded DEPLOYER key), curl,
#                                   # jq, python3, cast.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

GOV="${GOV_BASE:-http://localhost:44832}"
VET="${VET_BASE:-http://localhost:41874}"
RPC="${ROAX_RPC:-https://devrpc.roax.net}"
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c          # IssuerRegistry
VACC_CLONE=0x5c703910111f942EE0f47E02214291b5274cDb53   # VACCINATION issuer clone (live ROAX)

green(){ printf '\033[32mPASS\033[0m %s\n' "$1"; }
info(){ printf '\033[36m •\033[0m %s\n' "$1"; }
fail(){ printf '\033[31mFAIL\033[0m %s\n' "$1"; exit 1; }
step(){ printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
jqr(){ jq -r "$1"; }

MODE="${1:-demo}"
case "$MODE" in
  --live|live) MODE=live ;;
  ""|--demo|demo) MODE=demo ;;
  *) fail "unknown arg '$MODE' (use --live or --demo)" ;;
esac

# ==================================================================================================
# DEMO MODE — the government role stack, self-contained (no external deps). Proves the government
# role's two core actions (ISSUE + VERIFY) end-to-end as a separate running deployable.
# ==================================================================================================
if [ "$MODE" = demo ]; then
  command -v curl >/dev/null || fail "curl required"
  command -v jq   >/dev/null || fail "jq required"

  step "0. Build + boot government-api in GOV_DEMO_MODE (MemChain + MemStore, ephemeral)"
  cargo build -q -p government-api
  PORT=44932
  GOV="http://localhost:$PORT"
  GOV_DEMO_MODE=1 PORT=$PORT \
    TRAVEL_CLEARANCE_ISSUER_ADDR=0x1111111111111111111111111111111111111111 \
    "$ROOT/target/debug/government-api" >/tmp/e2e-roles-gov.log 2>&1 &
  GOVPID=$!
  trap 'kill "$GOVPID" 2>/dev/null || true' EXIT
  for _ in $(seq 1 30); do curl -fsS "$GOV/health" >/dev/null 2>&1 && break; sleep 0.3; done

  H=$(curl -fsS "$GOV/health")
  [ "$(echo "$H" | jqr .status)" = ok ] || fail "health: $H"
  green "government-api up on :$PORT (demo=$(echo "$H" | jqr .demo), canSign=$(echo "$H" | jqr .canSign))"

  step "1. GOVERNMENT ISSUES a TRAVEL_CLEARANCE (build root R + anchor on the emulated chain)"
  ISS=$(curl -fsS -X POST "$GOV/v1/travel-clearance/issue" -H 'content-type: application/json' \
    -d '{"record_type":"TRAVEL_CLEARANCE","dog_tag_id":"7","fields":{"originCountry":"US","destinationCountry":"FR"}}')
  [ "$(echo "$ISS" | jqr .anchored)" = true ] || fail "issue not anchored: $ISS"
  ROOT_HEX=$(echo "$ISS" | jqr .root)
  info "root = $ROOT_HEX  tx = $(echo "$ISS" | jqr .txHash)"
  green "TRAVEL_CLEARANCE issued + anchored"

  step "2. GOVERNMENT VERIFIES it (integrity offline + on-chain isValid + issuer identity)"
  WD=$(echo "$ISS" | jq -c '.wrappedDoc')
  SIGNER=$(echo "$H" | jqr .signer)
  VER=$(curl -fsS -X POST "$GOV/v1/verify" -H 'content-type: application/json' \
    -d "{\"wrapped_doc\":$WD,\"signer_addr\":\"$SIGNER\"}")
  [ "$(echo "$VER" | jqr .verdict)" = true ]                    || fail "verdict not true: $VER"
  [ "$(echo "$VER" | jqr .fragments.integrity)" = true ]        || fail "integrity: $VER"
  [ "$(echo "$VER" | jqr .fragments.onchain)" = true ]          || fail "onchain: $VER"
  [ "$(echo "$VER" | jqr .fragments.issuerWhitelisted)" = true ] || fail "issuer identity: $VER"
  [ "$(echo "$VER" | jqr .recomputedRoot)" = "$ROOT_HEX" ]      || fail "recomputed root mismatch: $VER"
  green "VERIFIED: verdict=true (integrity + onchain + issuer identity all pass)"

  step "3. Off-chain DB surfaces (records custody + verification audit log)"
  [ "$(curl -fsS "$GOV/v1/records" | jq '.records | length')" -ge 1 ]         || fail "records empty"
  [ "$(curl -fsS "$GOV/v1/verifications" | jq '.verifications | length')" -ge 1 ] || fail "audit empty"
  green "1 issued credential persisted + 1 verification audit record persisted"

  step "4. Negative: an UNANCHORED root must fail the on-chain pillar"
  ISS2=$(curl -fsS -X POST "$GOV/v1/travel-clearance/issue" -H 'content-type: application/json' \
    -d '{"dog_tag_id":"9","dry_run":true}')
  WD2=$(echo "$ISS2" | jq -c '.wrappedDoc')
  VER2=$(curl -fsS -X POST "$GOV/v1/verify" -H 'content-type: application/json' -d "{\"wrapped_doc\":$WD2}")
  [ "$(echo "$VER2" | jqr .fragments.onchain)" = false ] || fail "unanchored should be onchain=false: $VER2"
  [ "$(echo "$VER2" | jqr .verdict)" = false ]           || fail "unanchored should be verdict=false: $VER2"
  green "unanchored root correctly rejected (onchain=false, verdict=false)"

  printf '\n\033[1;32mDEMO OK\033[0m — the government role runs end-to-end as a separate deployable.\n'
  echo "For the full three-stack chain (vet ISSUES -> government VERIFIES) run: scripts/demo-up.sh then scripts/e2e-roles.sh --live"
  exit 0
fi

# ==================================================================================================
# LIVE MODE — the true cross-stack chain over the SEPARATE running stacks (from demo-up.sh) + ROAX.
# ==================================================================================================
for c in curl jq python3 cast; do command -v "$c" >/dev/null || fail "$c required for --live"; done
[ -f "$ROOT/contracts/.env" ] || fail "--live needs contracts/.env (funded DEPLOYER key). Run the demo per docs/DEMO.md."
curl -fsS "$GOV/health" >/dev/null 2>&1 || fail "government stack not up on $GOV (run scripts/demo-up.sh)"
curl -fsS "$VET/health" >/dev/null 2>&1 || fail "vet stack not up on $VET (run scripts/demo-up.sh)"

ADMIN_PW="${ADMIN_PASSWORD:-admin}"; OP_PW="${OPERATOR_PASSWORD:-operator}"; PASSPHRASE="${GENESIS_PASSPHRASE:-demo-pass-123}"
set -a; source "$ROOT/contracts/.env"; set +a
VACC_KEY=$(cast keccak VACCINATION)

step "1. VET stack ISSUES a VACCINATION credential (anchors root on the live clone)"
ATOK_V=$(curl -fsS -X POST "$VET/admin/login" -H 'content-type: application/json' -d "{\"password\":\"$ADMIN_PW\"}" | jqr .token)
[ -n "$ATOK_V" ] && [ "$ATOK_V" != null ] || fail "vet admin login"
GEN=$(curl -fsS -X POST "$VET/admin/genesis/start" -H "authorization: Bearer $ATOK_V")
echo "$GEN" | jq -e '.error' >/dev/null 2>&1 && fail "genesis/start: $GEN (restart vet-api for a fresh custody)"
CW=$(echo "$GEN" | python3 -c "import json,sys; g=json.load(sys.stdin); print(json.dumps([g['words'][i] for i in g['challengeIndices']]))")
SIGNER=$(curl -fsS -X POST "$VET/admin/genesis/confirm" -H "authorization: Bearer $ATOK_V" -H 'content-type: application/json' \
  -d "{\"words\":$CW,\"passphrase\":\"$PASSPHRASE\"}" | jqr .address)
[ -n "$SIGNER" ] && [ "$SIGNER" != null ] || fail "genesis/confirm"
curl -fsS -X POST "$VET/admin/unlock" -H "authorization: Bearer $ATOK_V" -H 'content-type: application/json' \
  -d "{\"passphrase\":\"$PASSPHRASE\"}" >/dev/null || fail "unlock"
"$ROOT/scripts/demo-bootstrap.sh" "$SIGNER" >/dev/null
[ "$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$VACC_KEY" "$SIGNER" --rpc-url "$RPC")" = true ] \
  || fail "vet signer not whitelisted for VACCINATION"
OTOK=$(curl -fsS -X POST "$VET/login" -H 'content-type: application/json' -d "{\"password\":\"$OP_PW\"}" | jqr .token)
curl -fsS -X PUT "$VET/settings/signing-mode" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d '{"mode":"backend"}' >/dev/null
DOG_TAG_ID=${DOG_TAG_ID:-42}
PREP=$(curl -fsS --max-time 120 -X POST "$VET/credentials/prepare" -H "authorization: Bearer $OTOK" -H 'content-type: application/json' -d "{
  \"recordType\":\"VACCINATION\",\"dogTagId\":\"$DOG_TAG_ID\",
  \"fields\":{\"credentialSubject\":{\"name\":{\"tag\":2,\"value\":\"Rex\"},\"microchip\":{\"code\":{\"tag\":2,\"value\":\"985141006580319\"}}},\"vaccineProductName\":{\"tag\":2,\"value\":\"Rabvac 3\"},\"vaccinationDate\":{\"tag\":2,\"value\":\"2026-01-11\"}}}")
RID=$(echo "$PREP" | jqr .recordId); ROOT_HEX=$(echo "$PREP" | jqr .merkleRoot)
[ "$(cast call "$VACC_CLONE" 'isValid(bytes32)(bool)' "$ROOT_HEX" --rpc-url "$RPC")" = true ] || fail "root not anchored on the vet clone"
green "VET issued + anchored VACCINATION root $ROOT_HEX (isValid=true on-chain)"

step "2. Fetch the vet-issued wrapped credential (share -> one-time /r/<token>)"
QR=$(curl -fsS -X POST "$VET/records/$RID/share" -H "authorization: Bearer $OTOK" | jqr .qrUrl)
TOKEN="${QR##*/r/}"
WD=$(curl -fsS "$VET/r/$TOKEN")
[ "$(echo "$WD" | jqr .signature.merkleRoot)" = "$ROOT_HEX" ] || fail "vet share did not return the wrapped doc"
green "got the vet wrapped credential (recordType=$(echo "$WD" | jqr .issuer.recordType))"

step "3. GOVERNMENT stack VERIFIES the vet-issued credential (cross-role, gasless ROAX read)"
VER=$(curl -fsS -X POST "$GOV/v1/verify" -H 'content-type: application/json' \
  -d "{\"wrapped_doc\":$(echo "$WD" | jq -c .),\"issuer_addr\":\"$VACC_CLONE\",\"signer_addr\":\"$SIGNER\"}")
[ "$(echo "$VER" | jqr .fragments.integrity)" = true ] || fail "government integrity check failed: $VER"
[ "$(echo "$VER" | jqr .fragments.onchain)" = true ]   || fail "government on-chain isValid failed: $VER"
[ "$(echo "$VER" | jqr .verdict)" = true ]             || fail "government verdict not true: $VER"
green "GOVERNMENT verified the VET credential ON-CHAIN: verdict=true"

step "4. GOVERNMENT stack ISSUES its own TRAVEL_CLEARANCE (build root R; dry_run unless a gov clone+signer is wired)"
GI=$(curl -fsS -X POST "$GOV/v1/travel-clearance/issue" -H 'content-type: application/json' \
  -d "{\"dog_tag_id\":\"$DOG_TAG_ID\",\"dry_run\":true,\"fields\":{\"originCountry\":\"US\",\"destinationCountry\":\"DE\"}}")
GROOT=$(echo "$GI" | jqr .root)
[ -n "$GROOT" ] && [ "$GROOT" != null ] || fail "government issue failed: $GI"
green "GOVERNMENT built a TRAVEL_CLEARANCE credential (root $GROOT); anchor by wiring TRAVEL_CLEARANCE_ISSUER_ADDR + GOV_SIGNER_KEY (see docs/ROLE_APPS.md §7)"

printf '\n\033[1;32mLIVE OK\033[0m — vet ISSUES -> government VERIFIES -> government ISSUES, across separate running stacks.\n'
info "GROOMER verify (owner-consent presentation, phone/wallet-driven) is covered by scripts/e2e-smoke.sh step 6."
