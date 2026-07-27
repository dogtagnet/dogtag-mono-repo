# DogTag — PRODUCTION deployment (REMOTE + go-live hardening)

**Goal / you'll end with:** a hardened, self-hosted DogTag deployment that is safe to put real users on —
running on a real production chain (or deliberately staying on ROAX testnet), with a **multi-party ZK
trusted-setup key wired through the on-chain timelock**, **rotated secrets**, a **dedicated funded admin
EOA**, edge-locked admin routes, and **mobile apps rebuilt for the production chain**.

> **Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps. Every
> fragile step has a **Verify.** block and a **STOP if…** gate — do not proceed past a failed gate.

---

## 0. Read REMOTE first — this doc is ONLY the go-live delta

This is **Tier 3 = REMOTE + hardening**. It does **not** re-teach the base bring-up. Stand the system up
exactly as in **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** first (Docker compose, persistent Mongo,
Caddy TLS, real DNS-TXT issuer legitimacy, manual custody genesis/unlock, on-chain onboarding). Everything
below is **only the differences** needed to go live:

| § | Delta over REMOTE |
|---|---|
| 1 | Readiness gates (don't start until REMOTE works end-to-end on testnet) |
| 2 | **Chain swap** — config only, no code edits (this doc OWNS the chain-swap checklist) |
| 3 | **ZK trusted setup + verifier timelock** — BLOCKING (this doc OWNS the ceremony/timelock runbook) |
| 4 | Hardened secrets + edge-locked admin |
| 5 | Run the prover-service as the **owner's trusted prover** |
| 6 | Known caveat: wallet (MetaMask) signing pins chainId 135 |
| 7 | Go-live verification checklist + final STOP gates |

The canonical reference tables live in REMOTE and DEPLOYMENT:

- **Backend `.env` table** and **portal `VITE_*` table** — owned by [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md).
- **Address Book** + **service/port tables** — owned by [DEPLOYMENT.md](./DEPLOYMENT.md).
- **Address rule:** never transcribe contract addresses into prose. The source of truth is
  `contracts/deployments/<chain>.json` (testnet: `contracts/deployments/roax.json`); for a quick lookup
  use the one Address Book in [DEPLOYMENT.md](./DEPLOYMENT.md).

---

## 1. Readiness checklist (do NOT start hardening until these pass)

**Prerequisite gate:** REMOTE must already work **end-to-end on ROAX testnet** before you change anything
here. Bring it up and exercise the full flow per [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md). Only then
apply this delta.

Go-live gates — every box must be checked:

- [ ] REMOTE stands up cleanly: `scripts/remote-up.sh` builds all three stacks (`admin`, `vet`, `groomer`)
      with `--build-arg FEATURES=mongo` and they boot (no fail-closed Mongo error).
- [ ] A full **issue → share → EXPORT/verify** round-trip succeeds against testnet with **no demo
      autofill** (`VITE_DEMO_MODE` unset; `remote-up.sh` rejects it if set).
- [ ] **TLS** is live on every stack: `https://<DOMAIN>/health` → `{"status":"ok"}` (Caddy + Let's Encrypt).
- [ ] **DNS-TXT** legitimacy works for real domains: the issuer `dogtag-verify=` TXT passes at approve
      time, and a phone EXPORT to the groomer passes the phone-side groomer DNS check (REMOTE §4).
- [ ] **Mongo** is internal-only (port 27017, never published) and **backups** of `admindata` / `vetdata`
      / `groomerdata` are running (custody lives there; a lost passphrase is unrecoverable).
- [ ] Custody genesis + unlock done per stack; you can **re-unlock after a restart** (REMOTE §5).
- [ ] You have decided your target chain (stay on ROAX testnet, or swap — §2) and have an **RPC**, a
      **funded deployer/admin EOA**, and (if swapping) a **fresh `contracts/deployments/<chain>.json`**.

**STOP if** any box is unchecked → fix it in [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) before
continuing. Hardening on top of a broken REMOTE only hides the failure.

---

## 2. Chain swap (config only — NO code edits)

> This doc OWNS the chain-swap checklist.

Moving off ROAX testnet to a production chain is a **configuration change, not a code change**: `CHAIN_ID`,
`ROAX_RPC`, and every contract address are env-driven on the backend and portal, and baked-but-editable in
the mobile apps. (`ROAX_RPC` / `VITE_ROAX_RPC` are just the variable *names* — set them to **whatever RPC
your target chain uses**; nothing requires ROAX.) The one exception is the browser-wallet signing path —
see the caveat in §6.

You must update **four** surfaces in lockstep, then **rebuild the apps**. Skipping any one leaves a split
brain (e.g. portals on the new chain, phones still on the old).

Placeholders used in this section:

- `<NEW_RPC>` — the JSON-RPC URL of the target chain. Replace: from your chain operator.
- `<NEW_CHAIN_ID>` — the target chain's numeric id. Replace: `cast chain-id --rpc-url <NEW_RPC>`.
- `<chain>` — a short slug for the deployment file, e.g. `roax`, `mainnet`. Replace: your choice.

### 2.1 Backend `.env` (per stack: admin, vet, groomer)

For **each** of `stacks/admin/.env`, `stacks/vet/.env`, `stacks/groomer/.env`, set the chain endpoint and
**every** `*_ADDR` to the new chain's addresses. Field ownership/values are in the backend `.env` table in
[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md); do not invent keys.

```bash
# Edit each stack .env. The chain endpoint (all stacks):
#   ROAX_RPC=<NEW_RPC>          # the variable is named ROAX_RPC but holds ANY chain's RPC
#   CHAIN_ID=<NEW_CHAIN_ID>
#
# The contract addresses — set on the stacks that read each one (see the REMOTE .env table for which
# stack owns which key). Take every value from contracts/deployments/<chain>.json (§2.2):
#   ISSUER_REGISTRY_ADDR=...            # all stacks
#   VERIFICATION_REGISTRY_CONSENT_ADDR=... # vet, groomer  (VerificationRegistryConsent - the owner-hidden
#                                       #   relay target for POST /v1/verify/consent. The relayer -
#                                       #   custody account 0 - must be whitelisted for the purpose it
#                                       #   submits. Unset/malformed -> only that route 503s, fail-closed)
#   SBT_CONSENT_ADDR=...                # vet only  (DogTagSBTConsent - the owner-blind mintCustodial
#                                       #   target for POST /profiles/issue/custodial-bind, the sole
#                                       #   profile issuance path. The signer must hold ISSUER_ROLE on
#                                       #   it. Inert on a groomer: BUSINESS_TYPE=groomer mounts no
#                                       #   issuance routes at all)
#   PROFILE_ISSUER_ADDR=...             # vet only  (a real factory-deployed DogTagIssuer clone that
#                                       #   profile roots are anchored into via issue(R), so rootIssuer[R]
#                                       #   resolves. NEVER the SBT - issue(R) sent to the SBT reverts.
#                                       #   Signer must be whitelisted for the clone's recordType)
#     SBT_CONSENT_ADDR + PROFILE_ISSUER_ADDR are REQUIRED AS A PAIR: if either is unset/malformed the
#     custodial-bind route fails closed with 503 BEFORE the one-time bind token is consumed.
#   VERIFICATION_REGISTRY_ADDR=...      # admin  (the SAME owner-hidden VerificationRegistryConsent,
#                                       #   read by admin as the protocol default stamped into
#                                       #   unstamped credential imports - provenance routing, not a
#                                       #   relay target)
#   SBT_ADDR=...                        # admin  (DogTagSBTConsent as the governance target -
#                                       #   ISSUER_ROLE administration only; admin does not issue tags)
#   FACTORY_ADDR=...                    # ALL  (DogTagIssuerFactory. admin: createIssuer/predictIssuer
#                                       #   + Ownable owner. vet/groomer: a SECURITY setting - its
#                                       #   write-once rootIssuer[R] is what tells POST /verify/credential
#                                       #   which clone really issued a credential, instead of believing
#                                       #   the document's own issuer.documentStore, which sits OUTSIDE
#                                       #   the Merkle root and is therefore attacker-chosen. Unset ->
#                                       #   that pillar reports unavailableNoFactoryConfigured and a
#                                       #   forged document is refused by nothing but integrity.
#                                       #   Malformed -> the route answers 500 instead of failing open.
#                                       #   Set it on every VERIFYING deployment, the groomer included)
#   VACCINATION_ISSUER_ADDR=...         # vet only  (per-recordType clone; 0x0…0 for pure verifiers)
# Leave VITE_DEMO_MODE UNSET (remote-up.sh rejects it).
```

**Critical:** the testnet deployment ledger still carries **retired and legacy registry generations** as
history (struck rows in the Address Book). The only live registry is **`VerificationRegistryConsent`** -
never point any `*_ADDR` at a retired or `_legacy` generation. The Address Book in
[DEPLOYMENT.md](./DEPLOYMENT.md) marks which is current.

### 2.2 `contracts/deployments/<chain>.json` (the new source of truth)

The address book for the new chain is a deployment JSON. If you deployed the contract set to the new
chain, the deploy scripts write this file: `forge script script/Deploy.s.sol:Deploy` deploys the shared
base (IssuerRegistry + DogTagIssuer impl + factory) and the owner-hidden issuance/verification set is
deployed by its own script - see the deploy runbook [DEPLOY.md](./DEPLOY.md). This file - not any doc,
not any `.env` - is the **canonical address source**; everything in 2.1, 2.3, and 2.4 must be copied
**from it**.

```bash
# Confirm the deployment file exists for the new chain and lists every contract you reference:
ls -1 contracts/deployments/<chain>.json
cat contracts/deployments/<chain>.json   # eyeball: IssuerRegistry, DogTagIssuerFactory (+impl),
                                         # DogTagSBTConsent, VerificationRegistryConsent,
                                         # Groth16VerifierConsent, ProtocolRegistry, chainId
```

**Verify.** `chainId` inside `contracts/deployments/<chain>.json` equals `<NEW_CHAIN_ID>`.

**STOP if** the file is missing or `chainId` mismatches → you have not actually deployed (or wired) the set
on the target chain. Deploy first per [DEPLOY.md](./DEPLOY.md); do not hand-edit addresses into `.env`.

### 2.3 Portal `web/.env` (every `VITE_*` address + `VITE_ROAX_RPC`)

For each stack's portal env (`stacks/admin/web/.env`, `stacks/vet/web/.env`, `stacks/groomer/web/.env`),
set the read-only chain RPC and every contract `VITE_*` address from `contracts/deployments/<chain>.json`.
The full `VITE_*` table is owned by [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md).

```bash
# Per stack web/.env — read-only chain RPC + contract addresses:
#   VITE_ROAX_RPC=<NEW_RPC>                  # the variable is named *_ROAX_RPC but holds ANY chain's RPC
#   VITE_ISSUER_REGISTRY_ADDR=...
#   VITE_DOGTAG_ISSUER_ADDR=...              # per-recordType issuer for isValid polling (optional)
# Keep VITE_DEMO_MODE UNSET.
```

### 2.4 REBUILD the mobile apps — each bundles its OWN `roax.json`

The phones do **not** read backend `.env`. Each app bundles its **own copy** of `roax.json` (a trimmed
subset of contract addresses) and bakes the chain RPC as a constant. **There is no sync script** that
copies addresses into the apps - you must hand-edit **both** files, re-vendor the production proving
assets, then rebuild and reinstall. Full mobile build steps are in **[MOBILE_BUILD.md](./MOBILE_BUILD.md)**.

```bash
# 1. Hand-edit BOTH app roax.json copies to the new chain's addresses (from contracts/deployments/<chain>.json):
#      apps/android/app/src/main/assets/roax.json
#      apps/ios/DogTag/roax.json
#    (verified iOS == Android; keep them identical.)
#
# 2. If the CHAIN itself changed (not just addresses), also update the baked RPC constant in each app
#    (exact file paths also in MOBILE_BUILD.md):
#      iOS:     apps/ios/DogTag/Models.swift                                            -> AppConfig.roaxRpc = "<NEW_RPC>"
#      Android: apps/android/app/src/main/java/io/liberalize/dogtag/data/AppConfig.kt   -> ROAX_RPC         = "<NEW_RPC>"
#
# 3. Re-vendor the PRODUCTION consent proving assets (the §3 ceremony zkey; the app needs the pair
#    consent_final.zkey + consent.graph):
cp circuits/build/consent_final.zkey apps/ios/DogTag/
cp circuits/build/consent_final.zkey apps/android/app/src/main/assets/
cp circuits/build/consent.graph      apps/ios/DogTag/
cp circuits/build/consent.graph      apps/android/app/src/main/assets/
#    Deliberately explicit copies rather than `make vendor-mobile-artifacts`: that target verifies
#    the graph against the TESTNET attested hash and would refuse a production ceremony's graph.
#    Rotate the attested hash FIRST (docs/ARTIFACT_PIN_RUNBOOK.md), after which the make target is
#    the better command because it re-checks the hash. Until then these copies are unverified - it
#    is on you to confirm the pair matches the deployed production verifier (AGENTS.md).
#
# 4. Rebuild + reinstall both apps (see MOBILE_BUILD.md for the full commands):
#      iOS:     cd apps/ios && xcodegen && <Xcode Run / xcodebuild ...>
#      Android: cd apps/android && ./gradlew :app:assembleDebug && ./gradlew :app:installDebug
```

**STOP if** you skip the rebuild: a phone built for the old chain will read **old addresses** and silently
talk to the old contracts even though the backend and portals moved. There is no runtime override for the
bundled addresses — only a rebuild changes them.

### 2.5 Verify the swap

Confirm the RPC's chain id and that the portals read the new addresses.

```bash
# (a) The RPC really is the target chain:
cast chain-id --rpc-url <NEW_RPC>        # expect: <NEW_CHAIN_ID>

# (b) A contract actually exists at the new registry address (non-empty bytecode):
cast code <VerificationRegistryConsent from contracts/deployments/<chain>.json> --rpc-url <NEW_RPC> | head -c 12
#     expect: 0x6080...  (non-empty). Empty (0x) = wrong address or wrong chain.
```

**Verify.** `cast chain-id --rpc-url <NEW_RPC>` prints `<NEW_CHAIN_ID>`; the portals (loaded over TLS)
show the new addresses in their config; a fresh issue/verify round-trip lands on the new chain.

**STOP on mismatch.** If `cast chain-id` ≠ `<NEW_CHAIN_ID>`, or `cast code` is empty, or the portals still
show old addresses → one of 2.1/2.2/2.3 is stale or you pointed at the wrong RPC. Reconcile **all** copies
against `contracts/deployments/<chain>.json` before going further. A split brain (portals new, phones old,
or vice versa) produces "unknown root" / signature-verification failures that look like ZK bugs.

> Note on `--legacy`: ROAX requires legacy gas (EIP-1559 txs are accepted but never mined). If your target
> chain is EIP-1559-normal you may drop `--legacy`; keep it for ROAX-family chains.

---

## 3. ZK trusted setup (BLOCKING) — ceremony + verifier timelock

> This doc OWNS the on-chain ceremony/timelock wiring (the step-by-step ceremony itself is in
> [CEREMONY.md](./CEREMONY.md) - the live consent-ceremony guide, with
> [CEREMONY_RUNBOOK.md](./CEREMONY_RUNBOOK.md) as the expanded captain-fill-in detail; the on-chain
> wiring procedure is here).

The consent zkey shipped in `circuits/build` (the key the on-chain consent verifier checks against) is a
**single-operator** setup - `circuits/scripts/ceremony-consent.sh`, transcript
[CEREMONY_TRANSCRIPT.consent.md](./CEREMONY_TRANSCRIPT.consent.md) - fine for testnet, **NOT
production**.
A sole contributor who kept the toxic waste could **forge consent attestations**.
Before any real user relies on the owner-hidden verify path you MUST replace it with a multi-party
ceremony key and wire it through the registry's on-chain timelock.

> The Merkle-proof side of the protocol (issuance anchoring, integrity, selective disclosure) does
> **not** depend on this ceremony.
> The ceremony gates the ZK consent path (`recordVerificationZK`) - the **only** on-chain verification
> path - which is why this is a go-live blocker, not an optional hardening step.

### 3.1 Run the ceremony (per CEREMONY.md)

Follow **[CEREMONY.md](./CEREMONY.md)**: **≥3 independent contributors** each add and destroy secret
entropy in sequence, then the coordinator applies a **public random beacon** (a value unpredictable at
contribution time - e.g. a drand round) and finalizes.
The production run is a re-run of the committed `circuits/scripts/ceremony-consent.sh` phases with one
`snarkjs zkey contribute` per independent contributor inserted between setup and beacon.

A completed ceremony yields these things you carry forward:

1. `circuits/Groth16Verifier.consent.sol` - the exported verifier contract for **this** key
   (contract `Groth16VerifierConsent`).
2. `circuits/build/consent_final.zkey` - the production proving key (re-vendor into both apps, §2.4,
   and into the owner's prover-service, §5).
3. `circuits/build/consent_verification_key.json` - the JSON verification key so anyone can
   independently run `snarkjs groth16 verify`; publish it alongside the transcript.
4. A pinned **sha256** of the final zkey - publish it in the transcript, pin it in CI and the prover
   config (§5).

**STOP: do not use the testnet zkey in production.** The testnet key's sha256 is recorded in
[CEREMONY_TRANSCRIPT.consent.md](./CEREMONY_TRANSCRIPT.consent.md) (single-operator self-run). If your
apps/prover are serving that hash, the consent path is **forgeable** - block go-live until they serve
the new ceremony hash.

### 3.2 Deploy the verifier and wire it through the 2-day timelock

The registry does **not** have a `setZkVerifier` — the swap is a real **2-day timelock**: propose, wait,
execute. The function names and constant are verbatim from
`contracts/src/VerificationRegistryConsent.sol`: `proposeZkVerifier(address)`, `executeZkVerifier()`,
and `ZK_TIMELOCK = 2 days`. Both calls are `onlyRole(DEFAULT_ADMIN_ROLE)` - send them from the
registry's `DEFAULT_ADMIN`.

Placeholders:

- `<VRC_ADDR>` - the **VerificationRegistryConsent** address. Replace: from
  `contracts/deployments/<chain>.json` (or the Address Book in [DEPLOYMENT.md](./DEPLOYMENT.md) / the
  apps' `roax.json`). Use the **current** registry, never a retired/`_legacy` generation.
- `<DEPLOYER_PRIVATE_KEY>` — the registry's `DEFAULT_ADMIN` key. Replace: your protocol-admin EOA key.
- `<NEW_RPC>` — the target chain RPC (§2).

```bash
cp circuits/Groth16Verifier.consent.sol contracts/src/Groth16VerifierConsent.sol
cd contracts && forge build

# 1) Deploy the ceremony's verifier:
VERIFIER=$(forge create src/Groth16VerifierConsent.sol:Groth16VerifierConsent \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy --json | jq -r .deployedTo)
echo "new verifier: $VERIFIER"

# 2) Propose it — starts the 2-day timer (ZK_TIMELOCK = 2 days):
cast send <VRC_ADDR> "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy

# 3) WAIT >= 2 days, then execute (reverts with "timelock" if you call it early):
cast send <VRC_ADDR> "executeZkVerifier()" \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy
```

**Verify.** After step 2, `cast call <VRC_ADDR> "pendingZkVerifier()(address)" --rpc-url <NEW_RPC>` returns
`$VERIFIER` and `cast call <VRC_ADDR> "zkVerifierEta()(uint256)" --rpc-url <NEW_RPC>` is ~now + 172800s.
After step 3 (≥2 days later), `cast call <VRC_ADDR> "zkVerifier()(address)" --rpc-url <NEW_RPC>` returns
`$VERIFIER` and `pendingZkVerifier()` is back to `0x0`.

**STOP if** `executeZkVerifier()` reverts `timelock` → fewer than 2 days have elapsed since `propose`;
wait. **STOP if** either call reverts on access control → you are not sending from the registry's
`DEFAULT_ADMIN`. **STOP if** `proposeZkVerifier` is rejected as an unknown function → you are pointed at
a retired/**legacy** registry address (the wrong generation); use the current `<VRC_ADDR>`.

> Note: on **testnet** the consent verifier was wired **at construction** of the registry (the
> constructor takes the verifier address), so no timelock wait was needed there. **In production use
> the timelock above**, not a redeploy: a redeploy changes the registry address and forces every
> backend/portal/app to re-point, defeating the point of go-live stability. (History: the retired
> owner-revealing registry's v2 verifier cutover, executed 2026-07-02, exercised exactly this
> propose → wait → execute flow.)

---

## 4. Hardened secrets + edge-locked admin

REMOTE already requires strong secrets and rejects `change-me` placeholders; production tightens it. The
backend `.env` table is owned by [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) — fill it from
`stacks/<x>/.env.example` and apply these production values.

### 4.1 Rotate every demo default

Generate **every** secret freshly; never ship a demo default to production.

```bash
# Run once per secret you need; paste the output into the matching stack .env key:
openssl rand -hex 32
```

| Secret | Stack(s) | Demo default to rotate AWAY from | Production rule |
|---|---|---|---|
| `OPERATOR_PASSWORD` | vet, groomer | `operator` | `openssl rand -hex 32`, per stack |
| `ADMIN_PASSWORD` | all | `admin` | `openssl rand -hex 32`, per stack |
| `CENTRAL_HMAC_SECRET` | all | `dev-central-hmac-secret` | `openssl rand -hex 32`, **identical across all three stacks** |
| genesis passphrase | vet, groomer, admin | (none — typed at the portal) | strong, **typed at the portal**, never in `.env`; **lost = unrecoverable** |

> `CENTRAL_HMAC_SECRET` must be the **same value in all three stacks** (it signs central↔business
> appointment events). It is **distinct** from the per-business `hmacSecret` that `register_business`
> returns **once** at registration — keep both; they are not interchangeable.

> **Fail-closed boot.** In production (neither `DEMO_MODE` nor `VITE_DEMO_MODE` set) each api binary
> **refuses to start** if any of these is unset/empty or still equal to its built-in dev default
> (`OPERATOR_PASSWORD` / `ADMIN_PASSWORD` / `CENTRAL_HMAC_SECRET` on vet+groomer; `ADMIN_PASSWORD` /
> `ADMIN_PRIVATE_KEY` on admin). It exits with a `FATAL:` message naming every offending secret, so a
> half-rotated `.env` can never boot a production stack on a demo credential.

### 4.2 Dedicated funded admin EOA (never the demo deployer)

`ADMIN_PRIVATE_KEY` / `ADMIN_ADDRESS` in `stacks/admin/.env` is the on-chain signer that broadcasts
`whitelistFor` and SBT `mint`. In production this must be a **dedicated, funded EOA you control** — **not**
the demo deployer key from `contracts/.env` (that file is **LOCAL-only**; remote/prod read the key from
`stacks/admin/.env`).

```bash
# Generate a fresh admin EOA and derive its address:
cast wallet new                                  # prints a fresh private key + address
cast wallet address --private-key <ADMIN_PRIVATE_KEY>   # confirm the address matches
# Set in stacks/admin/.env:  ADMIN_PRIVATE_KEY=<key>  ADMIN_ADDRESS=<address>
# Then FUND it with gas on the target chain (PLASMA on ROAX) and grant it the on-chain governance
# authorities it needs (IssuerRegistry WHITELIST_ADMIN; DogTagSBTConsent issuer-role administration;
# factory ownership for createIssuer) - see DEPLOY.md.
```

**Verify.** `cast balance <ADMIN_ADDRESS> --rpc-url <NEW_RPC>` is non-zero (it must pay gas to whitelist
issuers / mint).

**STOP if** `ADMIN_ADDRESS` is the demo deployer (`0x119F8c7F…`) or has zero balance → onboarding will
either reuse the demo key in production or fail with out-of-gas. Use a dedicated funded EOA that holds
the on-chain admin roles. (On the **live ROAX testnet** that authority is governance signer-1
`0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` - Governance Phase-2 (2026-07-05, block 123835) removed the
`0x119F…` deployer EOA's governance/admin authority, so it can no longer grant whitelists. It is **not
role-free** - it still holds the retired owner-revealing SBT's `ISSUER_ROLE` + record-type whitelists,
so it can still mint on that retired contract and must not be reused as a neutral key.)

### 4.3 Edge-lock the admin surface (Caddy)

Defence in depth on top of `ADMIN_LOOPBACK_ONLY=1`:

- With `ADMIN_LOOPBACK_ONLY=1` (set by `remote-up.sh`), the custody/genesis/unlock routes are served on a
  **separate `127.0.0.1:${ADMIN_PORT}`** listener (default `PORT+1`) and are **omitted from the public
  `0.0.0.0:PORT` listener**. Run admin actions from the host (e.g. over SSH), not the open internet.
- Caddy additionally **denies `/api/admin/*` at the edge** (returns **403**), so the central admin router
  is not reachable through the public proxy. See [`deploy/Caddyfile`](../deploy/Caddyfile)
  (`respond @admin 403`).
- Optionally allow a **trusted office/VPN CIDR** through to admin by uncommenting the `remote_ip`
  allowlist in the Caddyfile — only enable this for a CIDR you control.

**Verify.** From outside your network: `curl -s -o /dev/null -w '%{http_code}' https://<DOMAIN>/api/admin/login`
returns **403** (denied at the edge). `curl -s https://<DOMAIN>/health` still returns `{"status":"ok"}`.

**STOP if** `/api/admin/*` returns anything other than 403 from the public internet → the edge deny is not
in effect; fix the Caddyfile before go-live. The admin router can whitelist issuers and must never be
publicly reachable.

---

## 5. Run the prover-service as the owner's trusted prover

Same mechanics as the prover-service in [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) §8 — but in
production it is **operated by you (the owner), monitored, and behind TLS**.

- **Why it must be the owner's (or owner-trusted) infra:** the prover **sees the witness** (the request
  carries `ownerSecret` / `ownerAddress`) while it builds the proof. It is therefore **NOT the
  verifier/groomer** - they only ever receive the resulting proof, never the witness. Running it on
  verifier infra would leak exactly what the consent path exists to hide. It still hides the owner from
  the verifier, relayer, chain, and the emitted event.
- **Who actually needs it:** proving is **on-device**; the `POST /prove-consent` fallback exists for
  devices that cannot prove on-device (e.g. 32-bit-only Android). The device-side offload wiring lands
  in a later slice, so today the fallback is exercised against the API directly. If you have no such
  users, you do **not** need to run this at all.
- **REMOTE does not start one** (`scripts/remote-up.sh` runs `admin,vet,groomer` only). Production stands
  it up the same way REMOTE describes — but as the owner's monitored, TLS-fronted service.

```bash
# Build the prover binary (the `prover` cargo feature mounts POST /prove-consent; the stack images'
# FEATURES=mongo build-arg does not include it):
cargo build --release -p vet-api --features prover --target-dir target/prover

# Run it with the PRODUCTION circuits dir (must contain the §3 ceremony consent_final.zkey plus
# consent.r1cs + consent_js/consent.wasm). Loading is lazy and FAIL-CLOSED PER REQUEST: an unset dir,
# missing/corrupt artifacts, or a zkey hash mismatch make /prove-consent return an error instead of a
# proof - there is no stub or placeholder fallback.
#
# The prover ENFORCES a pinned zkey sha256: the built-in pin is the TESTNET consent hash, so the §3
# ceremony zkey would be REJECTED (hash mismatch) unless you tell it the new hash. Set
# CONSENT_EXPECTED_ZKEY_SHA256 to your §3 ceremony zkey's sha256 (the value the ceremony recorded;
# the same key is re-vendored into the apps in §2.4). This is a pure config swap, not a code change.
CIRCUITS_BUILD_DIR=<path to circuits/build with the ceremony consent artifacts> \
CONSENT_EXPECTED_ZKEY_SHA256=<sha256 of the §3 ceremony consent_final.zkey> \
ROAX_RPC=<NEW_RPC> \
PORT=<owner-chosen> \
  target/prover/release/vet-api
```

Put it behind **TLS** (its own hostname / Caddy) and **monitor** it; it mounts `POST /prove-consent`
(unauthenticated by design - it returns only a proof, not data). See
[TUNNELING.md](./TUNNELING.md) for giving it a reachable HTTPS URL and
[MOBILE_BUILD.md](./MOBILE_BUILD.md) for the app's endpoint model.

**Verify.** `CIRCUITS_BUILD_DIR` points at a dir containing the **ceremony** `consent_final.zkey`
(matching the §3.1 pinned sha256) + `consent.r1cs` + `consent_js/consent.wasm`, and
`CONSENT_EXPECTED_ZKEY_SHA256` is set to that same ceremony sha256 (so the pin check passes instead of
fail-closing on the testnet default). A proof produced by this service is accepted by
`recordVerificationZK` on the new chain.

**STOP if** `/prove-consent` fails closed with a prover-unavailable or hash-mismatch error → the dir is
unset/incomplete, or `CONSENT_EXPECTED_ZKEY_SHA256` was not set to the ceremony hash. **STOP if** the
service still serves the **testnet** zkey hash → forgeable key; only run with the ceremony key from §3.

---

## 6. Known caveat — wallet (MetaMask) signing pins chainId 135

The chain swap in §2 is "config only" for the **default** path (**backend signing** — the mode the e2e
flow exercises). There is **one** exception: the **vet stack's optional browser-wallet (MetaMask) signing
path** hardcodes chainId **135** in the **unsigned transaction** it hands the wallet **and** in its confirm
check. So **wallet mode on a non-135 chain needs a small code fix** to thread `CHAIN_ID` through that path.

- **Backend signing (the default, and what e2e tests):** **unaffected** by the swap — no code change.
- **Browser-wallet signing on chain 135 (incl. ROAX):** works as-is.
- **Browser-wallet signing on a non-135 chain:** needs the small code fix before it works.

**Decision fork.** If you stay on **chain 135** or only use **backend signing** → nothing to do here. If
you swap to a **non-135 chain AND want browser-wallet signing** → schedule the code fix (thread `CHAIN_ID`
into the unsigned-tx + confirm check on the vet stack) before relying on wallet mode.

---

## 7. Go-live verification checklist + final STOP gates

Run this last, after §§2–5. Every box must pass before real users.

- [ ] **Ceremony done + verifier wired (§3).** `cast call <VRC_ADDR> "zkVerifier()(address)"` returns the
      **ceremony** `Groth16VerifierConsent`; apps + prover serve the **ceremony** consent-zkey sha256
      (not the testnet hash).
- [ ] **Secrets rotated (§4).** No `operator` / `admin` / `dev-central-hmac-secret` defaults remain;
      `CENTRAL_HMAC_SECRET` identical across all three stacks; `ADMIN_PRIVATE_KEY` is a **dedicated funded
      EOA** (not the demo deployer) with non-zero balance.
- [ ] **Admin edge-locked (§4.3).** Public `GET /api/admin/login` → **403**; `/admin/*` is loopback-only.
- [ ] **Mongo backups.** `admindata` / `vetdata` / `groomerdata` are backed up and restorable (custody
      lives there; lost passphrase = unrecoverable).
- [ ] **DNS-TXT published.** Issuer `dogtag-verify=<lowercased documentStore addr>` resolves for every
      business; the phone-side **groomer** TXT resolves for the EXPORT host (REMOTE §4).
- [ ] **Apps rebuilt for the prod chain (§2.4).** Both `roax.json` files updated from
      `contracts/deployments/<chain>.json`, the production consent proving assets re-vendored, both apps
      rebuilt + reinstalled; a phone round-trip lands on the **new** chain.
- [ ] **Prover reachable if you run the server-prove fallback (§5).** Owner-run, TLS-fronted, real
      consent prover (`CIRCUITS_BUILD_DIR` at the ceremony build; `CONSENT_EXPECTED_ZKEY_SHA256` set to
      the ceremony hash). (Skip if you do not offer the fallback.)
- [ ] **Chain sanity (§2.5).** `cast chain-id --rpc-url <NEW_RPC>` == `<NEW_CHAIN_ID>`; portals show the
      new addresses; a fresh issue → verify round-trip succeeds.

**Final STOP gates** (any failure blocks go-live):

1. **STOP** if `zkVerifier()` is still the testnet verifier or the apps/prover serve the testnet zkey hash
   → the consent path is forgeable (§3).
2. **STOP** if any demo secret survives, or `ADMIN_PRIVATE_KEY` is the demo deployer / unfunded (§4).
3. **STOP** if `/api/admin/*` is reachable from the public internet (§4.3).
4. **STOP** if Mongo is published to the host or has no backups (custody loss is unrecoverable).
5. **STOP** if the apps were not rebuilt for the new chain (phones on old addresses → "unknown root"
   failures) (§2.4).

> **Privacy & legal obligations.** Going live means processing real owners' personal data. Read
> **[DPIA.md](./DPIA.md)** for the GDPR/CCPA obligations this deployment must satisfy — in particular the
> **right-to-erasure / crypto-shredding** flow (`erase(ownerId, scope)`: destroy the per-record DEK, delete
> the off-chain row, propagate central → business, burn the SBT). The DPIA is a **living document**:
> refresh it on any change to the on-chain data model, the verification subsystem, or the erasure flow.

---

## See also

- **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** — the base bring-up this doc deltas over (backend
  `.env` + portal `VITE_*` tables; prover-service §8).
- **[DEPLOYMENT.md](./DEPLOYMENT.md)** — index, Address Book, service/port tables, tier decision-guide.
- **[CEREMONY.md](./CEREMONY.md)** - the live consent trusted-setup ceremony guide
  (**[CEREMONY_RUNBOOK.md](./CEREMONY_RUNBOOK.md)** is the expanded captain-fill-in runbook).
- **[DEPLOY.md](./DEPLOY.md)** - contract deploy runbook (writes `contracts/deployments/<chain>.json`).
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** — build + install the iOS/Android apps and rebuild on chain swap.
- **[TUNNELING.md](./TUNNELING.md)** — prover/host reachability and the phone networking model.
- **[DPIA.md](./DPIA.md)** — Data Protection Impact Assessment (privacy + erasure obligations).
- **[`deploy/Caddyfile`](../deploy/Caddyfile)** · **[`scripts/remote-up.sh`](../scripts/remote-up.sh)** — TLS proxy + bring-up.
