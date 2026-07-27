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
- **Real DNS legitimacy.** Issuer (and EXPORT groomer) legitimacy is observed via Cloudflare
  **DNS-over-HTTPS** against a `dogtag-verify=` TXT record (§4, §7). The lookup is always a REAL
  resolution — there is no bypass switch — and for issuer approval it is **advisory**: it never blocks
  whitelisting, but a non-verified observation requires the admin's explicit `proceedWithoutDns` and is
  persisted on the application. See [ISSUER_DOMAIN_BINDING.md](./ISSUER_DOMAIN_BINDING.md).
- **Manual / no autofill.** `VITE_DEMO_MODE` is **unset** — no prefilled forms, no demo buttons, no
  stashed seed. Operators type passwords and re-type the genesis challenge words by hand.

REMOTE **is NOT**:

- **A production chain.** It still runs on **ROAX testnet (135)** with the **same addresses**. Moving
  to another / production chain is Tier 3 → **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)**.
- **A complete proving setup by default.** `scripts/remote-up.sh` starts **no prover-service**.
  Phones prove on-device; the optional `/prove-consent` server-prove fallback is a service **you run
  yourself** - see **§8**.

### LOCAL vs REMOTE at a glance

The single switch is **`VITE_DEMO_MODE`** (portal build-time flag): set = demo, **unset = production**.

| Aspect | LOCAL (`scripts/demo-up.sh`) | REMOTE (`scripts/remote-up.sh` / compose) |
|---|---|---|
| Form entry | Auto-filled, demo buttons shown (`VITE_DEMO_MODE=1`) | **Empty, no demo buttons** (flag unset) |
| Operator/admin passwords | Demo prefilled (`operator` / `admin`) | Strong, env-set, **typed** |
| Genesis seed | Stashed + auto-filled into confirm | Operator **reads + re-types** challenge words |
| Storage | `MemStore` (records/sessions ephemeral; **restart = re-unlock** — custody seal in `.demo/*-custody.json`; records/sessions re-created) | `MongoStore` (persistent; back up the volume) |
| Networking | LAN IP or cloudflared tunnel | Real domain + **TLS** (Caddy auto-HTTPS) |
| DNS legitimacy | real DoH lookup either way — `.local` simply never resolves, so approve is confirmed with `proceedWithoutDns`; phone groomer-DNS skipped for local hosts | real `dogtag-verify=` TXT resolves (issuer **and** EXPORT groomer, §4) |
| `/admin/*` exposure | On the main listener (single host) | **Loopback-isolated** + proxy-denied publicly |
| Confirmations | `CONFIRMATIONS=1` | `CONFIRMATIONS=2` |
| Prover-service | Started on `:41875` (consent server-prove fallback) | **Not started** - run it yourself (§8) |
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
run the consent server-prove fallback** (§8).

---

## 2. Topology

- **Per business (vet / groomer): self-hosted.** Each business runs its own
  `stacks/<vet|groomer>` stack — `web` (nginx serving the built SPA) + `api` (the **`vet-api`** binary;
  the groomer is the **same binary** run with `BUSINESS_TYPE=groomer`) + **its own Mongo** + **Caddy**
  (TLS). Custody (the issuer signer) lives in that business's Mongo and never leaves the box.
