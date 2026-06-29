# DogTag — REMOTE deployment (Tier 2: self-hosted, persistent + TLS, still ROAX testnet)

**Goal / you'll end with:** the three stacks (vet, groomer, central/admin) running on **your own
server** behind **Caddy auto-TLS** on **real domains**, backed by **persistent Mongo**, with custody
sealed per business, operators keying everything in by hand (no demo autofill), and phones onboarding
against your hosts — all still on the **live ROAX testnet** (chainId **135**, the **same contract
addresses**, **no redeploy**).

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps. Every
state-changing step has a **Verify.** block and a **STOP if…** gate. Placeholders look like `<DOMAIN>`
and are defined the first time they appear.

This is the **owner doc** for the canonical **backend `.env` table** (§3) and the **portal `VITE_`
table** (§3). Other docs link here rather than copying them.

> Tier map: **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)** = Tier 1 (one Mac, demo/dev). **This doc**
> = Tier 2 (self-host, testnet). **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** = Tier 3
> (REMOTE + go-live hardening: chain swap, multi-party ceremony, timelock). The contract-deploy runbook
> is **[DEPLOY.md](./DEPLOY.md)**.

---

## 0. What REMOTE is — and is NOT

REMOTE (Tier 2) **is**:

- **Persistent.** Each stack has its own **MongoStore** (`MONGO_URI`); records, sessions, and the
  encrypted custody seed survive restarts. The api is **fail-closed**: if `MONGO_URI` is set but
  unreachable, the api **refuses to boot** (it never silently falls back to MemStore).
- **TLS on a real domain.** Each stack runs **Caddy 2** ([`deploy/Caddyfile`](../deploy/Caddyfile)),
  which auto-issues a Let's Encrypt cert for that stack's `DOMAIN` and reverse-proxies to the internal
  nginx `web` service.
- **Real DNS legitimacy.** `DNS_CHECK=doh` — issuer (and EXPORT groomer) legitimacy is verified via
  Cloudflare **DNS-over-HTTPS** against a `dogtag-verify=` TXT record (§4, §7).
- **Manual / no autofill.** `VITE_DEMO_MODE` is **unset** — no prefilled forms, no demo buttons, no
  stashed seed. Operators type passwords and re-type the genesis challenge words by hand.

REMOTE **is NOT**:

- **A production chain.** It still runs on **ROAX testnet (135)** with the **same addresses**. Moving
  to another / production chain is Tier 3 → **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)**.
- **A complete proving setup by default.** `scripts/remote-up.sh` starts **no prover-service**.
  32-bit-only Android users need one, which **you run yourself** — see **§8**.

### LOCAL vs REMOTE at a glance

The single switch is **`VITE_DEMO_MODE`** (portal build-time flag): set = demo, **unset = production**.

| Aspect | LOCAL (`scripts/demo-up.sh`) | REMOTE (`scripts/remote-up.sh` / compose) |
|---|---|---|
| Form entry | Auto-filled, demo buttons shown (`VITE_DEMO_MODE=1`) | **Empty, no demo buttons** (flag unset) |
| Operator/admin passwords | Demo prefilled (`operator` / `admin`) | Strong, env-set, **typed** |
| Genesis seed | Stashed + auto-filled into confirm | Operator **reads + re-types** challenge words |
| Storage | `MemStore` (records/sessions ephemeral; **restart = re-unlock** — custody seal in `.demo/*-custody.json`; records/sessions re-created) | `MongoStore` (persistent; back up the volume) |
| Networking | LAN IP or cloudflared tunnel | Real domain + **TLS** (Caddy auto-HTTPS) |
| DNS legitimacy | `DNS_CHECK=skip` (`.local`); phone groomer-DNS skipped | `DNS_CHECK=doh` + real `dogtag-verify=` TXT (issuer **and** EXPORT groomer, §4) |
| `/admin/*` exposure | On the main listener (single host) | **Loopback-isolated** + proxy-denied publicly |
| Confirmations | `CONFIRMATIONS=1` | `CONFIRMATIONS=2` |
| Prover-service | Started on `:41875` (32-bit-Android fallback) | **Not started** — run it yourself (§8) |
| Chain | ROAX testnet (135) | **Same** ROAX testnet (135), same addresses |

---

## 1. Prerequisites for REMOTE

> **Before you start, you need:** (a) **Docker + Docker Compose** on the host; (b) a **domain you
> control** with **DNS-record access** (you'll add A records and TXT records); (c) **`openssl`** (to
> generate secrets); (d) the repo checked out on the host. The full canonical install matrix (macOS +
> Linux, per-tool "needed by" tags) is in **[PREREQUISITES.md](./PREREQUISITES.md)**.

Verify the toolchain.

```bash
docker --version          # any recent Docker Engine / Desktop
docker compose version    # Compose v2 (the `docker compose` subcommand, not `docker-compose`)
openssl version           # any OpenSSL/LibreSSL
dig -v 2>&1 | head -1      # for the DNS preflight in §4 (bind-tools / dnsutils)
```

**Verify.** Each command prints a version. `docker compose version` must show **v2.x** (this repo uses
the `docker compose` subcommand throughout).

**STOP if** `docker compose version` errors:
- **Symptom:** `docker: 'compose' is not a docker command`.
- **Cause:** only the legacy `docker-compose` v1 is installed.
- **Fix:** install Compose v2 (Docker Desktop bundles it; on Linux install the `docker-compose-plugin`
  package). See **[PREREQUISITES.md](./PREREQUISITES.md)**.

