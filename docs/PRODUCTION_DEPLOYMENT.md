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
#   SBT_ADDR=...                        # admin (+ vet via demo wiring); PROFILE_DOCUMENT_STORE usually = SBT_ADDR
#   VERIFICATION_REGISTRY_ADDR=...      # vet, groomer  (CURRENT VR, not a legacy generation)
#   CONSENT_KEY_REGISTRY_ADDR=...       # vet, groomer  (CURRENT CKR — meta-tx bindConsentKeyFor is the live path)
#   VACCINATION_ISSUER_ADDR=...         # vet, groomer  (per-recordType clone; 0x0…0 for pure verifiers)
# Leave VITE_DEMO_MODE UNSET (remote-up.sh rejects it).
```

**Critical:** there are **three VerificationRegistry generations** and two ConsentKeyRegistry generations
in the testnet Address Book. Use the **current** VR/CKR (the meta-tx ones), never a `_legacy` address. The
Address Book in [DEPLOYMENT.md](./DEPLOYMENT.md) marks which is current.

### 2.2 `contracts/deployments/<chain>.json` (the new source of truth)

The address book for the new chain is a deployment JSON. If you deployed the contract set to the new chain,
`forge script script/Deploy.s.sol:Deploy` writes this file (see the deploy runbook
[DEPLOY.md](./DEPLOY.md) §2, and §3.2 for wiring the verifier). This file — not any doc, not any `.env` —
is the **canonical address source**; everything in 2.1, 2.3, and 2.4 must be copied **from it**.

```bash
# Confirm the deployment file exists for the new chain and lists every contract you reference:
ls -1 contracts/deployments/<chain>.json
cat contracts/deployments/<chain>.json   # eyeball: VerificationRegistry, ConsentKeyRegistry, IssuerRegistry,
                                         # DogTagSBT, Groth16Verifier, Poseidon6, factory/impl, chainId
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
#   VITE_DOGTAG_SBT_ADDR=...
#   VITE_VERIFICATION_REGISTRY_ADDR=...      # CURRENT VR
#   VITE_POSEIDON6_ADDR=...
#   VITE_DOGTAG_ISSUER_ADDR=...              # per-recordType issuer for isValid polling (optional)
# Keep VITE_DEMO_MODE UNSET.
```

### 2.4 REBUILD the mobile apps — each bundles its OWN `roax.json`

The phones do **not** read backend `.env`. Each app bundles its **own copy** of `roax.json` (a trimmed
subset of contract addresses) and bakes the chain RPC as a constant. **There is no sync script** that
copies addresses into the apps — you must hand-edit **both** files, re-vendor the production zkey, then
rebuild and reinstall. Full mobile build steps are in **[MOBILE_BUILD.md](./MOBILE_BUILD.md)**.

```bash
# 1. Hand-edit BOTH app roax.json copies to the new chain's addresses (from contracts/deployments/<chain>.json):
#      apps/android/app/src/main/assets/roax.json
#      apps/ios/DogTag/roax.json
#    (verified iOS == Android; keep them identical.)
#
# 2. If the CHAIN itself changed (not just addresses), also update the baked RPC constant in each app
#    (exact file paths also in MOBILE_BUILD.md §8):
#      iOS:     apps/ios/DogTag/Models.swift                                            -> AppConfig.roaxRpc = "<NEW_RPC>"
#      Android: apps/android/app/src/main/java/io/liberalize/dogtag/data/AppConfig.kt   -> ROAX_RPC         = "<NEW_RPC>"
#
# 3. Re-vendor the PRODUCTION zkey (the gitignored ~65 MB key from §3) into BOTH apps:
cp circuits/build/verification_final.zkey apps/ios/DogTag/
cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/
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

# (b) A contract actually exists at the new VR address (non-empty bytecode):
cast code <VERIFICATION_REGISTRY_ADDR from contracts/deployments/<chain>.json> --rpc-url <NEW_RPC> | head -c 12
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

> This doc OWNS the ceremony/timelock runbook (the step-by-step ceremony itself is in
> [CEREMONY.md](./CEREMONY.md); the on-chain wiring procedure is here).

