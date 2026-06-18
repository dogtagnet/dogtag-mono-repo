#!/usr/bin/env bash
# DogTag testnet demo — boot the backends + portals wired to the LIVE ROAX deployment.
# Backends run with the in-memory store (no Mongo needed); restart = fresh state (re-genesis).
# Logs in .demo/, PIDs in .demo/pids. Stop with: scripts/demo-down.sh
#
#   scripts/demo-up.sh
#
# Then: open the portals (URLs printed), do the vet/groomer Setup wizard to genesis a signer,
# run scripts/demo-bootstrap.sh <thatSigner>, and click Issue -> Create QR. See docs/DEMO.md.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"; mkdir -p .demo; : > .demo/pids
RPC=https://devrpc.roax.net
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c
VR=0x19C1B5f80c41EE864149500bdF998Dd18aec2a43      # ZK-wired VerificationRegistry
SBT=0x1FB8986573Ac36d532cF7d5a5352202B094D4233      # DogTagSBT (central mints profiles)
VACC_CLONE=0x5c703910111f942EE0f47E02214291b5274cDb53
HMAC=dev-central-hmac-secret
# LAN IP so the share/verify QR points at a host the PHONE can reach (localhost is the phone itself).
# Override with: LAN_IP=192.168.x.x scripts/demo-up.sh
LAN_IP="${LAN_IP:-172.24.230.152}"
# Deployer key = registry WHITELIST_ADMIN + SBT ISSUER + PLASMA source (contracts/.env). The central
# stack broadcasts whitelistFor/mint AS this signer, so wire it at boot.
set -a; source "$ROOT/contracts/.env"; set +a
ADMIN_PK="$DEPLOYER_PRIVATE_KEY"; ADMIN_ADDR="$DEPLOYER_ADDRESS"
run(){ echo "  $1 -> $2 (log .demo/$1.log)"; ( "${@:3}" >".demo/$1.log" 2>&1 & echo $! >> .demo/pids ); }

echo "Building backend binaries (release for speed)…"
cargo build -q --release -p admin-api -p vet-api

echo "Starting backends:"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$SBT \
  ADMIN_PRIVATE_KEY=$ADMIN_PK ADMIN_ADDRESS=$ADMIN_ADDR DNS_CHECK=skip PORT=39742 \
  run admin-api ":39742" "$ROOT/target/release/admin-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Seaport Vet" ISSUER_DOMAIN=vet.local \
  BUSINESS_ID=biz-vet CONFIRMATIONS=1 PORT=41874 DEPLOYMENT_URL="${VET_PUBLIC_URL:-http://$LAN_IP:41874}" \
  run vet-api ":41874" "$ROOT/target/release/vet-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Pampered Paws" ISSUER_DOMAIN=groomer.local \
  BUSINESS_ID=biz-groomer BUSINESS_TYPE=groomer CONFIRMATIONS=1 PORT=43618 DEPLOYMENT_URL="http://$LAN_IP:43618" \
  run groomer-api ":43618" "$ROOT/target/release/vet-api"

echo "Starting portals (vite dev):"
run admin-web ":39741" pnpm --filter @dogtag/admin-web dev
run vet-web    ":41873" pnpm --filter @dogtag/vet-web dev
run groomer-web ":43617" pnpm --filter @dogtag/groomer-web dev

echo
echo "UP. Portals:  admin http://localhost:39741  vet http://localhost:41873  groomer http://localhost:43617"
echo "Backends:     admin :39742  vet :41874  groomer :43618   (ROAX chainId 135)"
echo "Next: docs/DEMO.md  (genesis the vet -> demo-bootstrap.sh <signer> -> Issue -> Create QR -> scan on phone)"
echo "For the PHONE: set its server base to this Mac's LAN IP (not localhost) — see docs/DEMO.md."
