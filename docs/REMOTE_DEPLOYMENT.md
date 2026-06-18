# DogTag — REMOTE deployment (hardened, self-hosted, production-shaped)

The realistic deployment guide: **self-hosted per business** + one central/admin stack, **persistent
Mongo**, real domain + **TLS**, real **DNS-TXT** issuer legitimacy, strong secrets, per-business
custody, and **operators key in everything** (no demo autofill). It still runs on the **same live ROAX
testnet** (chainId **135**, the same addresses in
[`../contracts/deployments/roax.json`](../contracts/deployments/roax.json) — **no contract redeploy**).

Going from this REMOTE-on-testnet setup to a different / production chain is then a **pure config
swap** — see [§9 Switching chains](#9-switching-to-a-different--production-chain--config-only). No code edits.

> The demo runbook is **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)**; the literal click-through is
> **[DEMO_CLICKS.md](./DEMO_CLICKS.md)**. The contract deploy + ceremony runbook is **[DEPLOY.md](./DEPLOY.md)**.

---

## LOCAL vs REMOTE at a glance

The single switch is **`VITE_DEMO_MODE`** (portal build-time flag): set = demo, **unset = production**.

| Aspect | LOCAL (`scripts/demo-up.sh`) | REMOTE (`scripts/remote-up.sh` / compose) |
|---|---|---|
| Form entry | Auto-filled, demo buttons shown (`VITE_DEMO_MODE=1`) | **Empty, no demo buttons** (flag unset) |
| Operator/admin passwords | Demo prefilled (`operator` / `admin`) | Strong, env-set, **typed** |
| Genesis seed | Stashed + auto-filled into confirm | Operator **reads + re-types** challenge words |
| Storage | `MemStore` (ephemeral; restart = re-genesis) | `MongoStore` (persistent; back up the volume) |
| Networking | LAN IP or cloudflared tunnel | Real domain + **TLS** (Caddy auto-HTTPS) |
| DNS legitimacy | `DNS_CHECK=skip` (`.local`) | `DNS_CHECK=doh` + real `dogtag-verify=` TXT |
| `/admin/*` exposure | On the main listener (single host) | **Loopback-isolated** + proxy-denied publicly |
| Confirmations | `CONFIRMATIONS=1` | `CONFIRMATIONS=2` |
| Chain | ROAX testnet (135) | **Same** ROAX testnet (135) |

---

## 1. Topology & scope

- **Per business (vet / groomer): self-hosted.** Each business runs its own `stacks/<vet|groomer>`
  stack — `web` (nginx serving the built SPA) + `api` (the **`vet-api`** binary; groomer sets
  `BUSINESS_TYPE=groomer`) + **its own Mongo** + **Caddy** (TLS). Custody (the issuer signer) lives in
  that business's Mongo and never leaves the box.
- **One central / admin stack (you host):** `stacks/admin` — registry/discovery, issuer whitelisting,
  mobile API, appointment source-of-truth, erasure. It holds the **admin protocol signer** that
  broadcasts `whitelistFor` / SBT `mint`.
- **Contracts are reused as-is.** All stacks point at the **live ROAX addresses** — no redeploy.
- **Per-business `documentStore` clones** are created **centrally** (the factory is `onlyOwner`) via
  `DogTagIssuerFactory.createIssuer(name, keccak256(recordType), businessAddr)`
  (factory `0xd3179AbBfb0274D0a5F7017d76015A93C159511D`). The resulting clone address is what the
  business puts in its issuer application **and** in its `dogtag-verify=` DNS TXT.

---

## 2. Build & bring-up

The api image must be built **with the `mongo` feature** (the Dockerfiles default `ARG FEATURES=mongo`;
a feature-less build falls back to MemStore). Compose passes `build.args.FEATURES=mongo`.

```bash
# 1. Per stack, copy the production env template and fill it in:
cp stacks/admin/.env.example   stacks/admin/.env
cp stacks/vet/.env.example     stacks/vet/.env
cp stacks/groomer/.env.example stacks/groomer/.env
#    -> set MONGO_URI, DOMAIN, ROAX_RPC, CHAIN_ID, the *_ADDR addresses, and strong secrets.
#       (Reject any leftover `change-me` placeholder; do NOT set VITE_DEMO_MODE.)

# 2. Build + bring up all three stacks (persistent Mongo + Caddy TLS):
scripts/remote-up.sh
```