The zkey shipped in `circuits/build` (and the one bundled in the testnet apps) is a **single-operator**
setup — fine for testnet, **NOT production**. A sole contributor who kept the toxic waste could **forge ZK
attestations**. Before any real user relies on the ZK EXPORT path you MUST replace it with a multi-party
ceremony key and wire it through the registry's on-chain timelock.

> The normal/ECDSA verification path (`recordVerification`) works on-chain today and does **not** depend on
> this ceremony. Only the ZK path (`recordVerificationZK`) is gated by it. Until the ceremony completes,
> leave the registry's `zkVerifier` unchanged / `0x0` for the ZK path.

### 3.1 Run the ceremony (per CEREMONY.md)

Follow **[CEREMONY.md](./CEREMONY.md)** exactly: **≥3 independent contributors** each add and destroy
secret entropy in sequence, then the coordinator applies a **public random beacon** (a value unpredictable
at contribution time — e.g. a future Bitcoin block hash or a drand round) and finalizes.

```bash
# (Coordinator) finalize, per CEREMONY.md — produces the production artefacts:
cd circuits
bash scripts/ceremony.sh finalize build/ceremony_final.zkey
#  -> exports circuits/Groth16Verifier.sol
#  -> copies build/verification_final.zkey   (the ~65 MB production key to vendor into the apps in §2.4)
#  -> prints the final zkey sha256 to PIN (CI + the prover image)
```

Finalize produces three things you carry forward:

