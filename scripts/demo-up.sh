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
cd "$ROOT"; mkdir -p .demo
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

# Read a deployed address out of the canonical ledger. Dependency-free, and the 40-hex-address value
# pattern means it only ever matches a real top-level address entry, never surrounding prose.
ledger_addr(){
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\(0x[0-9a-fA-F]\{40\}\)\".*/\1/p" \
    "$ROOT/contracts/deployments/roax.json" 2>/dev/null | head -1
}
# FACTORY — the admin portal's Issuers/Factory UI (predict + deploy a clone) needs this. It was never
# passed, so admin-api fell back to the zero address and every factory call answered
# "FACTORY_ADDR not configured" while governance/authority reported factoryOwner.target = 0x0.
# Resolved from env first, then the canonical ledger, so it is never a literal pinned in this script.
FACTORY="${FACTORY_ADDR:-${DOGTAG_ISSUER_FACTORY_ADDR:-$(ledger_addr DogTagIssuerFactory)}}"
# The generation-2 provider core, for the admin REGISTRAR screen only. Additive: no generation-1
# consumer reads it, and nothing is repointed onto it (that is C-9/C-10). Resolved by ledger KEY NAME
# so a stale literal cannot creep in; empty is tolerated here because admin-api refuses loudly on its
# own when the registrar routes are used without it.
PROVIDER_REGISTRY="${PROVIDER_REGISTRY_ADDR:-$(ledger_addr ProviderRegistry)}"
: "${FACTORY:?set FACTORY_ADDR, or add DogTagIssuerFactory to contracts/deployments/roax.json}"

HMAC=dev-central-hmac-secret
# LAN IP so the share/verify QR points at a host the PHONE can reach (localhost is the phone itself).
# Override with: LAN_IP=192.168.x.x scripts/demo-up.sh
LAN_IP="${LAN_IP:-172.24.230.152}"
# The key must hold the fresh deployment's registry/admin authorities and fund demo writes.
ADMIN_PK="${GOVERNANCE_PRIVATE_KEY:-${DEPLOYER_PRIVATE_KEY:-}}"
: "${ADMIN_PK:?set GOVERNANCE_PRIVATE_KEY or DEPLOYER_PRIVATE_KEY for the fresh deployment admin}"
ADMIN_ADDR="$(cast wallet address --private-key "$ADMIN_PK")"
run(){ echo "  $1 -> $2 (log .demo/$1.log)"; ( "${@:3}" >".demo/$1.log" 2>&1 & echo $! >> .demo/pids ); }
die(){ echo; echo "ERROR: $*" >&2; exit 1; }

# ------------------------------------------------------------------------------------------------
# PREFLIGHT — fail loudly here rather than boot a stack that looks healthy and silently does nothing.
# ------------------------------------------------------------------------------------------------
CHAIN_ID_EXPECTED="${CHAIN_ID:-135}"
echo "Preflight (chainId $CHAIN_ID_EXPECTED, $RPC):"

ACTUAL_CHAIN_ID="$(cast chain-id --rpc-url "$RPC" 2>/dev/null || true)"
[ -n "$ACTUAL_CHAIN_ID" ] || die "cannot reach $RPC (cast chain-id failed). The demo needs the live node."
[ "$ACTUAL_CHAIN_ID" = "$CHAIN_ID_EXPECTED" ] \
  || die "$RPC reports chainId $ACTUAL_CHAIN_ID, expected $CHAIN_ID_EXPECTED. Refusing to boot against the wrong chain."
echo "  chainId               $ACTUAL_CHAIN_ID  ok"

# The factory must be bound to the SAME IssuerRegistry the rest of the stack uses. A stale ledger entry
# from a superseded deployment is otherwise invisible: clones deploy, but against a registry nobody reads.
FACTORY_REGISTRY="$(cast call "$FACTORY" 'registry()(address)' --rpc-url "$RPC" 2>/dev/null || true)"
[ -n "$FACTORY_REGISTRY" ] || die "factory $FACTORY has no registry() - not a DogTagIssuerFactory (or wrong chain)."
if [ "$(echo "$FACTORY_REGISTRY" | tr 'A-Z' 'a-z')" != "$(echo "$IR" | tr 'A-Z' 'a-z')" ]; then
  die "factory $FACTORY is bound to registry $FACTORY_REGISTRY but the stack uses ISSUER_REGISTRY_ADDR=$IR.
  Clones deployed from it would be invisible to every verifier. Fix FACTORY_ADDR or ISSUER_REGISTRY_ADDR."