- **One central / admin stack (you host):** `stacks/admin` — registry/discovery, issuer whitelisting,
  mobile API, appointment source-of-truth, erasure. It holds the **admin protocol signer** that
  broadcasts `whitelistFor` and administers issuer roles + factory governance (it does not issue tags).
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
| `ISSUER_NAME` | vet, groomer | display name | "Example Veterinary Clinic" / "Example Grooming Salon" | real name |
| `ISSUER_DOMAIN` | vet, groomer | **the real DNS-TXT issuer-domain binding** | `*.local` | your real domain |
| `ISSUER_REGISTRY_ADDR` | all | IssuerRegistry | (roax.json, pre-filled) | per chain |
| `VERIFICATION_REGISTRY_CONSENT_ADDR` | vet, groomer | `VerificationRegistryConsent` - the owner-hidden relay target for `POST /v1/verify/consent`. The relayer (custody account 0) must be whitelisted for the purpose it submits. Unset or malformed → only that route fails closed with **503** | (roax.json, pre-filled) | per chain; **never** a retired/`_legacy` generation |
| `SBT_CONSENT_ADDR` | vet | `DogTagSBTConsent` - the owner-blind `mintCustodial` target for `POST /profiles/issue/custodial-bind`, the **sole profile issuance path**. The signer must hold `ISSUER_ROLE` on it. Required **as a pair** with `PROFILE_ISSUER_ADDR` | (roax.json) pre-filled in vet; commented out in groomer (inert there - `BUSINESS_TYPE=groomer` does not mount the issuance routes at all) | per chain |
| `PROFILE_ISSUER_ADDR` | vet | a real **factory-deployed** `DogTagIssuer` clone that profile roots are anchored into via `issue(R)`, so `rootIssuer[R]` resolves and the tag is revocable. **Never the SBT** - `issue(R)` sent to the SBT reverts. The signer must be whitelisted for the clone's recordType | unset | a real factory-deployed clone, never the SBT |
| `VERIFICATION_REGISTRY_ADDR` | admin | the **same** owner-hidden `VerificationRegistryConsent`, read by admin as the protocol default stamped into unstamped credential imports (provenance routing), not as a relay target | (roax.json, pre-filled) | per chain |
| `SBT_ADDR` | admin | `DogTagSBTConsent` as the admin governance target (`ISSUER_ROLE` administration only; the admin stack does not issue tags) | (roax.json, pre-filled) | per chain |
| `FACTORY_ADDR` | admin | DogTagIssuerFactory — `createIssuer`/`predictIssuer` target + the Ownable owner whose key gates deploys | (roax.json, pre-filled) | per chain |
| `VACCINATION_ISSUER_ADDR` | vet | per-recordType clone (inert on a groomer - that role mounts no issuance routes) | `0x0…0` (set to the real clone for an issuer) | `0x0…0` for pure verifiers |
| `ADMIN_SIGNER_INDEX` | admin | HD signer index | `0` | `0` |
| `ADMIN_PROPOSE_ONLY` (alias `ALLOW_UNAUTHORIZED_ADMIN_SIGNER`) | admin | **declares** that privileged writes are signed out-of-band, so a grant/revoke that broadcasts nothing is the intended outcome (`outcome:"proposed_by_design"`) rather than the wrong-key one (`proposed_unauthorized`). Reporting only — it never changes what is dispatched, and holdership is always read live from the chain | unset (`demo-up.sh` forwards it, and refuses to boot without it when the signer lacks `WHITELIST_ADMIN`) | set to `1` **only** when the hosted key is deliberately not the authority holder (Safe / offline governance signer) |
| `ADMIN_REQUIRE_AUTHORITY` | admin | turns the boot authority check from a logged ERROR into a refusal to boot when the hosted signer holds **none** of the three control-plane authorities. Best-effort: the check is time-boxed, and an unreadable chain leaves the verdict `Unknown`, which never refuses | unset | `1` for a deployment whose hosted key is meant to execute (leave unset for a propose-only one) |
| `CONFIRMATIONS` | all | reorg safety | `1` | **`2`** (enforced) |
| `ADMIN_LOOPBACK_ONLY` | all | bind `/admin/*` to `127.0.0.1:ADMIN_PORT` | unset | **`1`** (enforced) |
| `CORS_ALLOW_ORIGINS` | all | CORS allowlist | unset (permissive) | `https://<DOMAIN>` |
| `OPERATOR_PASSWORD` | vet, groomer | operator login (`POST /login`) | `operator` | **secret** → `openssl rand -hex 32` |
| `ADMIN_PASSWORD` | all | admin-session login (custody/console) | `admin` | **secret** → `openssl rand -hex 32` |
| `CENTRAL_HMAC_SECRET` | all | central↔business HMAC; **identical across all stacks** | `dev-central-hmac-secret` | **secret** → `openssl rand -hex 32` (same value everywhere) |
| `ADMIN_PRIVATE_KEY` | admin | on-chain signer (`whitelistFor` / issuer-role administration / factory `createIssuer`); its holdership of each authority is what the `GovernanceAction` dispatcher checks. Since Governance Phase-2 (2026-07-05, block 123835) this MUST be **governance signer-1**. The old deployer EOA `0x119F…` lost governance/admin authority but retains the retired owner-revealing SBT's issuer/whitelist capabilities and is not a neutral key | from `contracts/.env` (`GOVERNANCE_PRIVATE_KEY`) | **secret** - dedicated **funded** signer-1 key |
| `ADMIN_ADDRESS` | admin | address of `ADMIN_PRIVATE_KEY` - expected governance signer-1 `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` | from `contracts/.env` | derive from the key |
| `BUSINESS_ID` | vet, groomer | central registry id | `biz-vet-local` / `biz-groomer-local` | real id |
| `BUSINESS_TYPE` | groomer | run `vet-api` as groomer | `groomer` | `groomer` |
| `CENTRAL_BASE_URL` | vet, groomer | central api base for HMAC events | `http://localhost:39742` | `https://api.<DOMAIN>` (your admin stack) |
| `INDEXER_API_BASE` + `INDEXER_SCOPED_TOKEN` (business) / `INDEXER_OVERSIGHT_TOKEN` (admin) | all | optional oversight-indexer wiring (`stacks/indexer`). Unset → the business `/trace/*` / admin `/v1/admin/activity*` surfaces return **503** while the rest of the stack runs | unset | optional |
| `CIRCUITS_BUILD_DIR` | **prover only** | directory holding the consent proving artifacts (`consent_final.zkey`, `consent.r1cs`, `consent_js/consent.wasm`) so `POST /prove-consent` can prove. Unset → the route fails closed per request | `circuits/build` | set **only** on the prover-service (§8) |
| `CONSENT_EXPECTED_ZKEY_SHA256` | **prover only** | override of the pinned consent-zkey sha256 | unset (enforce the pinned testnet hash) | leave **unset** with the bundled testnet zkey; set to the ceremony zkey's sha256 only if you ship a different key (§8) |
| `DOGTAG_MANIFEST_SIGNING_KEY` | vet | signed-manifest fallback (M7 §5.1): a 32-byte ed25519 seed (64 hex). When set, serves `GET /protocol/manifest?version=<v>`, the dogtag-signed discovery manifest an app verifies OFFLINE (a cache/fallback for the on-chain `ProtocolRegistry`; on conflict on-chain wins) | unset (route → 503) | optional; leave unset to disable. If enabled it is a **secret** (the ed25519 seed whose paired public key apps pin); NEVER commit a real value |
| `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_CALENDAR_ID` | vet, groomer | Phase-7 calendar OAuth | unset / `primary` | optional |