1. `circuits/Groth16Verifier.sol` — the verifier contract for **this** key.
2. `circuits/build/verification_final.zkey` — the production proving key (re-vendor into both apps, §2.4,
   and into the owner's prover-service, §5).
3. A pinned **sha256** of the final zkey — publish it in the transcript, pin it in CI and the prover image.

**STOP: do not use the testnet zkey in production.** The testnet key's sha256 is recorded in
[CEREMONY_TRANSCRIPT.md](./CEREMONY_TRANSCRIPT.md) (single-operator). If your apps/prover are serving that
hash, the ZK path is **forgeable** — block go-live until they serve the new ceremony hash.

### 3.2 Deploy the verifier and wire it through the 2-day timelock

The registry does **not** have a `setZkVerifier` — the swap is a real **2-day timelock**: propose, wait,
execute. The function names and constant are verbatim from `contracts/src/VerificationRegistry.sol`:
`proposeZkVerifier(address)`, `executeZkVerifier()`, and `ZK_TIMELOCK = 2 days` (the same timelock also
guards the swappable consent-key registry via `proposeConsentKeys` / `executeConsentKeys`). Both calls are
`onlyRole(DEFAULT_ADMIN_ROLE)` — send them from the registry's `DEFAULT_ADMIN`.

Placeholders:

- `<VR_ADDR>` — the **VerificationRegistry** address. Replace: from `contracts/deployments/<chain>.json`
  (or the Address Book in [DEPLOYMENT.md](./DEPLOYMENT.md) / the apps' `roax.json`). Use the **current**
  VR, never a `_legacy` one.
- `<DEPLOYER_PRIVATE_KEY>` — the registry's `DEFAULT_ADMIN` key. Replace: your protocol-admin EOA key.
- `<NEW_RPC>` — the target chain RPC (§2).

```bash
cp circuits/Groth16Verifier.sol contracts/src/Groth16Verifier.sol
cd contracts && forge build

# 1) Deploy the ceremony's verifier (deployer must be the registry DEFAULT_ADMIN):
VERIFIER=$(forge create src/Groth16Verifier.sol:Groth16Verifier \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy --json | jq -r .deployedTo)
echo "new verifier: $VERIFIER"

# 2) Propose it — starts the 2-day timer (ZK_TIMELOCK = 2 days):
cast send <VR_ADDR> "proposeZkVerifier(address)" "$VERIFIER" \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy

# 3) WAIT >= 2 days, then execute (reverts with "timelock" if you call it early):
cast send <VR_ADDR> "executeZkVerifier()" \
  --rpc-url "<NEW_RPC>" --private-key "<DEPLOYER_PRIVATE_KEY>" --legacy
```

**Verify.** After step 2, `cast call <VR_ADDR> "pendingZkVerifier()(address)" --rpc-url <NEW_RPC>` returns
`$VERIFIER` and `cast call <VR_ADDR> "zkVerifierEta()(uint256)" --rpc-url <NEW_RPC>` is ~now + 172800s.
After step 3 (≥2 days later), `cast call <VR_ADDR> "zkVerifier()(address)" --rpc-url <NEW_RPC>` returns
`$VERIFIER` and `pendingZkVerifier()` is back to `0x0`.

**STOP if** `executeZkVerifier()` reverts `timelock` → fewer than 2 days have elapsed since `propose`;
wait. **STOP if** either call reverts on access control → you are not sending from the registry's
`DEFAULT_ADMIN`. **STOP if** `proposeZkVerifier` is rejected as an unknown function → you are pointed at a
**legacy** VR address (the wrong generation); use the current `<VR_ADDR>`.

> Note: on **testnet** the ZK verifier was wired by **redeploying** the whole VerificationRegistry with the
> verifier set at construction (to avoid waiting out the timelock on testnet) — see
> [DEPLOY.md](./DEPLOY.md) §3.2. **In production use the timelock above**, not a redeploy: the redeploy
> changes the VR address and forces every backend/portal/app to re-point, defeating the point of go-live
> stability.

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
# Then FUND it with gas on the target chain (PLASMA on ROAX) and grant it the on-chain admin roles
# (WHITELIST_ADMIN + DogTagSBT ISSUER_ROLE) — see DEPLOY.md §3.
```

**Verify.** `cast balance <ADMIN_ADDRESS> --rpc-url <NEW_RPC>` is non-zero (it must pay gas to whitelist
issuers / mint).

**STOP if** `ADMIN_ADDRESS` is the demo deployer (`0x119F8c7F…`) or has zero balance → onboarding will
either reuse the demo key in production or fail with out-of-gas. Use a dedicated funded EOA.

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

- **Why it must be the owner's (or owner-trusted) infra:** the prover **sees the witness** (the cleartext
  record + consent) while it builds the proof. It is therefore **NOT the groomer** — the groomer only ever
  receives the resulting proof, never the witness. Running it on groomer infra would leak exactly what the
  ZK path exists to hide.
- **Who actually needs it:** only **32-bit-only Android** devices (`Build.SUPPORTED_64_BIT_ABIS.isEmpty()`).
  64-bit iPhone and modern arm64 Android prove **on-device** and never call a prover. If you do not support
  32-bit Android, you do **not** need to run this at all.
- **REMOTE does not start one** (`scripts/remote-up.sh` runs `admin,vet,groomer` only). Production stands
  it up the same way REMOTE describes — but as the owner's monitored, TLS-fronted service.

```bash
# Build the prover binary (cargo feature `prover` — orthogonal to the FEATURES=mongo docker build-arg):
cargo build --release -p vet-api --features prover --target-dir target/prover

# Run it with the PRODUCTION circuits dir so the REAL ArkProver loads (must contain the §3 ceremony
# verification_final.zkey + verification.graph). If CIRCUITS_BUILD_DIR is UNSET it silently loads the
# StubProver, whose proofs are NOT chain-valid. If it is SET but the real prover fails to load
# (missing/corrupt zkey or graph), the service is fail-closed and EXITS with a FATAL error instead.
#
# The prover ENFORCES a pinned zkey sha256 (audit M4): its hardcoded default is the TESTNET hash, so a
# production ceremony zkey would be REJECTED at load (hash mismatch -> FATAL) unless you tell it the new
# hash. Set EXPECTED_ZKEY_SHA256 to your §3 ceremony zkey's sha256 (the value scripts/ceremony.sh finalize
# printed; also re-vendored into the apps in §2.4). This is a pure config swap, not a code change.
CIRCUITS_BUILD_DIR=<path to circuits/build with the ceremony zkey+graph> \
EXPECTED_ZKEY_SHA256=<sha256 of the §3 ceremony verification_final.zkey> \
ROAX_RPC=<NEW_RPC> \
PORT=<owner-chosen> \
  target/prover/release/vet-api
```

Put it behind **TLS** (its own hostname / Caddy) and **monitor** it; mounts `POST /prove-verification`
(unauthenticated by design — it returns only a proof, not data). 32-bit-Android users point at it via the
app's `prover_api` override (Android SharedPrefs) — see [MOBILE_BUILD.md](./MOBILE_BUILD.md) and
[TUNNELING.md](./TUNNELING.md). The app's baked `DEFAULT_PROVER_API` is a dead tunnel, so those users MUST
set `prover_api` to your live prover host.

**Verify.** `CIRCUITS_BUILD_DIR` points at a dir containing the **ceremony** `verification_final.zkey`
(matching the §3.1 pinned sha256) + `verification.graph`, and `EXPECTED_ZKEY_SHA256` is set to that same
ceremony sha256 (so the prover's pin check passes instead of fail-closing on the testnet default). A proof
produced by this service is accepted by `recordVerificationZK` on the new chain (i.e. it was built by
ArkProver, not StubProver).

**STOP if** `CIRCUITS_BUILD_DIR` is unset or points at the **testnet** zkey → StubProver / forgeable
proofs, or if the service FATALs on a zkey-hash mismatch → set `EXPECTED_ZKEY_SHA256` to the ceremony
hash. Only run with the ceremony key from §3.

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

- [ ] **Ceremony done + verifier wired (§3).** `cast call <VR_ADDR> "zkVerifier()(address)"` returns the
      **ceremony** verifier; apps + prover serve the **ceremony** zkey sha256 (not the testnet hash).
- [ ] **Secrets rotated (§4).** No `operator` / `admin` / `dev-central-hmac-secret` defaults remain;
      `CENTRAL_HMAC_SECRET` identical across all three stacks; `ADMIN_PRIVATE_KEY` is a **dedicated funded
      EOA** (not the demo deployer) with non-zero balance.
- [ ] **Admin edge-locked (§4.3).** Public `GET /api/admin/login` → **403**; `/admin/*` is loopback-only.
- [ ] **Mongo backups.** `admindata` / `vetdata` / `groomerdata` are backed up and restorable (custody
      lives there; lost passphrase = unrecoverable).
- [ ] **DNS-TXT published.** Issuer `dogtag-verify=<lowercased documentStore addr>` resolves for every
      business; the phone-side **groomer** TXT resolves for the EXPORT host (REMOTE §4).
- [ ] **Apps rebuilt for the prod chain (§2.4).** Both `roax.json` files updated from
      `contracts/deployments/<chain>.json`, the production zkey re-vendored, both apps rebuilt + reinstalled;
      a phone round-trip lands on the **new** chain.
- [ ] **Prover reachable if you have 32-bit users (§5).** Owner-run, TLS-fronted, real ArkProver
      (`CIRCUITS_BUILD_DIR` set to the ceremony build); 32-bit Android `prover_api` points at it. (Skip if
      no 32-bit Android.)
- [ ] **Chain sanity (§2.5).** `cast chain-id --rpc-url <NEW_RPC>` == `<NEW_CHAIN_ID>`; portals show the
      new addresses; a fresh issue → verify round-trip succeeds.

**Final STOP gates** (any failure blocks go-live):

1. **STOP** if `zkVerifier()` is still the testnet verifier or the apps/prover serve the testnet zkey hash
   → the ZK path is forgeable (§3).
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
- **[CEREMONY.md](./CEREMONY.md)** — the multi-party ZK trusted-setup ceremony, step by step.
- **[DEPLOY.md](./DEPLOY.md)** — contract deploy runbook (writes `contracts/deployments/<chain>.json`;
  §3.2 verifier wiring).
- **[MOBILE_BUILD.md](./MOBILE_BUILD.md)** — build + install the iOS/Android apps and rebuild on chain swap.
- **[TUNNELING.md](./TUNNELING.md)** — prover/host reachability and the phone networking model.
- **[DPIA.md](./DPIA.md)** — Data Protection Impact Assessment (privacy + erasure obligations).
- **[`deploy/Caddyfile`](../deploy/Caddyfile)** · **[`scripts/remote-up.sh`](../scripts/remote-up.sh)** — TLS proxy + bring-up.