`scripts/remote-up.sh` validates each stack `.env` (rejects `change-me` placeholders **and** any
`VITE_DEMO_MODE`), builds `--build-arg FEATURES=mongo`, brings up admin/vet/groomer with
`docker compose up -d`, and enforces `DNS_CHECK=doh`, `CONFIRMATIONS=2`, `ADMIN_LOOPBACK_ONLY=1`. It
**does not** automate genesis — it prints the manual custody + onboarding runbook (see §5–§7).

Per-stack manual control: `docker compose -f stacks/<x>/docker-compose.yml {build,up -d,logs -f,down}`,
or `make up-<x>`.

**TLS (Caddy).** Each stack's compose has a `caddy` service (ports **80/443**) parameterized by
[`deploy/Caddyfile`](../deploy/Caddyfile) via `{$DOMAIN}`. Caddy auto-issues a Let's Encrypt cert and
reverse-proxies to the **internal** `web` nginx (which serves `dist` and proxies `/api`). The `web`
service is internal-only (`expose: 80`, no host ports). **Before first boot:** a public DNS **A record**
for `$DOMAIN` → the host, and inbound **80** (ACME + redirect) and **443** open. Certs persist in the
`caddy_data` volume across restarts.

**Health.** Every api serves `GET /health` → `{"status":"ok"}` (no auth) — wired as the compose api
healthcheck; Mongo has its own healthcheck and api `depends_on: condition: service_healthy`.

---

## 3. Persistent storage (Mongo)

- Set **`MONGO_URI`** (compose default `mongodb://mongo:27017/dogtag`; internal port **27017**),
  **`MONGO_DB`** (default `dogtag`). Mongo is **`expose:`-only — never published** to the host.
- The api is **fail-closed**: if `MONGO_URI` is set but unreachable/bad, the api **refuses to boot**
  (it never silently falls back to MemStore). Unset/empty `MONGO_URI` = MemStore (demo only).
- **Custody persists in Mongo.** The issuer's age-encrypted seed is stored as a `CustodyBlob` in the
  store — *not* in the old `*seed` / `KEYSTORE_PATH` volume (that volume is dead code). **Back up the
  Mongo data volume** for each stack: `vetdata`, `admindata`, `groomerdata`.
- **Restart ≠ data loss, but custody is NOT auto-unlocked.** Records and the encrypted seed survive a
  restart, but after every api restart the operator must **re-`/admin/unlock`** with the passphrase
  before the signer can sign. See §5.

---

## 4. DNS-TXT issuer legitimacy (verbatim)

Before whitelisting an issuer, central verifies the business controls its domain by checking a DNS TXT
record via **Cloudflare DNS-over-HTTPS** (`DNS_CHECK=doh`). The exact record (from
[`stacks/admin/api/src/dns.rs`](../stacks/admin/api/src/dns.rs), `expected_txt`):

```
dogtag-verify=<lowercased documentStore address>
```

For example, a business whose clone is `0x5c703910111f942EE0f47E02214291b5274cDb53` publishes:

```
dogtag-verify=0x5c703910111f942ee0f47e02214291b5274cdb53
```

The address is **lowercased**, the prefix is the literal string `dogtag-verify=`, and the checker
matches a TXT record whose value **contains** that token (Cloudflare DoH, `accept: application/dns-json`).
The check runs at **approve time, before** the on-chain `whitelistFor`. Publish the TXT on the issuer's
`domain` (the same `domain` you submit in the issuer application) before central approves.

---

## 5. Custody runbook (manual — no autofill)

Per **business stack (vet, groomer)** and for the **admin signer**, on the portal Setup wizard
(reached through the TLS domain):

1. **Genesis** a new **24-word BIP-39** seed. The words are shown **once** — **write them down**.
   There is **no autofill** in production (`VITE_DEMO_MODE` unset), and the seed is never stashed.
2. **Re-type the challenge words** to confirm (you key them in manually).
3. Set a **strong passphrase**. The seed is scrypt/age-encrypted under it and stored as a `CustodyBlob`
   in Mongo. **A lost passphrase is unrecoverable** — there is no reset.
4. **Unlock** with that passphrase to wire the signer into the chain client.
5. **Re-unlock after every api restart** (custody is not auto-unlocked).