> **`SBT_CONSENT_ADDR` and `PROFILE_ISSUER_ADDR` are required together** - both are needed for
> `POST /profiles/issue/custodial-bind` (owner-hidden custodial issuance, the sole profile issuance path).
> If either is unset or malformed, that one route fails closed with **503** *before* it consumes the
> one-time bind token, so a half-wired stack never burns an operator's QR code.
> Everything else in the stack is unaffected.
> Worth knowing when diagnosing: a 503 on only that route means exactly this pair, and the addresses
> are shape-checked, so a typo is refused rather than silently coerced to `0x0…0`.

> **`VERIFICATION_REGISTRY_CONSENT_ADDR` is INDEPENDENT of that pair** - it wires the owner-hidden
> *verify* path (`POST /v1/verify/consent`), not issuance.
> Unset or malformed → only that one route fails closed with **503**, checked *before* custody or the
> chain is touched.
> Same shape-check discipline: a typo is refused, never coerced to `0x0…0` (a tx to a codeless address
> would otherwise SUCCEED and the mistake would only surface after the gas was spent).

> **The admin stack has no** `OPERATOR_PASSWORD`, `VACCINATION_ISSUER_ADDR`, `ISSUER_NAME` /
> `ISSUER_DOMAIN`, `BUSINESS_TYPE`, `CENTRAL_BASE_URL`, or `DEPLOYMENT_DOMAIN` - it is the central
> stack, not a business issuer. (It **does** read `VERIFICATION_REGISTRY_ADDR` and `SBT_ADDR` - both
> pointing at the owner-hidden pair - as its provenance-routing and governance targets.)

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
| `VITE_ISSUER_REGISTRY_ADDR` | IssuerRegistry (whitelist reads) | (roax.json, pre-filled) | per chain |

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
  **`CONFIRMATIONS=2`**, **`ADMIN_LOOPBACK_ONLY=1`**.
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
> enforcement (`CONFIRMATIONS=2` / `ADMIN_LOOPBACK_ONLY=1`). (Compose hardcodes the
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
through the TLS domain `https://<DOMAIN>/`). Setup owns **genesis** (steps 1-4); **re-unlocking**
after a restart (step 5) is a dedicated page, not a wizard step:

1. **Genesis** a new **24-word BIP-39** seed. The words are shown **once** — **WRITE THEM DOWN**. There
   is **no autofill** in production (`VITE_DEMO_MODE` unset) and the seed is never stashed.
