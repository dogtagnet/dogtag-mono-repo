#!/usr/bin/env bash
# DogTag testnet demo — boot the backends + portals wired to the LIVE ROAX deployment.
# Backends run with the in-memory store (no Mongo needed). The custody seal (age-encrypted seed +
# non-secret keystore meta) IS persisted to .demo/{vet,groomer}-custody.json (CUSTODY_SEAL_PATH), so
# after a restart the operator UNLOCKS (same signer) instead of re-genesising. Everything else (records,
# sessions, op/admin sessions) is still in-memory and lost on restart.
# Logs in .demo/, PIDs in .demo/pids. Stop with: scripts/demo-down.sh
#
#   scripts/demo-up.sh
#
# Then: open the portals (URLs printed), do the vet/groomer Setup wizard to genesis a signer,
# run scripts/demo-bootstrap.sh <thatSigner>, and click Issue -> Create QR. See docs/DEMO.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"; mkdir -p .demo; : > .demo/pids
if [ -f "$ROOT/contracts/.env" ]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/contracts/.env"
  set +a
fi
RPC="${ROAX_RPC:-https://devrpc.roax.net}"
IR="${ISSUER_REGISTRY_ADDR:-${ISSUER_REGISTRY:-}}"
VR="${VERIFICATION_REGISTRY_CONSENT_ADDR:-}"
SBT="${SBT_CONSENT_ADDR:-}"
PROFILE_ISSUER="${PROFILE_ISSUER_ADDR:-}"
VACC_CLONE="${VACCINATION_ISSUER_ADDR:-}"
: "${IR:?set ISSUER_REGISTRY_ADDR to the fresh shared deployment}"
: "${VR:?set VERIFICATION_REGISTRY_CONSENT_ADDR to the fresh owner-hidden registry}"
: "${SBT:?set SBT_CONSENT_ADDR to the fresh owner-hidden SBT}"
: "${PROFILE_ISSUER:?set PROFILE_ISSUER_ADDR to a fresh factory clone}"
: "${VACC_CLONE:?set VACCINATION_ISSUER_ADDR to a fresh factory clone}"
HMAC=dev-central-hmac-secret
# LAN IP so the share/verify QR points at a host the PHONE can reach (localhost is the phone itself).
# Override with: LAN_IP=192.168.x.x scripts/demo-up.sh
LAN_IP="${LAN_IP:-172.24.230.152}"
# The key must hold the fresh deployment's registry/admin authorities and fund demo writes.
ADMIN_PK="${GOVERNANCE_PRIVATE_KEY:-${DEPLOYER_PRIVATE_KEY:-}}"
: "${ADMIN_PK:?set GOVERNANCE_PRIVATE_KEY or DEPLOYER_PRIVATE_KEY for the fresh deployment admin}"
ADMIN_ADDR="$(cast wallet address --private-key "$ADMIN_PK")"
run(){ echo "  $1 -> $2 (log .demo/$1.log)"; ( "${@:3}" >".demo/$1.log" 2>&1 & echo $! >> .demo/pids ); }

echo "Building backend binaries (release for speed)…"
cargo build -q --release -p admin-api -p vet-api -p government-api -p indexer-api
# The PROVER SERVICE is the SAME vet-api binary but compiled WITH the `prover` feature (which mounts
# `/prove-consent`). We build it to a SEPARATE target dir so the vet/groomer instances stay on the
# feature-OFF binary and therefore cannot accept a proving witness.
echo "Building prover-service binary (vet-api --features prover)…"
cargo build -q --release -p vet-api --features prover --target-dir "$ROOT/target/prover"