fi
echo "  factory               $FACTORY -> registry $IR  ok"

# The operator's DECLARATION that out-of-band signing is intended, resolved ONCE so the refusal below and
# admin-api agree on it. `ADMIN_PROPOSE_ONLY` is the canonical name and `ALLOW_UNAUTHORIZED_ADMIN_SIGNER`
# its accepted alias: either suppresses the refusal AND reaches admin-api, which cannot otherwise tell a
# designed proposal from a wrong-key one and reports every not-broadcast grant as the latter. Defaults
# THROUGH the canonical name so an operator-set value (incl. one sourced from contracts/.env) survives.
# Truthy set mirrors admin_api::startup::env_flag (trim + lowercase, "1"|"true"), the ONE reader both
# admin-api control-plane flags now go through.
ADMIN_PROPOSE_ONLY="${ADMIN_PROPOSE_ONLY:-${ALLOW_UNAUTHORIZED_ADMIN_SIGNER:-0}}"
case "$(echo "$ADMIN_PROPOSE_ONLY" | tr -d '[:space:]' | tr 'A-Z' 'a-z')" in
  1|true) PROPOSE_ONLY_DECLARED=1 ;;
  *) PROPOSE_ONLY_DECLARED=0 ;;
esac

# The hosted admin signer MUST hold WHITELIST_ADMIN. The retired deployer EOA lost it in governance
# Phase-2, and with only DEPLOYER_PRIVATE_KEY the stack booted cleanly while every portal grant returned
# disposition:"proposed" with unsigned calldata and nothing landed on-chain. Fail here instead.
WL_ADMIN_ROLE="$(cast keccak "WHITELIST_ADMIN")"
HAS_WL="$(cast call "$IR" 'hasRole(bytes32,address)(bool)' "$WL_ADMIN_ROLE" "$ADMIN_ADDR" --rpc-url "$RPC" 2>/dev/null || true)"
if [ -z "$HAS_WL" ]; then
  # UNREADABLE is not the same answer as `false`, and must never be reported as one: the read itself
  # failed (RPC hiccup, wrong/absent contract at $IR), so we know nothing about the signer. admin-api's
  # own authority_preflight resolves this case to Unknown and never refuses to boot on it - this script
  # must not be stricter than the backend on the identical question, and must not accuse a correct key.
  echo "  WARNING: could not read hasRole(WHITELIST_ADMIN, $ADMIN_ADDR) on IssuerRegistry $IR." >&2
  echo "           The signer's authority is UNRESOLVED - this is not evidence it is the wrong key." >&2
  echo "           Check $IR is an IssuerRegistry on this chain and that $RPC is healthy; if the key" >&2
  echo "           really lacks WHITELIST_ADMIN, portal grants will come back disposition:\"proposed\"." >&2
elif [ "$HAS_WL" != "true" ]; then
  MSG="admin signer $ADMIN_ADDR does NOT hold WHITELIST_ADMIN on IssuerRegistry $IR (hasRole -> $HAS_WL).
  Every whitelist grant from the portal would come back disposition:\"proposed\" with unsigned calldata
  and NOTHING would land on-chain, while the stack looked healthy.
  Fix: set GOVERNANCE_PRIVATE_KEY in contracts/.env to the key that holds WHITELIST_ADMIN.
  (DEPLOYER_PRIVATE_KEY is the RETIRED deployer EOA - it lost this role in governance Phase-2.)
  To boot anyway for a genuine propose-for-external-signing setup: ADMIN_PROPOSE_ONLY=1
  (or its alias ALLOW_UNAUTHORIZED_ADMIN_SIGNER=1)"
  if [ "$PROPOSE_ONLY_DECLARED" = "1" ]; then
    echo "  WARNING: $MSG" >&2
  else
    die "$MSG"
  fi
else
  echo "  admin signer          $ADMIN_ADDR holds WHITELIST_ADMIN  ok"
fi