**`/admin/*` exposure.** With `ADMIN_LOOPBACK_ONLY=1` (set by `remote-up.sh`), the custody/genesis/
unlock routes are served on a **separate `127.0.0.1:${ADMIN_PORT}`** listener (default `PORT+1`) and
are **omitted from the public `0.0.0.0:PORT` listener**. Caddy additionally **denies `/api/admin/*`** at
the edge (`respond @admin 403`, with a commented `remote_ip` allowlist for a trusted office IP/VPN).
Run admin actions from the host (or via the allowlisted CIDR) — never from the open internet.

**Rate-limiting.** `/login`, `/admin/login`, and `/admin/unlock` are rate-limited (HTTP **429** on
lockout) with lenient thresholds.

The business signer also needs **on-chain funding + whitelisting** before it can issue (the funding is
not automated for production): fund the genesis signer with gas (PLASMA on ROAX) and have central
**approve** its issuer application (§7) to `whitelistFor` it.

---

## 6. Secrets

Generate every secret with `openssl rand -hex 32` (never reuse the demo defaults). The api fail-closes
on placeholders. Templates: [`stacks/<x>/.env.example`](../stacks/admin/.env.example) (backend) and
[`stacks/<x>/web/.env.example`](../stacks/vet/web/.env.example) (portal `VITE_*`).

| Secret | Stack(s) | Read from | Protects | Generate |
|---|---|---|---|---|
| `OPERATOR_PASSWORD` | vet, groomer | stack `.env` | operator login (`POST /login`) | `openssl rand -hex 32` |
| `ADMIN_PASSWORD` | all | stack `.env` | admin-session login (custody/admin console) | `openssl rand -hex 32` |
| `GENESIS_PASSPHRASE` (passphrase) | vet, groomer, admin | **typed at the portal** (never stored in `.env`) | decrypts the age-encrypted custody seed; **lost = unrecoverable** | strong human-chosen / `openssl rand -hex 32` |
| `CENTRAL_HMAC_SECRET` | all | stack `.env` | central↔business HMAC integrity (appointment events) | `openssl rand -hex 32` |
| `ADMIN_PRIVATE_KEY` | admin | `stacks/admin/.env` | on-chain signer for `whitelistFor` / SBT `mint` — a **dedicated funded EOA** | a funded EOA key (keep secret) |
| `ADMIN_ADDRESS` | admin | `stacks/admin/.env` | the public address derived from `ADMIN_PRIVATE_KEY` | derive from the key |

> **`CENTRAL_HMAC_SECRET` vs per-business `hmacSecret`.** `CENTRAL_HMAC_SECRET` is the shared-env
> secret. Separately, `register_business` (§7) **returns a per-business `hmacSecret` once** at
> registration (like an API key) — store it on the business side for appointment-event HMAC. These are
> distinct; keep both.

---

## 7. Manual on-chain onboarding (real endpoints, no demo buttons)

Forms are empty in production; operators key in real values. Endpoints below are verbatim from
[`stacks/admin/api/src/routes.rs`](../stacks/admin/api/src/routes.rs) and the ground-truth
[`scripts/e2e-smoke.sh`](../scripts/e2e-smoke.sh). The **central base** is the admin api (e.g.
`https://api.dogtag.io`). Note that `admin/login` and `approve` are **admin-router** routes
(loopback-only under `ADMIN_LOOPBACK_ONLY=1`); `businesses` and `issuer-applications` POST are public.

1. **Central admin login** (admin/loopback) → returns `token`:
   ```
   POST /v1/admin/login          { "password": "<ADMIN_PASSWORD>" }
   ```
2. **Register the business** (admin-session) → returns `businessId` + a one-time `hmacSecret`:
   ```
   POST /v1/businesses           Authorization: Bearer <token>
     { "type":"vet", "name":"<real name>", "lat":<lat>, "lng":<lng>,
       "services":["vaccination"], "apiBaseUrl":"https://<DOMAIN>",
       "domain":"<DOMAIN>", "documentStores":["<clone address>"] }
   ```
3. **Business applies as an issuer** (public submission) → returns `applicationId`:
   ```
   POST /v1/issuer-applications
     { "issuerEntityId":"<id>", "addresses":["<signer addr>"],
       "recordTypes":["VACCINATION"], "domain":"<DOMAIN>",
       "documentStore":"<clone address>", "license": { ... } }
   ```
   (Optional `usdaNan` is a 6-digit accreditation number; `license{number,jurisdiction,expiry}` if present.)