echo "Starting backends:"
# OVERSIGHT INDEXER (govarch PR-4) — scans ROAX events into a scope-enforced, non-PII index. In the
# demo it runs INDEXER_DEMO_MODE: scripted in-memory events + two well-known tokens (an UNSCOPED
# oversight token for government, a SCOPED token for vet/groomer). The role portals' Traceability /
# Oversight pages (govarch PR-5) consume it. NOTE: the scoped demo token is bound to a FIXED stand-in
# signer/clone, so a freshly-genesis'd vet/groomer sees "0 in scope" until its own signer is added to
# INDEXER_SCOPES; the government (unscoped) view always shows the full scripted cross-issuer feed.
INDEXER_DEMO_MODE=1 PORT=46001 VERIFICATION_REGISTRY_CONSENT_ADDR=$VR \
  run indexer-api ":46001" "$ROOT/target/release/indexer-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$PROFILE_ISSUER \
  ADMIN_PRIVATE_KEY=$ADMIN_PK ADMIN_ADDRESS=$ADMIN_ADDR DNS_CHECK=skip PORT=39742 \
  run admin-api ":39742" "$ROOT/target/release/admin-api"
# Every verifier/issuance process receives the same owner-hidden pair. PROFILE_ISSUER is a real
# factory clone: roots are issue(R)'d there, while mintCustodial seals the same R on the SBT.
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VERIFICATION_REGISTRY_CONSENT_ADDR=$VR SBT_ADDR=$SBT SBT_CONSENT_ADDR=$SBT \
  PROFILE_DOCUMENT_STORE=$PROFILE_ISSUER PROFILE_ISSUER_ADDR=$PROFILE_ISSUER \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Seaport Vet" ISSUER_DOMAIN=vet.local \
  BUSINESS_ID=biz-vet CONFIRMATIONS=1 PORT=41874 DEPLOYMENT_URL="${VET_PUBLIC_URL:-http://$LAN_IP:41874}" \
  INDEXER_API_BASE=http://localhost:46001 INDEXER_SCOPED_TOKEN=dogtag-indexer-vet-demo-token \
  CUSTODY_SEAL_PATH="$ROOT/.demo/vet-custody.json" \
  run vet-api ":41874" "$ROOT/target/release/vet-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VERIFICATION_REGISTRY_CONSENT_ADDR=$VR SBT_ADDR=$SBT SBT_CONSENT_ADDR=$SBT \
  PROFILE_DOCUMENT_STORE=$PROFILE_ISSUER PROFILE_ISSUER_ADDR=$PROFILE_ISSUER \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Pampered Paws" ISSUER_DOMAIN=groomer.local \
  BUSINESS_ID=biz-groomer BUSINESS_TYPE=groomer CONFIRMATIONS=1 PORT=43618 DEPLOYMENT_URL="${GROOMER_PUBLIC_URL:-http://$LAN_IP:43618}" \
  INDEXER_API_BASE=http://localhost:46001 INDEXER_SCOPED_TOKEN=dogtag-indexer-vet-demo-token \
  CUSTODY_SEAL_PATH="$ROOT/.demo/groomer-custody.json" \
  run groomer-api ":43618" "$ROOT/target/release/vet-api"
# PROVER SERVICE — the trusted 64-bit prover a 32-bit-only Android phone queries for its consent proof
# (the phone then submits that proof to the GROOMER itself, so the groomer never sees the witness).
# It's a vet-api built WITH `--features prover` and CIRCUITS_BUILD_DIR set so the real consent prover
# can load the frozen artifacts. TRUST: it sees the witness, so in prod it's the OWNER's trusted prover;
# the demo runs it as a platform service. Exposed via PROVER_PUBLIC_URL (mirrors VET/GROOMER_PUBLIC_URL):
#   cloudflared tunnel --url http://localhost:41875  ->  PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com
# then point the phone's `prover_api` pref at that URL (demo-prepare-phone.sh / Settings).
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VERIFICATION_REGISTRY_CONSENT_ADDR=$VR SBT_ADDR=$SBT SBT_CONSENT_ADDR=$SBT \
  PROFILE_DOCUMENT_STORE=$PROFILE_ISSUER PROFILE_ISSUER_ADDR=$PROFILE_ISSUER \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="DogTag Prover" ISSUER_DOMAIN=prover.local \
  BUSINESS_ID=biz-prover CONFIRMATIONS=1 PORT=41875 DEPLOYMENT_URL="${PROVER_PUBLIC_URL:-http://$LAN_IP:41875}" \
  CIRCUITS_BUILD_DIR="$ROOT/circuits/build" \
  CUSTODY_SEAL_PATH="$ROOT/.demo/prover-custody.json" \
  run prover-api ":41875" "$ROOT/target/prover/release/vet-api"

# GOVERNMENT stack — a SEPARATE deployable (its own government-api binary, own port, own DB), not a
# vet-api re-run. In the demo it runs against LIVE ROAX for gasless reads (verify), so it can verify a
# credential the vet stack just issued. On-chain issuance (TRAVEL_CLEARANCE) needs a funded, whitelisted
# GOV_SIGNER_KEY + a DogTagIssuer clone (TRAVEL_CLEARANCE_ISSUER_ADDR) — an ops step; unset here means
# /issue builds+persists via dry_run. See docs/ROLE_APPS.md §7.
ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR ISSUER_NAME="Example Competent Authority" ISSUER_DOMAIN=gov.local \
  CHAIN_ID=135 PORT=44832 DEPLOYMENT_URL="${GOV_PUBLIC_URL:-http://$LAN_IP:44832}" \
  TRAVEL_CLEARANCE_ISSUER_ADDR="${TRAVEL_CLEARANCE_ISSUER_ADDR:-}" GOV_SIGNER_KEY="${GOV_SIGNER_KEY:-}" \
  GOV_API_TOKEN="${GOV_API_TOKEN:-dogtag-gov-demo-token}" \
  INDEXER_API_BASE=http://localhost:46001 INDEXER_OVERSIGHT_TOKEN=dogtag-indexer-oversight-demo-token \
  run government-api ":44832" "$ROOT/target/release/government-api"

echo "Starting portals (vite dev):"
run admin-web ":39741" env VITE_DEMO_MODE=1 VITE_DOGTAG_SBT_ADDR="$SBT" pnpm --filter @dogtag/admin-web dev
run vet-web    ":41873" env VITE_DEMO_MODE=1 VITE_DOGTAG_SBT_ADDR="$SBT" VITE_DOGTAG_ISSUER_ADDR="$VACC_CLONE" pnpm --filter @dogtag/vet-web dev
run groomer-web ":43617" env VITE_DEMO_MODE=1 VITE_DOGTAG_SBT_ADDR="$SBT" VITE_DOGTAG_ISSUER_ADDR="$VACC_CLONE" pnpm --filter @dogtag/groomer-web dev
run government-web ":44831" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/government-web dev
# OWNER (holder) wallet - the consumer front. No backend; its prover URL points at the prover svc
# (:41875) and the verifier host comes from the /x/<token> link the owner pastes/scans.
run owner-web ":45931" env VITE_OWNER_PROVER_URL="${PROVER_PUBLIC_URL:-http://localhost:41875}" pnpm --filter @dogtag/owner-web dev

echo
echo "UP. Portals:  admin http://localhost:39741  vet http://localhost:41873  groomer http://localhost:43617  government http://localhost:44831  owner-wallet http://localhost:45931"
echo "Backends:     admin :39742  vet :41874  groomer :43618  government :44832  prover :41875  indexer :46001   (ROAX chainId 135)"
echo "Three-role showcase: scripts/e2e-roles.sh --live   (vet ISSUES -> government VERIFIES -> government ISSUES)"
echo "Prover svc:   POST :41875/prove-consent  (32-bit-Android fallback; set PROVER_PUBLIC_URL to tunnel it)"
echo "Owner wallet: http://localhost:45931  (Receive an issued wrapped doc -> Present a ZK proof to a verifier's /x/<token> link)"
echo "Next: provision the fresh issuance/verification roles, then Issue -> Create QR -> scan on phone (docs/DEMO.md)."
echo "For the PHONE: set its server base to this Mac's LAN IP (not localhost) — see docs/DEMO.md."