# GOVERNMENT chain backend. `live` (default) = real ROAX. The government stack used to select its
# in-process MemChain whenever DEMO_MODE was set - which contracts/.env sets - so its verify/records
# surfaces were simulated while /health still reported chainId 135. Simulation is now opt-in only.
GOV_CHAIN_BACKEND="${GOV_CHAIN_BACKEND:-live}"
GOV_SIGNER_KEY="${GOV_SIGNER_KEY:-}"
TRAVEL_CLEARANCE_ISSUER_ADDR="${TRAVEL_CLEARANCE_ISSUER_ADDR:-}"
# The accepted values are the match arm in stacks/government/api/src/main.rs (the source of truth), which
# lowercases first and process::exit(1)s on anything else. Mirror BOTH halves: an alias like `rpc` is
# genuinely live and must not be announced as simulated, and an unrecognised value must die HERE rather
# than printing a clean boot while government-api exits inside the backgrounded run().
GOV_BACKEND_LC="$(echo "$GOV_CHAIN_BACKEND" | tr 'A-Z' 'a-z')"
case "$GOV_BACKEND_LC" in
  live|alloy|rpc) GOV_SIMULATED=0 ;;
  mem|memory|simulated|sim) GOV_SIMULATED=1 ;;
  *) die "GOV_CHAIN_BACKEND='$GOV_CHAIN_BACKEND' is not recognised - government-api would exit(1) at boot.
  Use 'live' (real RPC node, the default) or 'mem' (in-process simulation; nothing is broadcast).
  See the match arm in stacks/government/api/src/main.rs for every accepted alias." ;;
esac
if [ "$GOV_SIMULATED" = "0" ]; then
  TC_ROLE="$(cast keccak "TRAVEL_CLEARANCE")"
  if [ -z "$GOV_SIGNER_KEY" ]; then
    echo "  government            LIVE chain, NO signer -> /issue can only dry_run (no on-chain anchor)."
    echo "                        Provision one with: scripts/demo-provision-government.sh"
  else
    GOV_ADDR="$(cast wallet address --private-key "$GOV_SIGNER_KEY")"
    GOV_BAL="$(cast balance "$GOV_ADDR" --rpc-url "$RPC" 2>/dev/null || echo 0)"
    GOV_WL="$(cast call "$IR" 'isWhitelistedFor(bytes32,address)(bool)' "$TC_ROLE" "$GOV_ADDR" --rpc-url "$RPC" 2>/dev/null || true)"
    echo "  government signer     $GOV_ADDR  balance ${GOV_BAL} wei  TRAVEL_CLEARANCE whitelisted=${GOV_WL:-unreadable}"
    [ "$GOV_BAL" != "0" ] || echo "    WARNING: unfunded - on-chain issuance will fail. scripts/demo-provision-government.sh funds it." >&2
    [ "$GOV_WL" = "true" ] || echo "    WARNING: not whitelisted for TRAVEL_CLEARANCE - DogTagIssuer.issue() reverts NotWhitelisted." >&2
  fi
  # A configured clone is a config error whether or not a signer exists, so this is checked either way.
  # `DogTagIssuer.issue` is onlyWhitelisted against the clone's OWN registry(), so a clone bound to a
  # superseded registry fails closed even after a correct whitelistFor on the one the stack uses - it
  # passes every other preflight line and only surfaces on the first issuance. Same check the
  # scripts/demo-provision-government.sh clone step makes.
  if [ -z "$TRAVEL_CLEARANCE_ISSUER_ADDR" ]; then
    echo "    WARNING: TRAVEL_CLEARANCE_ISSUER_ADDR unset - no clone to anchor into; /issue will dry_run." >&2
  else
    CLONE_REGISTRY="$(cast call "$TRAVEL_CLEARANCE_ISSUER_ADDR" 'registry()(address)' --rpc-url "$RPC" 2>/dev/null || true)"
    [ -n "$CLONE_REGISTRY" ] || die "TRAVEL_CLEARANCE_ISSUER_ADDR=$TRAVEL_CLEARANCE_ISSUER_ADDR has no registry() - not a DogTagIssuer clone (or wrong chain)."
    if [ "$(echo "$CLONE_REGISTRY" | tr 'A-Z' 'a-z')" != "$(echo "$IR" | tr 'A-Z' 'a-z')" ]; then
      die "TRAVEL_CLEARANCE clone $TRAVEL_CLEARANCE_ISSUER_ADDR is bound to registry $CLONE_REGISTRY but the stack uses ISSUER_REGISTRY_ADDR=$IR.
  Its onlyWhitelisted gate reads a registry nobody writes to, so issue() reverts NotWhitelisted even
  after a correct whitelistFor. See contracts/deployments/roax.json -> government_clones (the fresh set)
  vs government_clones_deadRegistry_legacy. Re-provision: scripts/demo-provision-government.sh"
    fi
    CLONE_RT="$(cast call "$TRAVEL_CLEARANCE_ISSUER_ADDR" 'recordType()(bytes32)' --rpc-url "$RPC" 2>/dev/null || true)"
    [ "$(echo "${CLONE_RT:-}" | tr 'A-Z' 'a-z')" = "$(echo "$TC_ROLE" | tr 'A-Z' 'a-z')" ] \
      || die "TRAVEL_CLEARANCE clone $TRAVEL_CLEARANCE_ISSUER_ADDR has recordType ${CLONE_RT:-unreadable}, expected keccak256(TRAVEL_CLEARANCE) $TC_ROLE."
    echo "  government clone      $TRAVEL_CLEARANCE_ISSUER_ADDR -> registry $IR, TRAVEL_CLEARANCE  ok"
  fi