2. **Re-type the challenge words** to confirm (you key them in manually).
3. Set a **strong passphrase**. The seed is scrypt/age-encrypted under it and stored as a
   **`CustodyBlob` in Mongo**.
4. **Unlock** with that passphrase to wire the signer into the chain client.
5. **Re-unlock after EVERY api restart** — custody is **not** auto-unlocked.
   Records and the encrypted seed survive the restart, but the signer cannot sign until you `POST /admin/unlock` again.
   You do **not** hunt for this in Setup.
   On this (Mongo) path the operator session outlives the restart while the seal re-locks, so the lock usually surfaces on the **first action** that trips it: the portal raises an **unlock prompt in place**, over the page you are already on, and **replays the refused request** once the seal opens - so a half-filled form is never discarded and nothing navigates.
   Arriving at an already-locked backend instead shows a **non-blocking banner** with an Unlock button; read-only pages (records, traceability, verification history) stay reachable, because the operator password and the custody-admin password are **separate credentials** and front-desk staff must not be shut out by a lock they cannot clear.
   The dedicated **`/unlock`** page remains as the fallback surface and as a direct link, restoring `?next=` when it carries one.
   Either surface asks for the custody-admin password and the passphrase; a wrong passphrase is an inline error, not a lost session.
   Both point at Setup **only** if the instance has no seal at all - that needs genesis (steps 1-4), not a passphrase.

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
   For example, a business whose clone is `0x1456…edD9A` publishes
   `dogtag-verify=0x1456f93f7376789c46408cc4616751eb853edd9a`.

   **Verify.** The TXT resolves with the lowercased clone before you approve:
   ```bash
   dig +short TXT "$DOM" | grep -i "$(echo "$CLONE" | tr 'A-F' 'a-f')"   # must print the dogtag-verify= record
   ```
5. **Central approves** (admin/loopback) — runs the **DoH DNS observation, then on-chain
   `whitelistFor`**. Reuse `$TOKEN` and `$APP_ID` from above:
   ```bash
   curl -fsS -X POST "$CENTRAL/v1/issuer-applications/$APP_ID/approve" \
     -H "authorization: Bearer $TOKEN" \
     -H 'content-type: application/json' -d '{}'
   ```
   Returns `{ "status":"approved", "whitelistTxs":[...], "dnsState":"verified", … }`.

   **Verify.** Response `status` is `approved` and `whitelistTxs` is non-empty:
   ```bash
   curl -fsS -X POST "$CENTRAL/v1/issuer-applications/$APP_ID/approve" \
     -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{}' \
     | jq -e '.status=="approved" and (.whitelistTxs|length>0)'
   ```

   **If** approve returns `409 {"error":"dnsConfirmationRequired", …}`:
   - **What it means:** the DNS lookup did not come back `verified` — the `dogtag-verify=` TXT is
     missing, not yet propagated, or doesn't contain the **lowercased** clone address. Nothing is
     on-chain yet. This is a decision to make, not a failure: the check is ADVISORY and says nothing
     about the organisation, whose legitimacy is the accreditation review's business.
   - **Preferred:** publish/correct the TXT (step 4), wait for propagation, re-approve — the 409 body
     carries the exact `expectedTxt` value and the observed `dnsState`.
   - **Or proceed deliberately** (routine when an org is KYC-approved before its DNS team publishes).
     Both the observation and the fact that you proceeded are PERSISTED on the application
     (`dnsStateAtApproval` / `dnsProceededUnverified`), so it never reads as a clean pass:
     ```bash
     curl -fsS -X POST "$CENTRAL/v1/issuer-applications/$APP_ID/approve" \
       -H "authorization: Bearer $TOKEN" \
       -H 'content-type: application/json' -d '{"proceedWithoutDns":true}'
     ```
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

> **Note - do NOT use a contract address here.** The relayer is a **wallet (EOA)**, not a contract.
> Publishing a registry/contract address as the "relayer" is wrong; use the groomer's actual relayer
> wallet address.

The phone resolves the QR host's domain via Cloudflare DoH and requires a TXT **containing**
`dogtag-verify=<groomerAddr>`; if it's absent, the app **hard-stops and discloses nothing**. This is
enforced for **real domains** (remote/prod) and **skipped for local hosts** (IP literal / `localhost` /
`*.local` / LAN) — see [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md). It mirrors the issuer DoH
convention in [`stacks/admin/api/src/dns.rs`](../stacks/admin/api/src/dns.rs). Note this phone-side
groomer gate is a HARD stop, unlike the advisory issuer-approval gate above; the two are separate.