4. **Publish the DNS TXT** on `<DOMAIN>`: `dogtag-verify=<lowercased documentStore address>` (see §4).
5. **Central approves** (admin/loopback) — runs the **DoH DNS check, then on-chain `whitelistFor`**:
   ```
   POST /v1/issuer-applications/<applicationId>/approve   Authorization: Bearer <token>
   ```
   Returns `{ "status":"approved", "whitelistTxs":[...] }`. (DNS failure → `403 DNS TXT verification failed`.)
6. **Business custody genesis + unlock** (§5), then operator login + backend signing mode, then
   **prepare → `issue(root)`** anchors the Merkle root on the business's clone, and **share** returns a
   short one-time `/r/:token` URL for the QR. (Same flow as the smoke test steps 2–5.)

---

## 8. ZK proof-of-verification

- The Normal / ECDSA verification path works today. The **ZK (Groth16)** path uses
  **`CIRCUITS_BUILD_DIR`**: set it (e.g. `/circuits/build`) to load the real prover keys; **unset**
  falls back to `StubProver` (no real proofs).
- The live `Groth16Verifier` (`0x138b433071Ad806E841B5AD53623290a9bf21761`) is wired into the
  `VerificationRegistry`. The shipped testnet key is a **single-operator** setup
  ([`CEREMONY_TRANSCRIPT.md`](./CEREMONY_TRANSCRIPT.md)) — fine for testnet, **NOT** for a real
  deployment. A production ZK key requires the **multi-party ceremony**: see
  **[CEREMONY.md](./CEREMONY.md)** (≥3 contributors + public beacon, wired via the registry's 2-day
  `setZkVerifier` timelock — also in [DEPLOY.md](./DEPLOY.md)).

---

## 9. Switching to a different / production chain = config only

Moving off ROAX testnet to another chain is a **configuration change, no code edits** (`CHAIN_ID` and
every contract address are env-driven). Checklist:

- [ ] **Backend `.env`** (each stack): set `ROAX_RPC` (the new RPC URL) and `CHAIN_ID`.
- [ ] **Backend `.env`** (each stack): update the `*_ADDR` contract addresses
      (`ISSUER_REGISTRY_ADDR`, `SBT_ADDR`, `VERIFICATION_REGISTRY_ADDR`, the `*_ISSUER_ADDR`, etc.).
- [ ] **`contracts/deployments/<chain>.json`**: the new chain's address book (source of truth).
- [ ] **Portal `web/.env`**: the `VITE_*` addresses (`VITE_ISSUER_REGISTRY_ADDR`, `VITE_DOGTAG_SBT_ADDR`,
      `VITE_VERIFICATION_REGISTRY_ADDR`, `VITE_POSEIDON6_ADDR`, `VITE_DOGTAG_ISSUER_ADDR`) **and**
      `VITE_ROAX_RPC`.
- [ ] **Mobile** `roax.json` bundle: the addresses + chainId the apps read.
- [ ] **`MONGO_URI`** + **endpoints/ports** as needed for the new environment.
- [ ] Leave **`VITE_DEMO_MODE` unset** — production stays manual-entry.

No source changes are required for any of the above. This covers the **backend-signing** path (the
default, and the path the e2e flow exercises). One known caveat: the optional **browser-wallet
(MetaMask) signing** path on the vet stack still pins chainId `135` in the unsigned-tx it hands the
wallet and in its confirm check, so wallet mode on a non-135 chain needs a small code fix until
`CHAIN_ID` is threaded through that path — backend signing is unaffected.

---

## See also

- **[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md)** — the demo runbook (`VITE_DEMO_MODE=1`, MemStore).
- **[DEPLOY.md](./DEPLOY.md)** — ROAX contract deploy + Docker bring-up runbook.
- **[CEREMONY.md](./CEREMONY.md)** — production ZK trusted-setup ceremony.
- **[DPIA.md](./DPIA.md)** — Data Protection Impact Assessment.
- **[`deploy/Caddyfile`](../deploy/Caddyfile)** · **[`scripts/remote-up.sh`](../scripts/remote-up.sh)** — TLS proxy + production bring-up.