You do **not** need the Rust toolchain, foundry, or `circuits/build` for the base REMOTE bring-up — the
api images are built inside Docker. You **only** need the Rust toolchain (or a prover image) **if you
run a prover-service** for 32-bit Android (§8).

---

## 2. Topology

- **Per business (vet / groomer): self-hosted.** Each business runs its own
  `stacks/<vet|groomer>` stack — `web` (nginx serving the built SPA) + `api` (the **`vet-api`** binary;
  the groomer is the **same binary** run with `BUSINESS_TYPE=groomer`) + **its own Mongo** + **Caddy**
  (TLS). Custody (the issuer signer) lives in that business's Mongo and never leaves the box.
- **One central / admin stack (you host):** `stacks/admin` — registry/discovery, issuer whitelisting,
  mobile API, appointment source-of-truth, erasure. It holds the **admin protocol signer** that
  broadcasts `whitelistFor` / SBT `mint`.
- **Contracts are reused as-is.** All stacks point at the **live ROAX addresses** — **no redeploy**.
  Don't transcribe addresses into your `.env` from memory; copy them from
  [`contracts/deployments/roax.json`](../contracts/deployments/roax.json) (the `.env.example` files are
  already pre-filled with the current ones). For a human-readable reference see the **Address Book** in
  **[DEPLOYMENT.md](./DEPLOYMENT.md)** (the canonical table; this doc does not reprint addresses).
- **Per-business `documentStore` clones** are created **centrally** (the factory is `onlyOwner`) via
  `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), businessAddr)`. The resulting clone
  address is what the business puts in its issuer application **and** in its `dogtag-verify=` DNS TXT
  (§7). (Factory address: see the Address Book / `roax.json`.)

### REMOTE service + port table

`docker compose`; Mongo is **internal-only** on every stack (never published to the host).

| Stack | Caddy (host) | api (host) | api (container) | mongo | back-up volume |
|---|---|---|---|---|---|
| admin / central | **80, 443** | 39742 | 39742 | **27017 internal-only** | `admindata` |
| vet | **80, 443** | 41874 | 41874 | **27017 internal-only** | `vetdata` |
| groomer | **80, 443** | 43618 | **43618** (→ container 43618) | **27017 internal-only** | `groomerdata` |
| prover-service | (manual; **not** started by `remote-up.sh`) | operator-chosen | — | n/a | n/a |

- Each stack's `web` (nginx) is **`expose: 80` internal-only**; Caddy reaches it as `web:80`. There is
  no host port for `web`.
- Mongo is **27017 internal-only** on every stack (compose uses `expose: "27017"`, never `ports:`).
- `/admin/*` binds a **separate `127.0.0.1:${ADMIN_PORT}`** listener (default = **`PORT+1`**) when
  `ADMIN_LOOPBACK_ONLY=1`. So vet's admin listener is `41875` (which equals the LOCAL prover port —
  harmless; they never co-run), admin's is `39743`, groomer's is `43619`.

---

## 3. Configure each stack `.env`

Copy each template, then fill it in. The `.env.example` files are **pre-filled with the current ROAX
addresses and sensible non-secret defaults** — you mainly fill in the **secrets** and your **domains**.

```bash
# Run from the repo root on the host.
cp stacks/admin/.env.example   stacks/admin/.env
cp stacks/vet/.env.example     stacks/vet/.env
cp stacks/groomer/.env.example stacks/groomer/.env
```

Then edit each `.env` per the tables below. Generate **every secret** with `openssl rand -hex 32`:

```bash
openssl rand -hex 32   # run once per secret slot; never reuse the demo defaults
```

### Backend `.env` keys (canonical — owned by this doc)

Verified against `stacks/{admin,vet,groomer}/.env.example`.

