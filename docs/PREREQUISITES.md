# PREREQUISITES — install matrix & per-tier verify gates

**Goal / you'll end with:** every tool, secret, and config file the DogTag deploy paths need —
installed and verified — so the tier docs ([LOCAL](./LOCAL_DEPLOYMENT.md),
[REMOTE](./REMOTE_DEPLOYMENT.md), [PRODUCTION](./PRODUCTION_DEPLOYMENT.md),
[MOBILE](./MOBILE_BUILD.md), [TUNNELING](./TUNNELING.md)) can run their fenced blocks top-to-bottom
without hitting a missing dependency.

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps.

This doc **OWNS** the install/tooling matrix. Other docs' per-tier "you need X, Y, Z" callouts link
here for the actual install commands — do not duplicate install steps there.

---

## 0. How to read this

Each tool is tagged with the tiers that need it. **Install once on your machine; then run the
per-tier verify block in [§3](#3-one-shot-verify-all-blocks-per-tier).**

| Tag | Tier | What it is |
|---|---|---|
| **LOCAL** | Tier 1 | Run everything on one Mac from source — [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md) (`scripts/demo-up.sh`). |
| **REMOTE** | Tier 2 | Self-host the stacks via Docker on your server, still ROAX testnet — [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) (`scripts/remote-up.sh`). |
| **PROD** | Tier 3 | REMOTE + go-live hardening (chain swap, ceremony, timelock) — [PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md). |
| **MOBILE-iOS** | — | Build + install the iPhone app on a real device — [MOBILE_BUILD.md](./MOBILE_BUILD.md). |
| **MOBILE-Android** | — | Build + install the Android app on a real device — [MOBILE_BUILD.md](./MOBILE_BUILD.md). |

Notes that apply everywhere on this chain:

- ROAX testnet uses **LEGACY gas** — every `cast`/`forge` call uses `--legacy` (EIP-1559 txs are
  accepted but never mined). RPC `https://devrpc.roax.net`, chainId **135**, gas token **PLASMA**.
- The repo pins toolchain versions in a few places — they are called out inline below. The ones that
  bite if wrong: **Node ≥ 22**, **pnpm 10.19.0**, **Rust ≥ 1.80**, **JDK 17** (Android),
  **iOS 16.0** deployment target, foundry **solc 0.8.28 / `evm_version=paris`**.

---

## 1. The install matrix

Install the tools your tier(s) need, then jump to [§3](#3-one-shot-verify-all-blocks-per-tier) to
verify them all at once. macOS uses [Homebrew](https://brew.sh); Linux examples use Debian/Ubuntu
`apt` (translate to your distro's package manager as needed).

| Tool | macOS (Homebrew) | Linux (apt) | Needed by | Verify command |
|---|---|---|---|---|
| **Rust toolchain** (`cargo`, `rustc`) | `brew install rustup-init && rustup-init -y` (or `brew install rust`) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y` then `. "$HOME/.cargo/env"` | LOCAL; REMOTE/PROD *(only if you run a prover-service)*; MOBILE *(to regenerate the native libs, **and** to run `apps/android`'s JVM unit tests — every Gradle `Test` task hard-`dependsOn` a HOST `cargo build`, see the Android note below)* | `cargo --version` → `cargo 1.8x` (repo needs **≥ 1.80**, see `Cargo.toml` `rust-version`) |
| **Node.js** | `brew install node` (need **≥ 22**) | `sudo apt-get install -y nodejs` (use [NodeSource](https://github.com/nodesource/distributions) for v22+; distro nodejs is usually too old) | LOCAL (vite portals); MOBILE *(only if rebuilding circuits)* | `node --version` → `v22.*` or newer (`package.json` engines `node >=22`) |
| **pnpm** | `brew install pnpm` (or `corepack enable && corepack prepare pnpm@10.19.0 --activate`) | `corepack enable && corepack prepare pnpm@10.19.0 --activate` (corepack ships with Node ≥ 16.10) | LOCAL (workspace install + portals) | `pnpm --version` → `10.19.0` (root `package.json` `packageManager: pnpm@10.19.0`) |
| **foundry** (`cast`, `forge`, `anvil`) | `curl -L https://foundry.paradigm.xyz \| bash` then restart shell and run `foundryup` | same — `curl -L https://foundry.paradigm.xyz \| bash` then `foundryup` | LOCAL (`cast balance`, chain prechecks); PROD (`forge`/`cast` for the timelock); contracts deploy | `cast --version` and `forge --version` both print a version line. `cast chain-id --rpc-url https://devrpc.roax.net` → `135` |
| **jq** | `brew install jq` | `sudo apt-get install -y jq` | LOCAL (scripts parse JSON; reading `contracts/deployments/roax.json`) | `jq --version` → `jq-1.*` |
| **python3** | preinstalled, or `brew install python` | `sudo apt-get install -y python3` | LOCAL (helper tooling) | `python3 --version` → `Python 3.*` |
| **cloudflared** | `brew install cloudflared` | download the `.deb` from [Cloudflare](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) and `sudo dpkg -i cloudflared*.deb` (no official apt package) | LOCAL *(only to reach a phone on another network — see [TUNNELING.md](./TUNNELING.md))* | `cloudflared --version` → `cloudflared version *` |
| **Docker + Docker Compose** | [Docker Desktop](https://www.docker.com/products/docker-desktop/) (bundles `docker compose`) | `sudo apt-get install -y docker.io docker-compose-plugin` (or follow [docs.docker.com](https://docs.docker.com/engine/install/)); add yourself to the `docker` group | REMOTE; PROD | `docker --version` and `docker compose version` both print a version line; `docker ps` works without sudo |
| **openssl** | preinstalled (LibreSSL), or `brew install openssl@3` | `sudo apt-get install -y openssl` | REMOTE; PROD (generate every secret with `openssl rand -hex 32`) | `openssl version` → `OpenSSL 3.*` / `LibreSSL *` |
| **git** | preinstalled (`xcode-select --install`), or `brew install git` | `sudo apt-get install -y git` | LOCAL (clone + the prover's `rust-witness` build fetches the witness source); all tiers | `git --version` → `git version 2.*` |
| **cmake** | `brew install cmake` | `sudo apt-get install -y cmake` | LOCAL (prover build: `build.rs` transpiles the circuit witness to native C via `rust-witness`); MOBILE *(only when regenerating native libs)* | `cmake --version` → `cmake version 3.*` |
| **C toolchain** (clang/gcc, make, linker) | `xcode-select --install` (Command Line Tools — ships clang + make) | `sudo apt-get install -y build-essential` | LOCAL (same prover `rust-witness` C build as cmake); MOBILE | `cc --version` (macOS: `clang`; Linux: `gcc`) prints a version |
| **Xcode** + Command Line Tools | install **Xcode** from the App Store, then `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer && sudo xcodebuild -license accept` | not available (Apple-only — iOS builds require macOS) | MOBILE-iOS | `xcodebuild -version` → `Xcode 15+` |
| **xcodegen** | `brew install xcodegen` | not available (macOS-only) | MOBILE-iOS (regenerates `apps/ios/DogTag.xcodeproj` from `project.yml`) | `xcodegen --version` → `Version: *` |
| **Apple Developer team** | a signed-in Apple Developer account in Xcode (Settings → Accounts) | n/a | MOBILE-iOS (device signing) | Xcode → Settings → Accounts shows your team. `project.yml` pins `DEVELOPMENT_TEAM: AYDBUX9433` — **replace with your own team id** if signing fails (see [MOBILE_BUILD.md](./MOBILE_BUILD.md)). |
| **Android SDK + platform-tools (`adb`)** | install [Android Studio](https://developer.android.com/studio) (SDK Manager → SDK Platforms + Platform-Tools), or `brew install --cask android-commandlinetools android-platform-tools` | install Android Studio, or `sudo apt-get install -y android-sdk` + the [command-line tools](https://developer.android.com/tools); `sudo apt-get install -y adb` for platform-tools | MOBILE-Android | `sdkmanager --version` (if installed) and `adb --version` → `Android Debug Bridge version 1.*`. macOS adb default path: `~/Library/Android/sdk/platform-tools/adb` |
| **JDK 17** | `brew install openjdk@17` then link per the brew caveat (e.g. `sudo ln -sfn $(brew --prefix)/opt/openjdk@17/libexec/openjdk.jdk /Library/Java/JavaVirtualMachines/openjdk-17.jdk`) | `sudo apt-get install -y openjdk-17-jdk` | MOBILE-Android (`build.gradle.kts` pins `JavaVersion.VERSION_17` + `JvmTarget.JVM_17`) | `java -version` → a **17.x** line |
| **Gradle** (via the committed wrapper) | nothing to install — use `./gradlew` | nothing to install — use `./gradlew` | MOBILE-Android | from `apps/android`: `./gradlew --version` → `Gradle 9.5.1` (pinned in `gradle/wrapper/gradle-wrapper.properties`; the wrapper downloads it on first run) |
| **cargo-ndk** | `cargo install cargo-ndk` | `cargo install cargo-ndk` | MOBILE-Android *(the native `.so` are gitignored — a fresh clone must (re)generate them, so this IS required for a fresh Android build; skip only if your working tree already has them)* | `cargo ndk --version` → `cargo-ndk *` |

### Notes on pinned / version-sensitive tools

- **Foundry / contracts** — `contracts/foundry.toml` pins `solc = "0.8.28"` and
  `evm_version = "paris"` ("consistent until ROAX PUSH0 confirmed"). Use a recent `foundryup`; the
  solc version is fetched per-project. All chain calls need `--legacy` (ROAX has no EIP-1559 mining).
- **Android SDK levels** — `apps/android/app/build.gradle.kts` pins **compileSdk 36**, **minSdk 26**,
  **targetSdk 34**, ABIs `armeabi-v7a` + `arm64-v8a`. Install the **android-36** platform via the SDK
  Manager (the build comment notes "android-36 is present in the SDK"). JDK **17** is required.
- **iOS** — `apps/ios/project.yml` pins deployment target **iOS 16.0**, bundle id
  `io.liberalize.dogtag`, scheme `DogTag`, `CODE_SIGN_STYLE: Automatic`, `DEVELOPMENT_TEAM AYDBUX9433`.
- **cargo-ndk IS needed for a fresh Android build.** The Android native libs (`libdogtag_standard.so`,
  `libcircom_witnesscalc-*.so`) under `apps/android/app/src/main/jniLibs/{armeabi-v7a,arm64-v8a}/` are
  **gitignored** (`apps/.gitignore:6`) and **NOT committed** — a fresh clone lacks them and must
  (re)generate them with `cargo-ndk` (plus `cmake` + a C toolchain + the NDK toolchain). If a dev
  machine already has them in the working tree, a rebuild isn't needed — but they are **not**
  "committed." Same for iOS: `DogTagFFI.xcframework` is gitignored and regenerated, but a normal app
  build does not regenerate it.
- **A HOST `cargo` is needed to run `apps/android`'s JVM unit tests** — separate from `cargo-ndk`, and
  separate from `assembleDebug`, which is unaffected. `app/build.gradle.kts` makes every Gradle `Test`
  task `dependsOn` a `cargo build -p dogtag-standard-rs --features prover --lib` against the workspace
  root, so `ProfileTreeParityTest` can call the REAL Rust core over JNA: the `jniLibs/` `.so` are
  Android-ABI-only and can never load on a dev machine. The dependency is deliberately hard, not a
  soft skip — a self-skipping parity test reports green in exactly the case it exists to catch — and
  `--features prover` is mandatory, since UniFFI checksum-verifies every export at library load and a
  default-feature build dies with `UnsatisfiedLinkError: …checksum_func_prove_consent`. `sdk.dir` in
  `apps/android/local.properties` (§2.4) is required for `test` as well as for the build. Details:
  AGENTS.md "Build & test".
- **The cargo `prover` feature ≠ the docker `FEATURES=mongo` build-arg** — orthogonal. The prover
  service is `cargo build --release -p vet-api --features prover`; the persistent-store docker image
  is built `--build-arg FEATURES=mongo`. Do not confuse the two.

---

## 2. Project secrets & config you must have

Tools alone are not enough. Each tier also needs specific config files filled in. **Secrets are
never committed** — generate them with `openssl rand -hex 32` (REMOTE/PROD).

### 2.1 `contracts/.env` — funded deployer (LOCAL only)

LOCAL sources `contracts/.env` and wires its **governance signer-1** key as the central stack's on-chain
admin signer (it is also read by `demo-bootstrap.sh`, `demo-prepare-phone.sh`, and the `e2e-*.sh`
harnesses). Since Governance Phase-2 (executed on-chain 2026-07-05, block 123835) the demo tooling's admin
authority is **governance signer-1** (`0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2`) - Phase-2 removed the
old deployer EOA `0x119F8c7F…`'s governance/admin authority, so it can no longer grant whitelists or own
the factory. It is **not role-free**: it still holds the retired owner-revealing `DogTagSBT`'s
`ISSUER_ROLE` and record-type whitelists (re-verified 2026-07-16), so it can still mint on that retired
SBT and must never be treated as a neutral custodian. The
`DEPLOYER_*` key is retained only for `forge` contract deploys and the ceremony verifier-swap scripts.
**`contracts/.env` is LOCAL-only** - REMOTE/PROD use `stacks/admin/.env`'s `ADMIN_PRIVATE_KEY`/`ADMIN_ADDRESS`
(also governance signer-1) instead.

| Key | Purpose | How to get it | Secret? |
|---|---|---|---|
| `GOVERNANCE_PRIVATE_KEY` | the demo tooling's on-chain admin signer - **governance signer-1** (whitelistFor / SBT mint / factory createIssuer) + PLASMA gas source | signer-1's private key (`0x…`, 64 hex) - must be **FUNDED with PLASMA**; its address must be `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` | **YES - never commit** |
| `GOVERNANCE_ADDRESS` | the address of `GOVERNANCE_PRIVATE_KEY` (governance signer-1) | `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2` (derive: `cast wallet address --private-key <GOVERNANCE_PRIVATE_KEY>`) | no |
| `DEPLOYER_PRIVATE_KEY` | deploying EOA for `forge` contract deploys / the ceremony verifier-swap scripts only - **NOT** admin ops (Phase-2 removed its governance/admin authority; it still holds the retired owner-revealing SBT's `ISSUER_ROLE` + record-type whitelists, so it is not a neutral key) | a funded ROAX EOA private key (`0x…`, 64 hex) | **YES - never commit** |
| `DEPLOYER_ADDRESS` | the address of `DEPLOYER_PRIVATE_KEY` | derive: `cast wallet address --private-key <DEPLOYER_PRIVATE_KEY>` | no |
| `ROAX_RPC` | chain RPC | `https://devrpc.roax.net` | no |

`contracts/.env` is **gitignored** (`.gitignore:24`) and there is **no `contracts/.env.example`** —
so a fresh clone will NOT have it and must create it with these keys. **But first check
whether it already exists** (e.g. the shared demo setup on this machine):

> **If `contracts/.env` already exists, do NOT overwrite it.** A pre-existing file may hold a
> **throwaway TESTNET key** from the shared demo setup, and may predate Phase-2 (only `DEPLOYER_*`). Add
> `GOVERNANCE_PRIVATE_KEY` / `GOVERNANCE_ADDRESS` (governance signer-1) if missing, and ensure the keys
> point at **YOUR funded EOAs** (edit in place, keep `ROAX_RPC`). Any key that ships with such a setup is
> a **throwaway testnet key** - never fund it with anything you care about, and never reuse it in production.

Only if the file is **absent**, create it (run from the repo root; fill in the FUNDED ROAX EOA private keys):

```bash
# Run from the repo root. ONLY if contracts/.env does not already exist.
cat > contracts/.env <<'EOF'
# Governance signer-1 - the demo tooling's admin authority since Phase-2 (2026-07-05, block 123835).
GOVERNANCE_PRIVATE_KEY=0x<GOVERNANCE_SIGNER1_PRIVATE_KEY>
GOVERNANCE_ADDRESS=0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2
# Deploying EOA - only for `forge` deploys / ceremony scripts (no admin roles after Phase-2).
DEPLOYER_PRIVATE_KEY=0x<YOUR_FUNDED_ROAX_EOA_PRIVATE_KEY>
DEPLOYER_ADDRESS=0x<ITS_ADDRESS>
ROAX_RPC=https://devrpc.roax.net
EOF
chmod 600 contracts/.env   # contains private keys
```

**Verify the governance signer is funded** (it is the key the demo tooling whitelists / mints with; gas
token is PLASMA; ROAX uses legacy gas):

```bash
# Load the address from contracts/.env, then read its on-chain balance.
set -a; source contracts/.env; set +a
cast balance "$GOVERNANCE_ADDRESS" --rpc-url https://devrpc.roax.net
```

**Verify.** Output is a balance in wei **greater than 0**, e.g. `500000000000000000`. Convert to a
human number with `cast balance "$GOVERNANCE_ADDRESS" --rpc-url https://devrpc.roax.net --ether`.

**STOP if** the balance is `0` (or the command errors):
- **Symptom:** `cast balance` prints `0`, or `error sending request` / a connection error.
- **Likely cause:** the signer has no PLASMA, or `GOVERNANCE_ADDRESS` is wrong, or the RPC is unreachable.
- **Fix:** fund governance signer-1 with PLASMA on ROAX testnet (faucet / transfer) before continuing - a
  zero-balance admin signer cannot whitelist, mint, or run `demo-bootstrap.sh`. Re-check the address with
  `cast wallet address --private-key "$GOVERNANCE_PRIVATE_KEY"` → must be `0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2`.
  Confirm the RPC with `cast chain-id --rpc-url https://devrpc.roax.net` → must print `135`.

### 2.2 `circuits/build/` — the proving artifacts

The prover-service and **both mobile apps** need the owner-hidden consent proving artifacts present
in `circuits/build/`:

| File | Size | Used by |
|---|---|---|
| `circuits/build/consent_final.zkey` | ~24 MB | prover-service (`CIRCUITS_BUILD_DIR`) **and** the proving asset each app needs bundled |
| `circuits/build/consent.r1cs` + `circuits/build/consent_js/consent.wasm` | committed | prover-service witness assembly (`CIRCUITS_BUILD_DIR`) |
| `circuits/build/consent.graph` | ~1.5 MB, committed | the on-device witness backend each app needs bundled - **not** read by the prover-service |

The proving assets are **gitignored in the apps** (not at their `circuits/build/` sources, which are
committed), so they are vendored into each app build with `make vendor-mobile-artifacts` (see
[MOBILE_BUILD.md](./MOBILE_BUILD.md) for what the current app builds require).

> **Note:** BOTH consent artifacts are **committed** under `circuits/build/` (present on a fresh clone,
> force-added past the `build/` ignore), so neither needs an out-of-band build. The zkey's sha256
> `f83a111f…` is unconditionally pinned by the prover - `CONSENT_EXPECTED_ZKEY_SHA256` only
> **overrides** the pin for a deployment shipping a different key; unset = enforce the pinned testnet
> hash. The graph's bytes are attested in-repo by
> `dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256`, and `make vendor-mobile-artifacts` refuses
> to bundle either file if its hash disagrees. Rebuilding the graph is **not** a normal build step: it
> is an artifact **rotation** (the attested hash and the on-chain `witnessMobileSha256` move together) -
> follow [ARTIFACT_PIN_RUNBOOK.md](./ARTIFACT_PIN_RUNBOOK.md), never a bare
> `build-circuit` overwrite. The artifact pair carries the internal protocol version key
> `dogtag-levelb/1` (an internal identifier, not a product label).
> The retired verification-circuit artifacts (`verification_final.zkey`, `verification.r1cs`, …) remain
> in `circuits/build/` only as frozen provenance for the retired owner-revealing path - nothing builds
> or serves them.
> Run the Verify below first to confirm what you actually have.

**Verify both artifacts exist:**

```bash
ls -la circuits/build/consent_final.zkey circuits/build/consent.graph
```

**Verify.** Both files are listed, non-empty (zkey ~24 MB).

**STOP if** either is missing:
- **Symptom:** `ls: … No such file or directory`.
- **Likely cause:** the artifacts weren't checked out.
- **Fix:** both `consent_final.zkey` and `consent.graph` are committed, so a missing file means a broken
  checkout, not a skipped build - restore it with `git checkout -- circuits/build`. Do **not** rebuild
  the graph to fill the gap: that is a rotation, not a repair
  ([ARTIFACT_PIN_RUNBOOK.md](./ARTIFACT_PIN_RUNBOOK.md)), and the vendoring script would refuse the
  unattested bytes anyway. Without these, the prover-service is **fail-closed per
  request**: `POST /prove-consent` returns unavailable (it never serves a non-chain-valid proof), and
  the apps cannot prove on-device.

### 2.3 REMOTE / PROD — `stacks/<x>/.env`

For REMOTE/PROD, each stack (`admin`, `vet`, `groomer`) has its own `.env`, copied from the example
and filled in. Do this for each stack:

```bash
# Repeat for x in admin, vet, groomer.
cp stacks/<x>/.env.example stacks/<x>/.env
# then edit stacks/<x>/.env — see the key tables in REMOTE_DEPLOYMENT.md.
```

Generate every secret (`ADMIN_PASSWORD`, `OPERATOR_PASSWORD`, `CENTRAL_HMAC_SECRET`, and the admin
stack's `ADMIN_PRIVATE_KEY`) with `openssl rand -hex 32`. `remote-up.sh` **rejects required secrets
that are empty/unset** (and any literal `change-me`) — and rejects any `VITE_DEMO_MODE`. The
`.env.example` templates ship these secrets **BLANK** (e.g. `ADMIN_PASSWORD=`, `CENTRAL_HMAC_SECRET=`,
`ADMIN_PRIVATE_KEY=`, `OPERATOR_PASSWORD=`), so **fill every key whose value after `=` is empty**. The
portal `VITE_*` keys live in
`stacks/<x>/web/.env` (also copied from `.env.example`). The full backend `.env` and portal `VITE_`
tables are owned by **[REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md)** — fill in the values there.

> The admin stack's `ADMIN_PRIVATE_KEY` must be a **dedicated, funded EOA** (same funding check as
> [§2.1](#21-contractsenv--funded-deployer-local-only), but for the remote admin signer).

### 2.4 MOBILE-Android — `apps/android/local.properties`

The Gradle build needs to know where your Android SDK is:

```bash
# Point sdk.dir at your SDK install (macOS default shown). Replace <SDK_PATH> on Linux, e.g. $HOME/Android/Sdk.
printf 'sdk.dir=%s\n' "$HOME/Library/Android/sdk" > apps/android/local.properties
```

Replace: `<SDK_PATH>` = your Android SDK root (macOS default `~/Library/Android/sdk`; Linux default
`~/Android/Sdk`). This file is per-machine and must not be committed.

**Verify** `adb` is reachable (macOS default path shown; or just `adb` if it's on your `PATH`):

```bash
~/Library/Android/sdk/platform-tools/adb --version
```

**Verify.** Prints `Android Debug Bridge version 1.0.x`.

---

## 3. One-shot verify-all blocks, per tier

Paste the block for your tier(s). Each line that errors means that tool is missing — install it from
[§1](#1-the-install-matrix). Each block ends with a STOP gate.

### 3.1 LOCAL — verify all

Run from the repo root.

```bash
# LOCAL tier tools. Each line must print a version; none may error.
cargo --version          # Rust toolchain (need >= 1.80)
rustc --version
node --version           # need >= 22
pnpm --version           # 10.19.0
cast --version           # foundry
forge --version
jq --version
python3 --version
git --version
cmake --version          # prover rust-witness build
cc --version             # C toolchain (clang on macOS / gcc on Linux)
cloudflared --version    # only needed to reach a phone on another network
# chain reachability + funded deployer (config from §2.1):
cast chain-id --rpc-url https://devrpc.roax.net          # -> 135
set -a; source contracts/.env; set +a
cast balance "$DEPLOYER_ADDRESS" --rpc-url https://devrpc.roax.net   # -> > 0
# proving artifacts present (§2.2):
ls circuits/build/consent_final.zkey circuits/build/consent.graph
```

**Verify.** Every `--version` prints a line; `cast chain-id` → `135`; `cast balance` → a number
**> 0**; both `circuits/build/*` files are listed.

**STOP if any line errors or `cast balance` is `0`:**
- A `command not found` → install that tool from [§1](#1-the-install-matrix).
- `cast chain-id` not `135` → wrong RPC; use `https://devrpc.roax.net`.
- `cast balance` is `0` or `contracts/.env` is missing → fix [§2.1](#21-contractsenv--funded-deployer-local-only)
  (a zero-balance deployer cannot bootstrap).
- `circuits/build/*` missing → fix [§2.2](#22-circuitsbuild--the-proving-artifacts) (else consent
  proving is unavailable - the prover fails closed rather than serving a non-chain-valid proof).

### 3.2 REMOTE / PROD — verify all

```bash
# REMOTE/PROD tier tools. Each line must print a version; none may error.
docker --version
docker compose version
openssl version
git --version
# PROD timelock + ceremony also use cast/forge (PROD only):
cast --version           # PROD: proposeZkVerifier / executeZkVerifier (--legacy)
forge --version
# Only if you will run a prover-service on this host (REMOTE has none by default):
cargo --version          # >= 1.80   (skip if not running a prover here)
```

**Verify.** `docker --version`, `docker compose version`, and `openssl version` all print a line;
`docker ps` works without `sudo`. On PROD, `cast`/`forge` print versions.

**STOP if any line errors:**
- `docker compose` not found → install the Compose plugin (Docker Desktop bundles it; on Linux
  install `docker-compose-plugin`).
- `docker ps` needs `sudo` → add your user to the `docker` group and re-login.
- Missing `cargo` is **only** a problem if you intend to run a prover-service here (REMOTE does not
  start one — see [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) for standing one up yourself).
- Before `remote-up.sh`: confirm each `stacks/<x>/.env` exists and has no empty/unset required secrets
  (the `.env.example` templates ship them blank) — and no literal `change-me`
  ([§2.3](#23-remote--prod--stacksxenv)).

### 3.3 MOBILE — verify all

iOS requires **macOS**. Android works on macOS or Linux.

```bash
# --- MOBILE-iOS (macOS only) ---
xcodebuild -version       # Xcode 15+
xcodegen --version
# --- MOBILE-Android ---
java -version             # a 17.x line
adb --version             # or ~/Library/Android/sdk/platform-tools/adb --version
( cd apps/android && ./gradlew --version )   # Gradle 9.5.1 (wrapper downloads on first run)
# Native libs are gitignored (not committed): a fresh clone MUST regenerate them, so these are
# required for a fresh Android build (skip only if your working tree already has the .so):
cargo ndk --version
cmake --version
# HOST cargo is separately required to run the JVM unit tests (`gradle test` hard-depends on a host
# `cargo build --features prover`); not needed for assembleDebug:
cargo --version
# proving key+graph must exist before vendoring into the apps (§2.2):
ls circuits/build/consent_final.zkey circuits/build/consent.graph
```

**Verify.** iOS: `xcodebuild -version` → `Xcode 15+`, `xcodegen --version` prints a version. Android:
`java -version` shows **17.x**, `adb --version` prints, `./gradlew --version` → `Gradle 9.5.1`; both
`circuits/build/*` files are listed.

**STOP if any line errors:**
- `java -version` is not 17.x → install JDK 17 and make it active (Android pins `VERSION_17`; a
  different JDK will fail the Gradle build).
- `xcodebuild` / `xcodegen` not found → MOBILE-iOS needs macOS + Xcode + `brew install xcodegen`.
- `adb` not found → install platform-tools; macOS default path
  `~/Library/Android/sdk/platform-tools/adb`; ensure `apps/android/local.properties` `sdk.dir` is set
  ([§2.4](#24-mobile-android--appsandroidlocalproperties)).
- `./gradlew` fails on SDK → fix `local.properties` `sdk.dir`.
- `circuits/build/*` missing → fix [§2.2](#22-circuitsbuild--the-proving-artifacts); the proving
  assets are gitignored in the apps and are vendored at build time (see
  [MOBILE_BUILD.md](./MOBILE_BUILD.md)).

---

## Next

Tools verified — continue to your tier:
[LOCAL](./LOCAL_DEPLOYMENT.md) · [REMOTE](./REMOTE_DEPLOYMENT.md) ·
[PRODUCTION](./PRODUCTION_DEPLOYMENT.md) · [MOBILE](./MOBILE_BUILD.md) ·
[TUNNELING](./TUNNELING.md). Index: [DEPLOYMENT.md](./DEPLOYMENT.md).