else
  echo "  government            GOV_CHAIN_BACKEND=$GOV_CHAIN_BACKEND -> SIMULATED chain (nothing broadcast)."
fi
echo

# Truncate the PID list only once every preflight refusal is behind us. Doing it at the top meant a
# re-run against an ALREADY-RUNNING stack wiped the record scripts/demo-down.sh kills by, and then
# refused to boot - orphaning the running services with nothing left pointing at them.
: > .demo/pids

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
# The scanner takes one ATOMIC generation set rather than three parallel address variables. Remove the
# legacy singleton variables inherited from contracts/.env for this child: startup deliberately rejects
# both forms together so stale legacy values cannot silently disagree with INDEXER_GENERATIONS.
INDEXER_GENERATIONS_JSON="[{\"factory\":\"$FACTORY\",\"issuerRegistry\":\"$IR\",\"verificationRegistry\":\"$VR\",\"seedClones\":[]}]"
run indexer-api ":46001" env \
  -u FACTORY_ADDR -u ISSUER_REGISTRY_ADDR -u VERIFICATION_REGISTRY_CONSENT_ADDR -u SEED_CLONES \
  INDEXER_DEMO_MODE=1 PORT=46001 INDEXER_GENERATIONS="$INDEXER_GENERATIONS_JSON" \
  "$ROOT/target/release/indexer-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_ADDR=$VR \
  SBT_ADDR=$SBT FACTORY_ADDR=$FACTORY PROVIDER_REGISTRY_ADDR=$PROVIDER_REGISTRY \
  ADMIN_PRIVATE_KEY=$ADMIN_PK ADMIN_ADDRESS=$ADMIN_ADDR PORT=39742 \
  ADMIN_PROPOSE_ONLY="$ADMIN_PROPOSE_ONLY" \
  run admin-api ":39742" "$ROOT/target/release/admin-api"
# Every verifier/issuance process receives the same owner-hidden pair. PROFILE_ISSUER is a real
# factory clone: roots are issue(R)'d there, while mintCustodial seals the same R on the SBT.
# FACTORY_ADDR goes to every vet-api instance because all three serve POST /verify/credential, whose
# issuer-whitelist pillar resolves the issuing clone from the factory's write-once rootIssuer[R]. A
# deployment without it cannot evaluate that pillar and reports it `unavailableNoFactoryConfigured` -
# which is honest, but leaves a forged issuer.documentStore refused by nothing but integrity.
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_CONSENT_ADDR=$VR \
  SBT_CONSENT_ADDR=$SBT PROFILE_ISSUER_ADDR=$PROFILE_ISSUER FACTORY_ADDR=$FACTORY \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="Seaport Vet" ISSUER_DOMAIN=vet.local \
  BUSINESS_ID=biz-vet CONFIRMATIONS=1 PORT=41874 DEPLOYMENT_URL="${VET_PUBLIC_URL:-http://$LAN_IP:41874}" \
  INDEXER_API_BASE=http://localhost:46001 INDEXER_SCOPED_TOKEN=dogtag-indexer-vet-demo-token \
  CUSTODY_SEAL_PATH="$ROOT/.demo/vet-custody.json" \
  run vet-api ":41874" "$ROOT/target/release/vet-api"