| Key | Stacks | Purpose | Demo value | Prod / REMOTE guidance |
|---|---|---|---|---|
| `ROAX_RPC` | all | chain RPC | `https://devrpc.roax.net` | keep for testnet; new RPC on chain swap |
| `CHAIN_ID` | all | chain id | `135` | keep `135` for REMOTE; swap target for prod |
| `MONGO_URI` | all | persistent store; **fail-closed** | unset → MemStore | `mongodb://mongo:27017/dogtag` |
| `MONGO_DB` | all | db name | `dogtag` | `dogtag` |
| `PORT` | all | api listener | 39742 / 41874 / 43618 | keep default |
| `ADMIN_PORT` | all | loopback admin listener | default **PORT+1** | leave default (commented) |
| `DEPLOYMENT_URL` | all | public base; **QR host** (vet/groomer); JWT issuer | LAN-IP via `*_PUBLIC_URL` | `https://<DOMAIN>` |
| `DEPLOYMENT_DOMAIN` | vet, groomer | **NO-OP — not read by code; do NOT rely on it** | unset | use `ISSUER_DOMAIN` instead |
| `ISSUER_NAME` | all | display name | "Example Veterinary Clinic" / "Example Grooming Salon" / "DogTag Central" | real name |
| `ISSUER_DOMAIN` | all | **the real DNS-TXT issuer-domain binding** | `*.local` | your real domain |
| `ISSUER_REGISTRY_ADDR` | all | IssuerRegistry | (roax.json, pre-filled) | per chain |
| `VERIFICATION_REGISTRY_ADDR` | vet, groomer | **current** VR (`0x8bA836eCe9…`) | (roax.json, pre-filled) | current, **not** legacy |
| `CONSENT_KEY_REGISTRY_ADDR` | vet, groomer | gasless `bindConsentKeyFor` (`0xA74DDe4a9b…`) | (roax.json, pre-filled) | current, **not** legacy |
| `SBT_ADDR` | admin | DogTagSBT | (roax.json, pre-filled) | per chain |
| `PROFILE_DOCUMENT_STORE` | admin | SBT mint target | `=SBT_ADDR` | usually `=SBT_ADDR` |
| `VACCINATION_ISSUER_ADDR` | vet, groomer | per-recordType clone | `0x0…0` (set to the real clone for an issuer) | `0x0…0` for pure verifiers |
| `ADMIN_SIGNER_INDEX` | admin | HD signer index | `0` | `0` |
| `DNS_CHECK` | all | issuer DNS legitimacy | `skip` (local) | **`doh`** (enforced by `remote-up.sh`) |
| `CONFIRMATIONS` | all | reorg safety | `1` | **`2`** (enforced) |
| `ADMIN_LOOPBACK_ONLY` | all | bind `/admin/*` to `127.0.0.1:ADMIN_PORT` | unset | **`1`** (enforced) |
| `CORS_ALLOW_ORIGINS` | all | CORS allowlist | unset (permissive) | `https://<DOMAIN>` |
| `OPERATOR_PASSWORD` | vet, groomer | operator login (`POST /login`) | `operator` | **secret** → `openssl rand -hex 32` |
| `ADMIN_PASSWORD` | all | admin-session login (custody/console) | `admin` | **secret** → `openssl rand -hex 32` |
| `CENTRAL_HMAC_SECRET` | all | central↔business HMAC; **identical across all stacks** | `dev-central-hmac-secret` | **secret** → `openssl rand -hex 32` (same value everywhere) |
| `ADMIN_PRIVATE_KEY` | admin | on-chain signer (`whitelistFor` / SBT `mint`) | from `contracts/.env` | **secret** — dedicated **funded** EOA key |
| `ADMIN_ADDRESS` | admin | address of `ADMIN_PRIVATE_KEY` | from `contracts/.env` | derive from the key |
| `BUSINESS_ID` | vet, groomer | central registry id | `biz-vet-local` / `biz-groomer-local` | real id |
| `BUSINESS_TYPE` | groomer | run `vet-api` as groomer | `groomer` | `groomer` |
| `CENTRAL_BASE_URL` | vet, groomer | central api base for HMAC events | `http://localhost:39742` | `https://api.<DOMAIN>` (your admin stack) |
| `CIRCUITS_BUILD_DIR` | **prover only** | load real ArkProver vs StubProver | `circuits/build` | set **only** on the prover-service (§8) |
| `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_CALENDAR_ID` | vet, groomer | Phase-7 calendar OAuth | unset / `primary` | optional |

> **The admin stack has no** `OPERATOR_PASSWORD`, `VACCINATION_ISSUER_ADDR`, `VERIFICATION_REGISTRY_ADDR`,
> `CONSENT_KEY_REGISTRY_ADDR`, `BUSINESS_TYPE`, `CENTRAL_BASE_URL`, or `DEPLOYMENT_DOMAIN` — it is the
> central stack, not a business issuer.

### Portal `VITE_` keys (canonical — owned by this doc)

Verified against `stacks/{admin,vet,groomer}/web/.env.example`. These are **build-time** (baked into the
SPA bundle at `docker compose build`).

| Key | Purpose | Demo / default | REMOTE / prod |
|---|---|---|---|
| `VITE_DEMO_MODE` | the single LOCAL/REMOTE switch | `1` (demo-up.sh) | **UNSET** — `remote-up.sh` **rejects** the build if set |
| `VITE_{CENTRAL,VET,GROOMER}_API_BASE` | api base (via the `/api` proxy) | `/api` | `/api` |
| `VITE_{CENTRAL,VET,GROOMER}_API_PROXY` | dev proxy target | 39742 / 41874 / 43618 | n/a (build serves `/api`) |
| `VITE_REOWN_PROJECT_ID` | WalletConnect projectId | placeholder | real Reown id (needed only for browser-wallet mode) |
| `VITE_DEPLOYMENT_URL` | QR caption URL | localhost portal port | `https://<DOMAIN>` |
| `VITE_ROAX_RPC` | read-only chain RPC | `https://devrpc.roax.net` | per chain |
| `VITE_DOGTAG_ISSUER_ADDR` | per-recordType issuer for `isValid` polling | empty | optional |
| `VITE_ISSUER_REGISTRY_ADDR` / `VITE_DOGTAG_SBT_ADDR` / `VITE_VERIFICATION_REGISTRY_ADDR` / `VITE_POSEIDON6_ADDR` | contract addrs | (roax.json, pre-filled) | per chain |

> **Known template typo:** `stacks/vet/web/.env.example` ships `VITE_CENTRAL_API_BASE=http://localhost:41870`,
> which is **wrong** — the central (admin) api listens on **39742**. The correct value is
> **`http://localhost:39742`** (matching `stacks/admin/web/.env.example` and `stacks/groomer/web/.env.example`).
> Do **not** propagate `:41870`. For REMOTE you set `VITE_CENTRAL_API_BASE` to your central origin (or
> leave the `/api` proxy convention), so the typo only bites if you copy the literal vet template value.

