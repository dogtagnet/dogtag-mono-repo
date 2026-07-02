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
RPC=https://devrpc.roax.net
IR=0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c
VR=0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1      # ZK-wired VerificationRegistry (meta-tx CKR)
CKR=0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671     # ConsentKeyRegistry (gasless bindConsentKeyFor)
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
cargo build -q --release -p admin-api -p vet-api -p government-api
# The PROVER SERVICE is the SAME vet-api binary but compiled WITH the `prover` feature (which mounts
# the `/prove-verification` route + the prover-independent circuit-input assembly). We build it to a
# SEPARATE target dir so the vet/groomer instances stay on the feature-OFF binary — the groomer
# literally cannot accept /prove-verification (the route is #[cfg(feature = "prover")] compiled out),
# so it can never see a witness. See stacks/vet/api/src/routes.rs::prove_verification.
echo "Building prover-service binary (vet-api --features prover)…"
cargo build -q --release -p vet-api --features prover --target-dir "$ROOT/target/prover"

echo "Starting backends:"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$SBT \
  ADMIN_PRIVATE_KEY=$ADMIN_PK ADMIN_ADDRESS=$ADMIN_ADDR DNS_CHECK=skip PORT=39742 \
  run admin-api ":39742" "$ROOT/target/release/admin-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR CONSENT_KEY_REGISTRY_ADDR=$CKR \
  SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$SBT \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Seaport Vet" ISSUER_DOMAIN=vet.local \
  BUSINESS_ID=biz-vet CONFIRMATIONS=1 PORT=41874 DEPLOYMENT_URL="${VET_PUBLIC_URL:-http://$LAN_IP:41874}" \
  CUSTODY_SEAL_PATH="$ROOT/.demo/vet-custody.json" \
  run vet-api ":41874" "$ROOT/target/release/vet-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR CONSENT_KEY_REGISTRY_ADDR=$CKR \
  SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$SBT \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Pampered Paws" ISSUER_DOMAIN=groomer.local \
  BUSINESS_ID=biz-groomer BUSINESS_TYPE=groomer CONFIRMATIONS=1 PORT=43618 DEPLOYMENT_URL="${GROOMER_PUBLIC_URL:-http://$LAN_IP:43618}" \
  CUSTODY_SEAL_PATH="$ROOT/.demo/groomer-custody.json" \
  run groomer-api ":43618" "$ROOT/target/release/vet-api"
# PROVER SERVICE — the trusted 64-bit prover a 32-bit-only Android phone queries for its Groth16 proof
# (the phone then submits that proof to the GROOMER itself, so the groomer never sees the witness).
# It's a vet-api built WITH `--features prover` and CIRCUITS_BUILD_DIR set so the REAL ArkProver (not
# the StubProver) is loaded. TRUST: it sees the witness, so in prod it's the OWNER's trusted prover;
# the demo runs it as a platform service. Exposed via PROVER_PUBLIC_URL (mirrors VET/GROOMER_PUBLIC_URL):
#   cloudflared tunnel --url http://localhost:41875  ->  PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com
# then point the phone's `prover_api` pref at that URL (demo-prepare-phone.sh / Settings).
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR CONSENT_KEY_REGISTRY_ADDR=$CKR \
  SBT_ADDR=$SBT PROFILE_DOCUMENT_STORE=$SBT \
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
  run government-api ":44832" "$ROOT/target/release/government-api"

echo "Starting portals (vite dev):"
run admin-web ":39741" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/admin-web dev
run vet-web    ":41873" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/vet-web dev
run groomer-web ":43617" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/groomer-web dev
run government-web ":44831" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/government-web dev
# OWNER (holder) wallet - the consumer front. No backend; its prover URL points at the prover svc
# (:41875) and the verifier host comes from the /x/<token> link the owner pastes/scans.
run owner-web ":45931" env VITE_OWNER_PROVER_URL="${PROVER_PUBLIC_URL:-http://localhost:41875}" pnpm --filter @dogtag/owner-web dev

echo
echo "UP. Portals:  admin http://localhost:39741  vet http://localhost:41873  groomer http://localhost:43617  government http://localhost:44831  owner-wallet http://localhost:45931"
echo "Backends:     admin :39742  vet :41874  groomer :43618  government :44832  prover :41875   (ROAX chainId 135)"
echo "Three-role showcase: scripts/e2e-roles.sh --live   (vet ISSUES -> government VERIFIES -> government ISSUES)"
echo "Prover svc:   POST :41875/prove-verification  (32-bit-Android fallback; set PROVER_PUBLIC_URL to tunnel it)"
echo "Owner wallet: http://localhost:45931  (Receive an issued wrapped doc -> Present a ZK proof to a verifier's /x/<token> link)"
echo "Next: docs/DEMO.md  (genesis the vet -> demo-bootstrap.sh <signer> -> Issue -> Create QR -> scan on phone)"
echo "For the PHONE: set its server base to this Mac's LAN IP (not localhost) — see docs/DEMO.md."