ADMIN_PASSWORD=admin OPERATOR_PASSWORD=operator CENTRAL_HMAC_SECRET=$HMAC \
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_CONSENT_ADDR=$VR \
  SBT_CONSENT_ADDR=$SBT PROFILE_ISSUER_ADDR=$PROFILE_ISSUER FACTORY_ADDR=$FACTORY \
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
  ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR VERIFICATION_REGISTRY_CONSENT_ADDR=$VR \
  SBT_CONSENT_ADDR=$SBT PROFILE_ISSUER_ADDR=$PROFILE_ISSUER FACTORY_ADDR=$FACTORY \
  VACCINATION_ISSUER_ADDR=$VACC_CLONE ISSUER_NAME="DogTag Prover" ISSUER_DOMAIN=prover.local \
  BUSINESS_ID=biz-prover CONFIRMATIONS=1 PORT=41875 DEPLOYMENT_URL="${PROVER_PUBLIC_URL:-http://$LAN_IP:41875}" \
  CIRCUITS_BUILD_DIR="$ROOT/circuits/build" \
  CUSTODY_SEAL_PATH="$ROOT/.demo/prover-custody.json" \
  run prover-api ":41875" "$ROOT/target/prover/release/vet-api"

# GOVERNMENT stack — a SEPARATE deployable (its own government-api binary, own port, own DB), not a
# vet-api re-run. It runs against LIVE ROAX (GOV_CHAIN_BACKEND=live, the default): reads AND on-chain
# TRAVEL_CLEARANCE issuance are real. It used to fall onto its in-process MemChain purely because
# contracts/.env sets DEMO_MODE, so its verify/records surfaces were simulated while /health still
# claimed chainId 135; the chain backend is now an explicit, separate switch from the demo store.
# On-chain issuance needs a funded, whitelisted GOV_SIGNER_KEY + a DogTagIssuer clone
# (TRAVEL_CLEARANCE_ISSUER_ADDR) — provision both with scripts/demo-provision-government.sh. Without
# them the stack still runs live-read-only and /issue builds+persists via dry_run.
# GOV_CHAIN_BACKEND=mem opts INTO simulation; /health then reports backend="simulated", chainId=null.
# FACTORY_ADDR is LINK 1 of the issuer↔domain chain and is what makes verification resolve the issuing
# clone from the chain's write-once `rootIssuer[R]`. Without it the government backend falls back to the
# document's own `documentStore` for the `isValid` pillar, reads no on-chain issuer name at all, and
# reports every binding as `unavailable` — i.e. the whole three-link chain is dark in the showcase, which
# is precisely what this change removes. ISSUER_DOMAIN_REGISTRY_ADDR is resolved from the ledger, which
# now carries a deployed IssuerDomainRegistry, so the on-chain-claim link is READ rather than skipped. The
# zero-address fallback is retained for a ledger that has no such key: that is the honest `unavailable`,
# not an invented address. Note a deployed-but-EMPTY registry still renders `unavailable` for every clone
# until a domain is bound - deploying the contract publishes no claims by itself.
# NOT government-scoped despite where it is first used: there is ONE IssuerDomainRegistry for the
# protocol, and the admin portal's verification bench reads the SAME address (see the admin-web line
# below). Keeping one variable is what stops the two surfaces disagreeing about which registry is
# authoritative.
ISSUER_DOMAIN_REGISTRY="${ISSUER_DOMAIN_REGISTRY_ADDR:-$(ledger_addr IssuerDomainRegistry)}"
ISSUER_DOMAIN_REGISTRY="${ISSUER_DOMAIN_REGISTRY:-0x0000000000000000000000000000000000000000}"
ROAX_RPC=$RPC ISSUER_REGISTRY_ADDR=$IR ISSUER_NAME="Example Competent Authority" ISSUER_DOMAIN=gov.local \
  VERIFICATION_REGISTRY_ADDR=$VR \
  FACTORY_ADDR=$FACTORY \
  ISSUER_DOMAIN_REGISTRY_ADDR="$ISSUER_DOMAIN_REGISTRY" \
  DNS_DOH_ENDPOINT="${DNS_DOH_ENDPOINT:-https://cloudflare-dns.com/dns-query}" \
  CHAIN_ID="$CHAIN_ID_EXPECTED" PORT=44832 DEPLOYMENT_URL="${GOV_PUBLIC_URL:-http://$LAN_IP:44832}" \
  GOV_CHAIN_BACKEND="$GOV_CHAIN_BACKEND" \
  TRAVEL_CLEARANCE_ISSUER_ADDR="$TRAVEL_CLEARANCE_ISSUER_ADDR" GOV_SIGNER_KEY="$GOV_SIGNER_KEY" \
  GOV_API_TOKEN="${GOV_API_TOKEN:-dogtag-gov-demo-token}" \
  INDEXER_API_BASE=http://localhost:46001 INDEXER_OVERSIGHT_TOKEN=dogtag-indexer-oversight-demo-token \
  run government-api ":44832" "$ROOT/target/release/government-api"