### Call-outs (get these right)

- **`VITE_DEMO_MODE` must be UNSET.** It lives (commented) in each `web/.env.example`. If set to `1` or
  `true` in any stack `.env`, `remote-up.sh` **aborts preflight** (§5).
- **`DEPLOYMENT_DOMAIN` is a NO-OP** — it is **not read by code**. The real DNS-TXT binding is
  **`ISSUER_DOMAIN`** (and `DOMAIN`, which Caddy uses for TLS). Set `ISSUER_DOMAIN` to your real domain;
  don't rely on `DEPLOYMENT_DOMAIN`.
- **Secrets via `openssl rand -hex 32`** for `OPERATOR_PASSWORD`, `ADMIN_PASSWORD`,
  `CENTRAL_HMAC_SECRET`, and a dedicated funded `ADMIN_PRIVATE_KEY` (with its `ADMIN_ADDRESS`). Never
  reuse the demo defaults; never commit secrets.
- **The backends are fail-closed on boot.** Beyond `remote-up.sh`'s env preflight, each api binary itself
  **refuses to start in production** (neither `DEMO_MODE` nor `VITE_DEMO_MODE` set) if a required secret is
  unset/empty or still equal to its built-in dev default — `OPERATOR_PASSWORD` / `ADMIN_PASSWORD` /
  `CENTRAL_HMAC_SECRET` (vet, groomer) or `ADMIN_PASSWORD` / `ADMIN_PRIVATE_KEY` (admin). It exits with a
  `FATAL:` message naming every offending secret. Set `DEMO_MODE=1` to keep the convenient defaults for a
  local/demo run.
- **`CENTRAL_HMAC_SECRET` must be IDENTICAL across all stacks** (admin, vet, groomer). It signs the
  central↔business appointment-event HMAC. This is **distinct** from the per-business `hmacSecret` that
  `register_business` returns **once** at registration (§7) — keep both.

**STOP if** any `change-me` (or empty required secret) remains in a `.env` before you bring up:
- **Symptom:** `remote-up.sh` aborts with `… is still a placeholder` or `… must be set`.
- **Cause:** an unfilled secret slot.
- **Fix:** generate the value with `openssl rand -hex 32` and set it, then re-run.

---

## 4. DNS + TLS preflight

For **each** stack's `<DOMAIN>` (e.g. `vet.example.com`, `groomer.example.com`, `api.dogtag.io`):

1. Add a public DNS **A record** for `<DOMAIN>` → the host's public IP (and AAAA if you have IPv6).
2. Open inbound **TCP 80** (Let's Encrypt ACME HTTP-01 challenge + HTTP→HTTPS redirect) and **TCP 443**
   (public HTTPS) to the host. Replace: `<DOMAIN>` = the public hostname you set in that stack's `.env`.

Verify DNS resolves to your host and the ports are reachable (run **before** bring-up; the second curl
will only fully succeed once Caddy is up, but it confirms the port is open).

```bash
DOMAIN=vet.example.com                 # repeat for each stack's domain
dig +short A "$DOMAIN"                  # must print THIS host's public IP
nc -vz "$DOMAIN" 80 2>&1 | tail -1     # port 80 reachable
nc -vz "$DOMAIN" 443 2>&1 | tail -1    # port 443 reachable
```

**Verify.** `dig +short A <DOMAIN>` prints your host's public IP; both `nc` checks report the port
**open / succeeded**.

**STOP if** `dig` prints nothing or a different IP:
- **Symptom:** empty output, or an IP that isn't your host.
- **Cause:** missing/incorrect A record, or DNS not yet propagated.
- **Fix:** add/correct the A record and wait for propagation; re-run `dig`. Caddy **cannot** issue a
  cert until DNS points at the host **and** port 80 is reachable.

---

## 5. Bring up

Build and start all three stacks (persistent Mongo + Caddy TLS) with the one script.

```bash
# Run from the repo root on the host, after §3 (.env filled) and §4 (DNS + ports).
scripts/remote-up.sh
```

`scripts/remote-up.sh`:

- **Validates** each `stacks/<x>/.env`: the file **exists**, every required var is **set** (rejects
  required secrets that are **empty/unset**, and **separately** any literal **`change-me`**), and
  **rejects `VITE_DEMO_MODE`** (`1`/`true`). The `.env.example` templates ship secrets **BLANK** — fill
  every key whose value after `=` is empty (generate with `openssl rand -hex 32`). Required vars:
  `MONGO_URI`, `DOMAIN`, `ADMIN_PASSWORD`, `CENTRAL_HMAC_SECRET` (all stacks); plus `OPERATOR_PASSWORD`
  (business stacks); plus `ADMIN_PRIVATE_KEY` + `ADMIN_ADDRESS` (admin stack).
- **Enforces** the hardening defaults: **`FEATURES=mongo`** (build-arg → MongoStore-capable image),
  **`DNS_CHECK=doh`**, **`CONFIRMATIONS=2`**, **`ADMIN_LOOPBACK_ONLY=1`**.
- Builds each stack with `docker compose build --build-arg FEATURES=mongo`, then
  `docker compose up -d`. Caddy **auto-issues** the Let's Encrypt cert on first request and persists it
  in the `caddy_data` volume.
- **Does NOT automate genesis** — it prints the manual custody + onboarding runbook (§6–§7). It also
  **starts no prover-service** (§8).

**Per-stack alternative** (build/start/inspect one stack at a time):

```bash
make up-<x>                                                   # x = admin | vet | groomer
# or the explicit form:
docker compose -f stacks/<x>/docker-compose.yml build --build-arg FEATURES=mongo
docker compose -f stacks/<x>/docker-compose.yml up -d
docker compose -f stacks/<x>/docker-compose.yml logs -f      # tail
docker compose -f stacks/<x>/docker-compose.yml down         # stop
```

> **STOP — these bypass `remote-up.sh`'s preflight.** Both `make up-<x>` (which is just
> `cd stacks/<x> && docker compose up -d`) and the explicit `docker compose … up -d` skip
> `remote-up.sh`'s `.env` validation (empty-secret + `VITE_DEMO_MODE` rejection) **and** its hardening
> enforcement (`DNS_CHECK=doh` / `CONFIRMATIONS=2` / `ADMIN_LOOPBACK_ONLY=1`). (Compose hardcodes the
> `FEATURES=mongo` build-arg, so the image is still MongoStore-capable — but nothing checks your
> secrets or the demo flag.) Use them only for **inspection / per-stack restarts**; do the PRODUCTION
> bring-up via **`scripts/remote-up.sh`**.