---

## 8. Run the prover-service yourself (the gap)

`scripts/remote-up.sh` starts **STACKS = admin, vet, groomer only** — it stands up **NO
prover-service**.
Phones prove **on-device**: the app needs the consent proving pair `consent_final.zkey` +
`consent.graph` bundled (see **[MOBILE_BUILD.md](./MOBILE_BUILD.md)**).
The backend `POST /prove-consent` route is the **independent server-prove fallback** for devices that
cannot prove on-device (e.g. 32-bit-only Android): the owner's device assembles the circuit input
locally and offloads only the heavy Groth16 proving.
The route is live server-side; the device-side offload wiring lands in a later slice, so today the
fallback is exercised against the API directly.
The prover **sees the witness** (the request carries `ownerSecret` / `ownerAddress`), so it is
**owner-trusted only** and **unauthenticated by design** — never expose it as a shared/public service.
It still hides the owner from the verifier, relayer, chain, and the emitted event.

The prover is the `vet-api` binary built with the **`prover` cargo feature** (which mounts
`POST /prove-consent`).
The stack images build with the `FEATURES=mongo` build-arg (a cargo feature list that does **not**
include `prover`), so a normal stack api never serves the route.

**Build.**

```bash
# Build the prover binary (separate target dir so it doesn't clobber the normal build):
cargo build --release -p vet-api --features prover --target-dir target/prover
#   -> target/prover/release/vet-api
```

**Run.** Set **`CIRCUITS_BUILD_DIR`** to a directory holding the consent proving artifacts -
**`consent_final.zkey`**, **`consent.r1cs`**, **`consent_js/consent.wasm`** (the committed
`circuits/build` has all three).
Loading is lazy (deferred to the first `/prove-consent` request) and **fail-closed per request**: if
the directory is unset, an artifact is missing/corrupt, or the zkey's sha256 does not match the pinned
hash, the route returns an error instead of a proof - there is no stub or placeholder fallback, so a
misconfigured prover-service can never ship a bogus proof.
`CONSENT_EXPECTED_ZKEY_SHA256` overrides the pinned hash - leave it **unset** with the bundled testnet
zkey; set it only if you ship a different proving key (e.g. a production ceremony output).
Also pass the usual chain env (`ROAX_RPC` and the `*_ADDR` contract addresses).

```bash
CIRCUITS_BUILD_DIR=/circuits/build \
ROAX_RPC=https://devrpc.roax.net \
CHAIN_ID=135 \
PORT=41875 \
  target/prover/release/vet-api
#   mount /circuits/build with consent_final.zkey + consent.r1cs + consent_js/consent.wasm
```

