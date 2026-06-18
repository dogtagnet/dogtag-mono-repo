#!/usr/bin/env bash
# DogTag PRODUCTION bring-up — builds + starts the admin, vet and groomer stacks via docker compose,
# each with its own Mongo (persistent), Caddy TLS reverse-proxy, and the MongoStore-capable api image
# (built `--features mongo`). This is the REMOTE counterpart to scripts/demo-up.sh.
#
#   scripts/remote-up.sh
#
# Unlike the demo this does NOT auto-genesis custody and shows no demo buttons (VITE_DEMO_MODE unset).
# After the stacks are up you run the manual custody + onboarding runbook printed at the end.
#
# Prerequisites per stack (stacks/<x>/.env, copied from stacks/<x>/.env.example and filled in):
#   - MONGO_URI                  (persistent store; the api fail-closes if it can't connect)
#   - DOMAIN                     (public hostname for Caddy auto-TLS; DNS A record + ports 80/443)
#   - Strong secrets             (OPERATOR_PASSWORD, ADMIN_PASSWORD, CENTRAL_HMAC_SECRET;
#                                 admin also ADMIN_PRIVATE_KEY + ADMIN_ADDRESS) — `openssl rand -hex 32`
#   - ROAX_RPC + CHAIN_ID + the *_ADDR contract addresses (config-only chain swap)
# Hardening knobs (DNS_CHECK=doh, CONFIRMATIONS=2, ADMIN_LOOPBACK_ONLY=1) are set in compose; this
# script enforces them as a sanity check.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

STACKS=(admin vet groomer)

# ---- Preflight: every stack must have a populated .env (no placeholder secrets) --------------------
fail() { echo "ERROR: $*" >&2; exit 1; }

echo "==> Preflight: checking stacks/<x>/.env files"
for s in "${STACKS[@]}"; do
  envf="$ROOT/stacks/$s/.env"
  [ -f "$envf" ] || fail "missing $envf — copy stacks/$s/.env.example to .env and fill it in"

  # Required vars: every stack needs these...
  required=(MONGO_URI DOMAIN ADMIN_PASSWORD CENTRAL_HMAC_SECRET)
  # ...business stacks also need an operator password...
  [ "$s" != "admin" ] && required+=(OPERATOR_PASSWORD)
  # ...and the admin stack needs the on-chain signer.
  [ "$s" = "admin" ] && required+=(ADMIN_PRIVATE_KEY ADMIN_ADDRESS)

  # Source in a subshell, validate, then check placeholders / demo flag — all without leaking vars.
  STACK="$s" REQUIRED="${required[*]}" bash -c '
    set -euo pipefail
    set -a; source "$0"; set +a
    for v in $REQUIRED; do
      [ -n "${!v:-}" ] || { echo "ERROR: $v must be set in stacks/$STACK/.env" >&2; exit 1; }
      case "${!v}" in *change-me*) echo "ERROR: $v in stacks/$STACK/.env is still a placeholder" >&2; exit 1;; esac
    done
    if [ "${VITE_DEMO_MODE:-}" = "1" ] || [ "${VITE_DEMO_MODE:-}" = "true" ]; then
      echo "ERROR: VITE_DEMO_MODE is set in stacks/$STACK/.env — must be UNSET in production" >&2; exit 1
    fi
  ' "$envf" || fail "stacks/$s/.env failed validation (see above)"
  echo "    ok: stacks/$s/.env"
done

# ---- Build + start each stack ----------------------------------------------------------------------
# Hardening defaults the api expects (compose also sets these; we export so any ${VAR} resolves).
export DNS_CHECK="${DNS_CHECK:-doh}"
export CONFIRMATIONS="${CONFIRMATIONS:-2}"
export ADMIN_LOOPBACK_ONLY="${ADMIN_LOOPBACK_ONLY:-1}"
export FEATURES=mongo

for s in "${STACKS[@]}"; do
  compose="$ROOT/stacks/$s/docker-compose.yml"
  echo
  echo "==> [$s] docker compose build (FEATURES=mongo)"
  docker compose -f "$compose" build --build-arg FEATURES=mongo

  echo "==> [$s] docker compose up -d"
  docker compose -f "$compose" up -d
done

echo
echo "================================================================================"
echo "UP. Stacks running (admin, vet, groomer) with persistent Mongo + Caddy TLS."
echo "Each is reachable at https://<that stack's DOMAIN>/  (Caddy auto-issues the cert)."
echo "================================================================================"
echo
echo "MANUAL RUNBOOK (no demo automation — operators key everything in):"
echo
echo "  1) CUSTODY GENESIS (per business stack: vet, groomer, and the admin signer):"
echo "       a. Open the portal Setup wizard at https://<DOMAIN>/"
echo "       b. Genesis a new 24-word BIP-39 seed; WRITE THE WORDS DOWN (no autofill in production)."
echo "       c. Re-type the challenge words to confirm."
echo "       d. Set a STRONG passphrase (scrypt/age-encrypted; stored as a CustodyBlob in Mongo)."
echo "       e. Unlock with that passphrase. NOTE: you must re-unlock after every API restart."
echo "          (/admin/* is loopback-only + proxy-denied — run these from the host, not the internet.)"
echo
echo "  2) CENTRAL ONBOARDING (on the admin stack, after its custody is unlocked):"
echo "       a. Central operator logs in."
echo "       b. Register the business (real name / lat / lng / apiBaseUrl=https://<DOMAIN> / domain)."
echo "       c. Business operator applies as an issuer (issuerEntityId, addresses, recordTypes, domain,"
echo "          documentStore, license)."
echo "       d. Publish the DNS TXT record:  dogtag-verify=<lowercased documentStore address>"
echo "       e. Central APPROVES the issuer-application (DoH DNS check -> on-chain whitelistFor)."
echo
echo "  3) BACK UP the Mongo data volume for EACH stack (custody lives there):"
echo "       vet -> vetdata   admin -> admindata   groomer -> groomerdata"
echo
echo "Logs:  docker compose -f stacks/<x>/docker-compose.yml logs -f"
echo "Down:  docker compose -f stacks/<x>/docker-compose.yml down"