**Verify.** Every api serves `GET /health` (no auth). Hit it through the TLS domain:

```bash
curl -fsS https://<DOMAIN>/health      # one per stack domain
```

Expected: `{"status":"ok"}`.

**STOP if** `curl https://<DOMAIN>/health` fails with a TLS error:
- **Symptom:** `SSL certificate problem` / connection refused on 443.
- **Cause:** Caddy hasn't issued the cert yet (DNS/port-80 not ready), or DNS still wrong.
- **Fix:** re-check §4 (A record + port 80), then `docker compose -f stacks/<x>/docker-compose.yml
  logs -f caddy` for the ACME error. Caddy retries automatically once DNS/ports are correct.

**STOP if** the api container restarts / health never goes green:
- **Symptom:** `depends_on` healthcheck fails; api keeps restarting.
- **Cause:** `MONGO_URI` set but Mongo unreachable — the api is **fail-closed** and refuses to boot.
- **Fix:** check `… logs -f mongo` and `… logs -f api`; confirm `MONGO_URI=mongodb://mongo:27017/dogtag`.

---

## 6. Custody runbook (manual — no autofill)

Per **business stack (vet, groomer)** and for the **admin signer**, on the portal Setup wizard (reached
through the TLS domain `https://<DOMAIN>/`):

1. **Genesis** a new **24-word BIP-39** seed. The words are shown **once** — **WRITE THEM DOWN**. There
   is **no autofill** in production (`VITE_DEMO_MODE` unset) and the seed is never stashed.
2. **Re-type the challenge words** to confirm (you key them in manually).
3. Set a **strong passphrase**. The seed is scrypt/age-encrypted under it and stored as a
   **`CustodyBlob` in Mongo**.
4. **Unlock** with that passphrase to wire the signer into the chain client.
5. **Re-unlock after EVERY api restart** — custody is **not** auto-unlocked. Records and the encrypted
   seed survive the restart, but the signer cannot sign until you `POST /admin/unlock` again.

**Where custody lives.** The encrypted seed is a **`CustodyBlob` in Mongo** (in the stack's data
volume) — **NOT on disk**. The legacy `KEYSTORE_PATH` / `seed.age` volume (`vetseed` / `adminseed` /
`groomerseed` mounted at `/data`) is **DEAD CODE** retained only for backward compat; do not rely on it.
Back up the **Mongo** volume (§10), not the seed volume.

**`/admin/*` exposure.** With `ADMIN_LOOPBACK_ONLY=1` (set by `remote-up.sh`), the custody / genesis /
unlock routes are served on a **separate `127.0.0.1:${ADMIN_PORT}`** listener (default `PORT+1`) and are
**omitted from the public `0.0.0.0:PORT` listener**. Caddy additionally **denies `/api/admin/*`** at the
edge (`respond @admin 403`, with a commented `remote_ip` CIDR allowlist for a trusted office IP/VPN).
Run admin actions **from the host** (or via the allowlisted CIDR) — never from the open internet.

**Rate-limiting.** `/login`, `/admin/login`, and `/admin/unlock` are rate-limited (HTTP **429** on
lockout).

The business signer also needs **on-chain funding + whitelisting** before it can issue (not automated
for production): fund the genesis signer with gas (PLASMA on ROAX) and have central **approve** its
issuer application (§7), which runs `whitelistFor`.

> **STOP — a lost passphrase is UNRECOVERABLE.** There is no reset and no backdoor. If you lose the
> passphrase, the custody seed cannot be decrypted; you must genesis a **new** signer and re-fund +
> re-whitelist it. Store the passphrase and the 24 words safely and separately.

---

## 7. On-chain onboarding (real endpoints, no demo buttons)

Forms are empty in production; operators key in real values. Endpoints below are verbatim from
[`stacks/admin/api/src/routes.rs`](../stacks/admin/api/src/routes.rs) and the ground-truth
[`scripts/e2e-smoke.sh`](../scripts/e2e-smoke.sh). The **central base** is your admin api (e.g.
`https://api.dogtag.io`). `admin/login` and `approve` are **admin-router** routes (loopback-only under
`ADMIN_LOOPBACK_ONLY=1`); `businesses` and `issuer-applications` POST are public.