**Expose it behind its own TLS host** (a separate domain with its own Caddy, or a tunnel) — do **not**
co-locate it on a business/admin domain. See **[TUNNELING.md](./TUNNELING.md)** (giving the prover a
reachable HTTPS URL) and **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** (the app's endpoint model).

**Verify.** The service answers (the prover route itself is unauthenticated):

```bash
curl -fsS https://<PROVER_DOMAIN>/health     # {"status":"ok"}
```

**STOP if** `/prove-consent` answers with a prover-unavailable error:
- **Symptom:** the route returns an error naming the prover as unavailable, instead of a proof.
- **Cause:** `CIRCUITS_BUILD_DIR` is unset, an artifact is missing/corrupt, or the zkey's sha256 does
  not match the pin (the load error is cached until restart).
- **Fix:** point `CIRCUITS_BUILD_DIR` at a dir containing `consent_final.zkey` + `consent.r1cs` +
  `consent_js/consent.wasm` (set `CONSENT_EXPECTED_ZKEY_SHA256` only for a non-bundled key), then
  restart the prover.

---

## 9. Phones against REMOTE

Phones get the vet/groomer **hosts from the scanned QR**, not from a baked URL: `remote-up.sh` /
compose set each business's **`DEPLOYMENT_URL=https://<DOMAIN>`**, which becomes the host embedded in the
`/p/<token>` (issue) and `/x/<token>` (export) QR codes the phone scans. The device only ever calls the
**scanned host**.

Because REMOTE stays on **ROAX testnet with the same contract addresses**, **no app rebuild is needed**
to point phones at a REMOTE deployment — the bundled `roax.json` (addresses + chainId) is unchanged. You
only rebuild the apps when you **change chains/addresses** (Tier 3) or set a new baked default. Full
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

REMOTE stays on **ROAX testnet** with the **single-operator testnet consent-ceremony key** - fine for
testnet, **NOT** for a real deployment. Going live (a different / production chain, a **multi-party
trusted-setup ceremony**, the verifier wired via the registry's **2-day timelock**, edge hardening, and
rebuilding the mobile apps for the new addresses) is **Tier 3**:

➡ **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — the go-live delta over REMOTE (chain-swap
checklist, ceremony + timelock runbook). The ceremony itself is **[CEREMONY.md](./CEREMONY.md)** (the
live consent-ceremony guide; **[CEREMONY_RUNBOOK.md](./CEREMONY_RUNBOOK.md)** is the expanded
captain-fill-in runbook).

---

## 12. Troubleshooting (REMOTE subset)

| Symptom | Likely cause | Fix |
|---|---|---|
| `remote-up.sh` aborts: `… is still a placeholder` / `… must be set` | unfilled secret in a `.env` | generate with `openssl rand -hex 32`, set it, re-run (§3) |
| `remote-up.sh` aborts: `VITE_DEMO_MODE is set … must be UNSET` | demo flag left in a stack `.env` | remove/unset `VITE_DEMO_MODE`, rebuild (§3, §5) |
| `curl https://<DOMAIN>/health` → TLS / cert error | Caddy hasn't issued the cert (DNS or port 80 not ready) | fix the A record + open port 80 (§4); `… logs -f caddy` for the ACME error |
| api container keeps restarting | `MONGO_URI` set but Mongo unreachable (fail-closed) | `… logs -f mongo` / `… logs -f api`; confirm `mongodb://mongo:27017/dogtag` (§3, §5) |
| `/v1/issuer-applications/<id>/approve` → `409 dnsConfirmationRequired` | `dogtag-verify=` TXT missing / not propagated / not lowercased — the check is advisory, so nothing is on-chain yet | publish/correct the issuer TXT, wait, re-approve; or re-send `{"proceedWithoutDns":true}` to whitelist anyway (recorded on the application) (§4, §7) |
| phone hard-stops on EXPORT, discloses nothing | groomer host's `dogtag-verify=<relayer>` TXT missing/wrong (used a contract addr) | publish the TXT with the **lowercased groomer RELAYER wallet** address (§7) |
| `/admin/*` route returns 403 from the internet | `ADMIN_LOOPBACK_ONLY=1` + Caddy edge-deny (by design) | run admin actions from the host or an allowlisted CIDR (§6) |
| `/prove-consent` returns a prover-unavailable error | `CIRCUITS_BUILD_DIR` unset, artifacts missing/corrupt, or consent-zkey hash mismatch (fail-closed per request) | point `CIRCUITS_BUILD_DIR` at `consent_final.zkey` + `consent.r1cs` + `consent_js/consent.wasm`; restart the prover (§8) |
| `429` on login/unlock | rate-limit lockout | wait out the lockout window; retry (§6) |

---

## See also

- **[DEPLOYMENT.md](./DEPLOYMENT.md)** — index, tier decision-guide, the canonical **Address Book** +
  service/port tables.
- **[PREREQUISITES.md](./PREREQUISITES.md)** — install matrix (macOS + Linux), per-tool "needed by".
- **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)** — Tier 1 demo runbook (`VITE_DEMO_MODE=1`, MemStore).
- **[PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md)** — Tier 3 go-live delta (chain swap, ceremony, timelock).
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** - build/install iOS & Android, endpoint model.
- **[TUNNELING.md](./TUNNELING.md)** — public HTTPS for phones + the prover's own TLS host.
- **[DEPLOY.md](./DEPLOY.md)** — ROAX contract deploy + Docker bring-up runbook.
- **[CEREMONY.md](./CEREMONY.md)** - the live consent trusted-setup ceremony guide
  (**[CEREMONY_RUNBOOK.md](./CEREMONY_RUNBOOK.md)** is the expanded captain-fill-in runbook).
- **[DPIA.md](./DPIA.md)** — Data Protection Impact Assessment.
- **[`deploy/Caddyfile`](../deploy/Caddyfile)** · **[`scripts/remote-up.sh`](../scripts/remote-up.sh)** — TLS proxy + production bring-up.