echo "Starting portals (vite dev):"
# The verification bench reads VITE_ISSUER_DOMAIN_REGISTRY_ADDR and has NO fallback by design, so
# without this the bench's on-chain-domain row reports "could not run" in the showcase no matter what
# an issuer has published. VITE_* is inlined by vite at startup, so a portal already running when this
# address changes needs a restart to pick it up.
run admin-web ":39741" env VITE_DEMO_MODE=1 \
  VITE_ISSUER_DOMAIN_REGISTRY_ADDR="$ISSUER_DOMAIN_REGISTRY" \
  pnpm --filter @dogtag/admin-web dev
# The S-17 content mirror, so the provider self-service page can publish into the demo indexer
# instead of reporting "no content mirror is configured". Both are needed: the base names the
# mirror and the token is the ONLY bearer its PUT accepts. LAN_IP rather than localhost, because the
# browser holding these values may not be on this machine - the same reason every DEPLOYMENT_URL
# above uses it.
#
# This wires the DEMO ONLY. The shipped `.env.example` entries stay BLANK and fallback-free, so a
# real deployment still refuses to publish until an operator sets both deliberately.
#
# The token is PUBLIC BY CONSTRUCTION: vite inlines it into the bundle, so every visitor to the demo
# portal holds it. That is fine precisely because of what it grants - publish bytes that hash to
# their own address, bounded by the mirror's caps - and it is deliberately NOT either well-known
# oversight token, which would put read authority over the event feed into the same bundle.
DEMO_MIRROR_BASE="${DEMO_MIRROR_BASE:-http://$LAN_IP:46001}"
DEMO_MIRROR_INGEST_TOKEN=dogtag-indexer-mirror-ingest-demo-token
run vet-web    ":41873" env VITE_DEMO_MODE=1 VITE_DOGTAG_ISSUER_ADDR="$VACC_CLONE" \
  VITE_CONTENT_MIRROR_BASE="$DEMO_MIRROR_BASE" VITE_CONTENT_MIRROR_TOKEN="$DEMO_MIRROR_INGEST_TOKEN" \
  pnpm --filter @dogtag/vet-web dev
run groomer-web ":43617" env VITE_DEMO_MODE=1 VITE_DOGTAG_ISSUER_ADDR="$VACC_CLONE" \
  VITE_CONTENT_MIRROR_BASE="$DEMO_MIRROR_BASE" VITE_CONTENT_MIRROR_TOKEN="$DEMO_MIRROR_INGEST_TOKEN" \
  pnpm --filter @dogtag/groomer-web dev
run government-web ":44831" env VITE_DEMO_MODE=1 pnpm --filter @dogtag/government-web dev
# OWNER (holder) wallet — local records, selective disclosure, and verification receipts. The native
# apps own the owner-hidden scan/prove flow; the browser wallet has no backend or prover wiring.
run owner-web ":45931" pnpm --filter @dogtag/owner-web dev

echo
echo "UP. Portals:  admin http://localhost:39741  vet http://localhost:41873  groomer http://localhost:43617  government http://localhost:44831  owner-wallet http://localhost:45931"
echo "Backends:     admin :39742  vet :41874  groomer :43618  government :44832  prover :41875  indexer :46001   (ROAX chainId 135)"
echo "Three-role showcase: scripts/e2e-roles.sh --live   (vet ISSUES -> government VERIFIES -> government ISSUES)"
echo "Prover svc:   POST :41875/prove-consent  (32-bit-Android fallback; set PROVER_PUBLIC_URL to tunnel it)"
echo "Owner wallet: http://localhost:45931  (Receive a wrapped doc -> inspect receipts or share selected fields)"
echo "Next: provision the fresh issuance/verification roles, then Issue -> Create QR -> scan on phone (docs/DEMO.md)."
echo "For the PHONE: set its server base to this Mac's LAN IP (not localhost) — see docs/DEMO.md."