Set the central base once, then run each block top-to-bottom (it captures returned values into shell
vars and chains them). `CENTRAL` is your admin api over TLS; the rest are the values you key in.

```bash
CENTRAL=https://api.<DOMAIN>          # your admin/central api base
CLONE=<clone address>                 # this business's documentStore clone (from the factory)
DOM=<DOMAIN>                          # this business's real domain
```

1. **Central admin login** (admin/loopback) → capture `token`. Run **from the host** (the admin router
   is loopback-only under `ADMIN_LOOPBACK_ONLY=1`):
   ```bash
   TOKEN=$(curl -fsS -X POST "$CENTRAL/v1/admin/login" \
     -H 'content-type: application/json' \
     -d "{\"password\":\"$ADMIN_PASSWORD\"}" | jq -r .token)
   ```
   **Verify.** `curl` exits `0` (HTTP **200**) and `[ -n "$TOKEN" ] && [ "$TOKEN" != null ]` — i.e. the
   token is non-empty. (A wrong `ADMIN_PASSWORD` returns **401** and `curl -f` exits non-zero.)
2. **Register the business** (admin-session) → capture `businessId` + a one-time `hmacSecret`:
   ```bash
   REG=$(curl -fsS -X POST "$CENTRAL/v1/businesses" \
     -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d "{\"type\":\"vet\",\"name\":\"<real name>\",\"lat\":<lat>,\"lng\":<lng>,
          \"services\":[\"vaccination\"],\"apiBaseUrl\":\"https://$DOM\",
          \"domain\":\"$DOM\",\"documentStores\":[\"$CLONE\"]}")
   BUSINESS_ID=$(echo "$REG" | jq -r .businessId)
   echo "$REG" | jq -r .hmacSecret      # SAVE THIS — returned ONCE (the per-business hmacSecret, §3)
   ```
   **Verify.** HTTP **200/201** (`curl -f` exits `0`) and `BUSINESS_ID` is non-empty; `hmacSecret`
   printed once — record it now (it is **not** re-shown).
3. **Business applies as an issuer** (public submission) → capture `applicationId`:
   ```bash
   APP_ID=$(curl -fsS -X POST "$CENTRAL/v1/issuer-applications" \
     -H 'content-type: application/json' \
     -d "{\"issuerEntityId\":\"<id>\",\"addresses\":[\"<signer addr>\"],
          \"recordTypes\":[\"VACCINATION\"],\"domain\":\"$DOM\",
          \"documentStore\":\"$CLONE\",\"license\":{ }}" | jq -r .applicationId)
   ```
   (Optional `usdaNan` is a 6-digit accreditation number; `license{number,jurisdiction,expiry}` if present.)
   **Verify.** HTTP **200/201** and `[ -n "$APP_ID" ] && [ "$APP_ID" != null ]` — a non-empty
   `applicationId` to chain into approve (step 5).
4. **Publish the issuer DNS TXT** on `<DOMAIN>`:
   ```
   dogtag-verify=<lowercased documentStore address>
   ```
   Replace: `<DOMAIN>` = the issuer's domain (the same `domain` you submitted). The address is
   **lowercased**; the prefix is the literal `dogtag-verify=`; the checker (Cloudflare DoH,
   `accept: application/dns-json`) matches a TXT record whose value **contains** that token. See §4 and
   [`stacks/admin/api/src/dns.rs`](../stacks/admin/api/src/dns.rs) (`expected_txt`).
   For example, a business whose clone is `0x5c70…cDb53` publishes
   `dogtag-verify=0x5c703910111f942ee0f47e02214291b5274cdb53`.

   **Verify.** The TXT resolves with the lowercased clone before you approve:
   ```bash
   dig +short TXT "$DOM" | grep -i "$(echo "$CLONE" | tr 'A-F' 'a-f')"   # must print the dogtag-verify= record
   ```
5. **Central approves** (admin/loopback) — runs the **DoH DNS check, then on-chain `whitelistFor`**.
   Reuse `$TOKEN` and `$APP_ID` from above:
   ```bash
   curl -fsS -X POST "$CENTRAL/v1/issuer-applications/$APP_ID/approve" \
     -H "authorization: Bearer $TOKEN"
   ```
   Returns `{ "status":"approved", "whitelistTxs":[...] }`.

   **Verify.** Response `status` is `approved` and `whitelistTxs` is non-empty:
   ```bash
   curl -fsS -X POST "$CENTRAL/v1/issuer-applications/$APP_ID/approve" \
     -H "authorization: Bearer $TOKEN" \
     | jq -e '.status=="approved" and (.whitelistTxs|length>0)'
   ```

   **STOP if** approve returns `403 DNS TXT verification failed`:
   - **Symptom:** approve fails before any on-chain tx.
   - **Cause:** the `dogtag-verify=` TXT is missing, not yet propagated, or doesn't contain the
     **lowercased** clone address.
   - **Fix:** publish/correct the TXT (step 4), wait for propagation, re-approve.
6. **Business custody genesis + unlock** (§6), operator login + backend signing mode, then
   **prepare → `issue(root)`** anchors the Merkle root on the business's clone, and **share** returns a
   one-time `/r/:token` URL for the QR.

### Groomer / verifier — EXPORT DNS legitimacy (phone-side)

The **EXPORT** flow (owner → groomer, §8) is symmetric: when the phone scans the groomer's EXPORT QR
(`https://<host>/x/<token>?a=<groomerAddr>`), the **phone** (not central) DNS-verifies the groomer
**before** generating or disclosing any proof. The groomer's `<host>` domain MUST publish a TXT that
binds the host to the **groomer's relayer wallet address** — the **same format** as the issuer record:

```
dogtag-verify=<lowercased GROOMER RELAYER address>
```

Replace: `<GROOMER_RELAYER>` = the groomer's **relayer wallet** address (the address embedded as `?a=`
in the EXPORT QR). For example, a groomer whose relayer is `0x<GROOMER_RELAYER>` publishes on its `<host>`
domain `dogtag-verify=0x<groomer_relayer_lowercased>`.

> **Note — do NOT use a contract address here.** The relayer is a **wallet (EOA)**, not a registry
> contract. In particular it is **not** the `ConsentKeyRegistry` (`0xA74DDe4a…`); using a registry
> address as the "relayer" is wrong. Use the groomer's actual relayer wallet address.

The phone resolves the QR host's domain via Cloudflare DoH and requires a TXT **containing**
`dogtag-verify=<groomerAddr>`; if it's absent, the app **hard-stops and discloses nothing**. This is
enforced for **real domains** (remote/prod) and **skipped for local hosts** (IP literal / `localhost` /
`*.local` / LAN) — the LOCAL demo (`DNS_CHECK=skip`,
[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)). It mirrors the issuer DoH convention in
[`stacks/admin/api/src/dns.rs`](../stacks/admin/api/src/dns.rs).

---

## 8. Run the prover-service yourself (the gap)

`scripts/remote-up.sh` starts **STACKS = admin, vet, groomer only** — it stands up **NO
prover-service**. Most phones don't need one: **64-bit iOS and modern arm64 Android prove on-device**.
But **32-bit-only Android** (`Build.SUPPORTED_64_BIT_ABIS.isEmpty()`, `ScanScreen.kt`) **cannot** prove
on-device and **must** offload to a prover-service. If any of your users are on 32-bit Android, **you
run a prover yourself**. The prover **sees the witness** (the raw record + consent), so it is
**owner-trusted only** and **unauthenticated by design** — never expose it as a shared/public service.

The prover is the `vet-api` binary built with the **`prover` cargo feature** (a `POST /prove-verification`
endpoint). Note: the `prover` **cargo feature** is **orthogonal** to the `FEATURES=mongo` **docker
build-arg** — don't confuse them.

**Build.** Either build the binary directly, or bake a `FEATURES=prover` image:

```bash
# Build the prover binary (separate target dir so it doesn't clobber the normal build):
cargo build --release -p vet-api --features prover --target-dir target/prover
#   -> target/prover/release/vet-api
```

**Run.** Set **`CIRCUITS_BUILD_DIR`** to a directory holding **`verification_final.zkey`** +
**`verification.graph`** so the real **ArkProver** loads. If `CIRCUITS_BUILD_DIR` is unset, the binary
**silently loads `StubProver`**, which emits placeholder proofs that are **NOT chain-valid**. If it is
**set but the real prover fails to load** (missing/corrupt zkey or graph), the process is **fail-closed**
and **exits with a FATAL error** rather than degrading to `StubProver` — so a misconfigured prover-service
never silently ships forgeable proofs. Also pass the usual chain env (`ROAX_RPC` and the `*_ADDR`
contract addresses).

```bash
CIRCUITS_BUILD_DIR=/circuits/build \
ROAX_RPC=https://devrpc.roax.net \
CHAIN_ID=135 \
VERIFICATION_REGISTRY_ADDR=<current VerificationRegistry — see contracts/deployments/roax.json> \
PORT=41875 \
  target/prover/release/vet-api
#   mount /circuits/build with verification_final.zkey (~65 MB) + verification.graph (~3 MB)
```

> **`CIRCUITS_BUILD_DIR` IS the production proving path for 32-bit Android.** (An earlier version of
> this doc wrongly called it "not the production proving path" — that was about the e2e test oracle.
> With a live prover-service it is exactly how 32-bit phones obtain chain-valid proofs.)

**Expose it behind its own TLS host** (a separate domain with its own Caddy, or a tunnel) — do **not**
co-locate it on a business/admin domain. Then point the phone's **`prover_api`** override at it. The
host comes from the in-app `prover_api` setting (Android only), **not** from any QR. See
**[MOBILE_BUILD.md](./MOBILE_BUILD.md)** (the `prover_api` setting) and **[TUNNELING.md](./TUNNELING.md)**
(giving the prover a reachable HTTPS URL).

**Verify.** The endpoint answers (it's unauthenticated):

```bash
curl -fsS https://<PROVER_DOMAIN>/health     # {"status":"ok"}
```

**STOP if** 32-bit-Android proofs are rejected on-chain:
- **Symptom:** the groomer's verification reverts / `isValid` stays false for proofs from a 32-bit phone.
- **Cause:** the prover loaded **`StubProver`** (placeholder proofs) because `CIRCUITS_BUILD_DIR` was
  **unset**. (If it was **set** but the zkey/graph were missing/corrupt, the prover-service would have
  **refused to boot** — see the FATAL log — rather than degrade to `StubProver`.)
- **Fix:** set `CIRCUITS_BUILD_DIR` to a dir containing `verification_final.zkey` + `verification.graph`
  and restart the prover.

---

## 9. Phones against REMOTE

Phones get the vet/groomer **hosts from the scanned QR**, not from a baked URL: `remote-up.sh` /
compose set each business's **`DEPLOYMENT_URL=https://<DOMAIN>`**, which becomes the host embedded in the
`/p/<token>` (issue) and `/x/<token>` (export) QR codes the phone scans. The device only ever calls the
**scanned host**.

Because REMOTE stays on **ROAX testnet with the same contract addresses**, **no app rebuild is needed**
to point phones at a REMOTE deployment — the bundled `roax.json` (addresses + chainId) is unchanged. You
only rebuild the apps when you **change chains/addresses** (Tier 3) or set a new baked default. The one
manual per-device setting that is **not** in any QR is `prover_api` (32-bit Android only, §8). Full
build + install + endpoint model: **[MOBILE_BUILD.md](./MOBILE_BUILD.md)**.

---

## 10. Backups

Custody lives in **Mongo** (a `CustodyBlob`), so backing up the Mongo data volume backs up the signer.
Back these up **before go-live** and on a schedule:

| Stack | Mongo data volume (back this up) |
|---|---|
| admin / central | `admindata` |
| vet | `vetdata` |
| groomer | `groomerdata` |

The legacy seed volumes (`adminseed` / `vetseed` / `groomerseed`) are **dead code** — backing them up
does **nothing** for custody. Example dump of one stack's Mongo volume:

```bash
# Dump the vet stack's Mongo to a host directory (run on the host).
docker compose -f stacks/vet/docker-compose.yml exec -T mongo \
  mongodump --port 27017 --archive > vetdata-$(date +%F).archive
```

> Losing the Mongo volume **and** the passphrase = unrecoverable custody (§6). Back up the volume; store
> the passphrase separately.

---

## 11. Going to PRODUCTION

REMOTE stays on **ROAX testnet** with the **single-operator testnet ZK key** — fine for testnet, **NOT**
for a real deployment. Going live (a different / production chain, a **multi-party trusted-setup
ceremony**, the verifier wired via the registry's **2-day timelock**, edge hardening, and rebuilding the
mobile apps for the new addresses) is **Tier 3**:

➡ **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — the go-live delta over REMOTE (chain-swap
checklist, ceremony + timelock runbook). The ceremony itself is **[CEREMONY.md](./CEREMONY.md)**.

---

## 12. Troubleshooting (REMOTE subset)

| Symptom | Likely cause | Fix |
|---|---|---|
| `remote-up.sh` aborts: `… is still a placeholder` / `… must be set` | unfilled secret in a `.env` | generate with `openssl rand -hex 32`, set it, re-run (§3) |
| `remote-up.sh` aborts: `VITE_DEMO_MODE is set … must be UNSET` | demo flag left in a stack `.env` | remove/unset `VITE_DEMO_MODE`, rebuild (§3, §5) |
| `curl https://<DOMAIN>/health` → TLS / cert error | Caddy hasn't issued the cert (DNS or port 80 not ready) | fix the A record + open port 80 (§4); `… logs -f caddy` for the ACME error |
| api container keeps restarting | `MONGO_URI` set but Mongo unreachable (fail-closed) | `… logs -f mongo` / `… logs -f api`; confirm `mongodb://mongo:27017/dogtag` (§3, §5) |
| `/v1/issuer-applications/<id>/approve` → `403 DNS TXT verification failed` | `dogtag-verify=` TXT missing / not propagated / not lowercased | publish/correct the issuer TXT, wait, re-approve (§4, §7) |
| phone hard-stops on EXPORT, discloses nothing | groomer host's `dogtag-verify=<relayer>` TXT missing/wrong (used a contract addr) | publish the TXT with the **lowercased groomer RELAYER wallet** address (§7) |
| `/admin/*` route returns 403 from the internet | `ADMIN_LOOPBACK_ONLY=1` + Caddy edge-deny (by design) | run admin actions from the host or an allowlisted CIDR (§6) |
| 32-bit-Android proofs rejected on-chain | prover ran `StubProver` (no `CIRCUITS_BUILD_DIR`) | set `CIRCUITS_BUILD_DIR` to a dir with the zkey + graph; restart the prover (§8) |
| `429` on login/unlock | rate-limit lockout | wait out the lockout window; retry (§6) |

---

## See also

- **[DEPLOYMENT.md](./DEPLOYMENT.md)** — index, tier decision-guide, the canonical **Address Book** +
  service/port tables.
- **[PREREQUISITES.md](./PREREQUISITES.md)** — install matrix (macOS + Linux), per-tool "needed by".
- **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)** — Tier 1 demo runbook (`VITE_DEMO_MODE=1`, MemStore).
- **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — Tier 3 go-live delta (chain swap, ceremony, timelock).
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** — build/install iOS & Android, endpoint model, `prover_api`.
- **[TUNNELING.md](./TUNNELING.md)** — public HTTPS for phones + the prover's own TLS host.
- **[DEPLOY.md](./DEPLOY.md)** — ROAX contract deploy + Docker bring-up runbook.
- **[CEREMONY.md](./CEREMONY.md)** — production ZK trusted-setup ceremony.
- **[DPIA.md](./DPIA.md)** — Data Protection Impact Assessment.
- **[`deploy/Caddyfile`](../deploy/Caddyfile)** · **[`scripts/remote-up.sh`](../scripts/remote-up.sh)** — TLS proxy + production bring-up.
