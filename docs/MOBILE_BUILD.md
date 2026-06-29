# MOBILE_BUILD — build & install the DogTag apps on real phones

**Goal / you'll end with:** the DogTag iOS app on a real iPhone and the DogTag Android app on a
real Android phone, each correctly configured to talk to the right vet/groomer hosts, the right chain,
and (for 32-bit Android only) a live prover-service.

> **Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps.
> Run every command from the repo root (`/Users/zhenhaowu/code/dogtag-mono-repo`) unless a block
> `cd`s somewhere. This doc OWNS the **mobile endpoint-model table** (§2); the LOCAL, REMOTE, and
> PRODUCTION docs link here rather than copying it.

Placeholders used below (define-once):

- `<DEVICE_UDID>` — the iPhone's device id. Replace: `<DEVICE_UDID>` = `xcrun xctrace list devices`
  (or Xcode → Window → Devices and Simulators), copy the UDID of the plugged-in iPhone.
- `<PROVER_TUNNEL_URL>` — the public base URL of a running prover-service (e.g. a `cloudflared`
  tunnel `https://<sub>.trycloudflare.com`, or your remote prover's TLS host). 32-bit Android only.
- `<SDK_DIR>` — the Android SDK path (`/Users/zhenhaowu/Library/Android/sdk` on this machine).

---

## 0. Goal + the one diagram

A phone gets its configuration from **four** distinct places. Knowing which is which is the whole
point of this doc — most "it's talking to the wrong thing" bugs are a confusion between them.

```
                         ┌──────────────────────────────────────────────┐
                         │                  THE PHONE APP                 │
                         └──────────────────────────────────────────────┘
                                 ▲           ▲            ▲           ▲
        SCANNED QR  (per scan)   │           │            │           │   MANUAL  (in-app pref)
   ┌─────────────────────────────┘           │            │           └─────────────────────────┐
   │  vet host     = QR  /p/<token>           │            │            prover_api               │
   │  groomer host = QR  /x/<token>           │            │            (32-bit Android ONLY;     │
   │  (the app has NO field for these)        │            │             POST /prove-verification)│
   └──────────────────────────────────────┐  │            │  ┌────────────────────────────────  ┘
                                           │  │            │  │
                              BAKED (bundled in the build) │  LEGACY fallback (rarely set)
                       ┌──────────────────────────────────┘  └──────────────────────────────┐
                       │  contract addresses = bundled roax.json                              │
                       │  chain RPC          = baked constant (https://devrpc.roax.net)       │
                       │  zkey + graph       = bundled assets (vendored each build)           │
                       │                                          central_api = api.dogtag.io │
                       └──────────────────────────────────────────────────────────────────────┘
```

- **SCANNED QR** — the vet host (issue a dog tag) and the groomer host (export/verify) come **only**
  from the QR the operator's portal renders. The app has no UI field for either host. See §2.
- **BAKED** — contract addresses (`roax.json`), the chain RPC constant, and the proving artifacts
  (`verification_final.zkey`, `verification.graph`) are compiled/bundled into the app. To change any
  of them you **edit + rebuild + reinstall** (§8).
- **MANUAL** — `prover_api` is the **only** setting a user ever types in-app, and **only** on a
  32-bit-only Android device (§3, §7).
- **LEGACY** — `central_api` (default `https://api.dogtag.io`) is the old ECDSA fallback path. It is
  **not** used in the QR/ZK flow. Leave it alone unless you know you need it.

---

## 1. Prerequisites

Full cross-tier install matrix is in [PREREQUISITES — install matrix](./PREREQUISITES.md). This
section is the mobile subset; verify each before building.

### 1a. iOS prerequisites (build on macOS only)

You need **Xcode** (with command-line tools), **xcodegen**, and an **Apple developer team** you are
signed into in Xcode.

```bash
# Verify Xcode + the command-line toolchain are installed and selected.
xcodebuild -version              # → e.g. "Xcode 16.x"
xcode-select -p                  # → a path ending in /Contents/Developer
# Verify xcodegen is installed (brew install xcodegen if missing).
xcodegen --version               # → a version string, e.g. "2.x.x"
```

**Verify.** `xcodebuild -version` prints an Xcode version and `xcodegen --version` prints a version.

**STOP if** `xcodegen: command not found` — install it: `brew install xcodegen`, then re-run.
**STOP if** `xcodebuild` errors about license/agreement — run `sudo xcodebuild -license accept`.

> You also need an Apple team selected in **Xcode → Settings → Accounts** (a free personal team
> works for on-device debug installs). The team id is set in `project.yml` (§5).

### 1b. Android prerequisites (macOS or Linux)

You need the **Android SDK**, **JDK 17**, **cargo-ndk**, and **adb**.

```bash
# Verify JDK 17 is the active JDK (Gradle here is pinned to Java 17 — see app/build.gradle.kts).
java -version                    # → version "17.x" (a 17.x line)
# Verify cargo-ndk is installed (cargo install cargo-ndk if missing).
cargo ndk --version              # → a cargo-ndk version string
# Verify the Android SDK path is recorded for Gradle.
test -f apps/android/local.properties && grep '^sdk.dir=' apps/android/local.properties
# Verify adb is present at the expected location and the daemon runs.
~/Library/Android/sdk/platform-tools/adb version   # → "Android Debug Bridge version ..."
```

**Verify.** `java -version` shows `17.x`, `cargo ndk --version` prints a version, the `grep` prints a
line like `sdk.dir=/Users/zhenhaowu/Library/Android/sdk`, and `adb version` prints a version banner.

**STOP if** `local.properties` is missing or has no `sdk.dir` — create it (one line):
`sdk.dir=<SDK_DIR>` (e.g. `sdk.dir=/Users/zhenhaowu/Library/Android/sdk`). Gradle reads this to find
the SDK. On this machine `<SDK_DIR>` = `/Users/zhenhaowu/Library/Android/sdk`.
**STOP if** `java -version` is not 17 — install/select JDK 17 (e.g. `brew install openjdk@17`) and
point `JAVA_HOME` at it.
**STOP if** `cargo ndk` is missing — `cargo install cargo-ndk`. The native libs in `jniLibs/`
(`libdogtag_standard.so`, `libcircom_witnesscalc-*.so`) are **gitignored and not in the repo**, and
the Gradle build does **not** invoke cargo-ndk to (re)build them — `assembleDebug` only bundles `.so`
files that already exist in `jniLibs/`. So on a **fresh clone** you must build them with cargo-ndk
**before** `:app:assembleDebug` (which is why cargo-ndk + the Rust/Android-NDK toolchain are required
here). A dev machine that already has the `.so` in its working tree will reuse them as-is — Gradle
won't rebuild them.

> `adb` is referenced throughout this doc as `~/Library/Android/sdk/platform-tools/adb`. If it is on
> your `PATH` you may just type `adb`.

---

## 2. The endpoint model (canonical — this table is owned here)

What the phone talks to, and where each value comes from. **Other docs link to this table; they do
not copy it.**

| setting | source | who sets it / when | notes |
|---|---|---|---|
| contract addresses | bundled `roax.json` | baked at build; edit + rebuild to change | iOS `apps/ios/DogTag/roax.json`, Android `apps/android/app/src/main/assets/roax.json` — a hand-maintained trimmed subset, **no sync script** copies it from `contracts/deployments/roax.json` |
| chain RPC | baked constant | rebuild to change | iOS `apps/ios/DogTag/Models.swift` `AppConfig.roaxRpc`; Android `AppConfig.ROAX_RPC` — both `https://devrpc.roax.net` |
| vet host (issue dog tag) | scanned QR `/p/<token>` | per scan | the device calls **only** the scanned host; the app has no field for it |
| groomer host (export / verify) | scanned QR `/x/<token>` | per scan | the device calls **only** the scanned host; the app has no field for it |
| `central_api` | iOS `UserDefaults` / Android `SharedPrefs`, default `https://api.dogtag.io` | rarely; legacy ECDSA fallback | **not** used in the QR / ZK path |
| `prover_api` | Android `SharedPrefs` only, default `AppConfig.DEFAULT_PROVER_API` | manually, **32-bit-only Android** | the lone manual setting; the baked default is a dead tunnel (§7) |

**The vet and groomer hosts come ONLY from the scanned QR.** There is no settings field for them in
either app — whatever host the operator's portal encodes into the `/p/` or `/x/` QR is the host the
phone calls, and nothing else. Contract addresses and the RPC are baked; do not look for them in the
app's settings either.

Per-contract addresses live in `contracts/deployments/roax.json` (and a quick-reference table in
[DEPLOYMENT — address book](./DEPLOYMENT.md)). This doc never transcribes addresses.

---

## 3. Proving: 64-bit vs 32-bit

The Groth16 proof for a groomer verification (the privacy-preserving export, `/x/` flow) is
generated **on the phone** wherever the hardware can run the native circom prover.

- **64-bit devices** — every iPhone, and any modern **arm64** Android — prove **on-device**. They do
  **not** use a prover URL at all; `prover_api` is irrelevant to them.
- **32-bit-only Android** — a device whose `Build.SUPPORTED_64_BIT_ABIS` is empty (checked in
  `apps/android/app/src/main/java/io/liberalize/dogtag/ui/screens/ScanScreen.kt`,
  `val is32BitOnly = Build.SUPPORTED_64_BIT_ABIS.isEmpty()`) **cannot** run the on-device prover. It
  POSTs `{wrappedDoc, consent, eddsaSig}` to a **prover-service** (`POST /prove-verification`) and
  submits the returned proof to the groomer itself — the groomer still never sees the witness.

Decision fork at export time:

- If the device is **64-bit** → nothing to configure; it proves locally. Skip §7.
- If the device is **32-bit-only Android** → you **must** set `prover_api` to a live prover-service
  (§7) before the groomer export will work.

**STOP if** a 32-bit-only Android has a **blank or stale** `prover_api` — the **Approve & present**
step in the groomer export **fails to produce a proof** (export proof fails / "no remote prover
configured" or a connection error). Fix: set `prover_api` to a live prover-service per §7.

---

## 4. Bundled assets (both apps)

Both apps bundle their own copies of the proving artifacts and a trimmed address file. Two of these
are committed; **two are absent from a fresh clone and must be vendored from `circuits/build/`** — the
`verification_final.zkey` (gitignored) and the `verification.graph` (untracked / never committed).

| asset | iOS path | Android path | committed? |
|---|---|---|---|
| `verification_final.zkey` (~65 MB) | `apps/ios/DogTag/verification_final.zkey` | `apps/android/app/src/main/assets/verification_final.zkey` | **no — vendor from `circuits/build`** (gitignored) |
| `verification.graph` (~3 MB) | `apps/ios/DogTag/verification.graph` | `apps/android/app/src/main/assets/verification.graph` | **no — vendor from `circuits/build`** (untracked, not committed) |
| `roax.json` (hand-maintained subset) | `apps/ios/DogTag/roax.json` | `apps/android/app/src/main/assets/roax.json` | yes |
| `testvectors.json` | `apps/ios/DogTag/testvectors.json` | `apps/android/app/src/main/assets/testvectors.json` | yes |

Both the `verification_final.zkey` and the `verification.graph` are 1:1 copies of the files under
`circuits/build/`. The zkey is gitignored in `apps/.gitignore` (so the 65 MB blob is never
double-committed) and the graph is simply never committed (untracked), which means **a fresh checkout
has neither in either app and the apps will not prove correctly until you vendor both.** Copy them
into both bundles:

```bash
# Vendor the proving key into BOTH app bundles (gitignored; ~65 MB each).
cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey
cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/verification_final.zkey
# Vendor the witness graph into BOTH app bundles (untracked; ~3 MB each).
cp circuits/build/verification.graph apps/ios/DogTag/verification.graph
cp circuits/build/verification.graph apps/android/app/src/main/assets/verification.graph
```

**Verify.** All four files exist and are non-trivial in size.

```bash
ls -l apps/ios/DogTag/verification_final.zkey \
      apps/android/app/src/main/assets/verification_final.zkey
# → two lines, each ~65 MB (≈ 64570945 bytes)
ls -l apps/ios/DogTag/verification.graph \
      apps/android/app/src/main/assets/verification.graph
# → two lines, each ~3 MB (≈ 2991853 bytes)
```

**STOP if** any path is missing or 0 bytes — `circuits/build/verification_final.zkey` or
`circuits/build/verification.graph` is absent, or the copy failed. Ensure `circuits/build/` is
populated (see [PREREQUISITES — circuits/build](./PREREQUISITES.md)), then re-run the copies.

> `roax.json` is **hand-maintained** — there is no script that syncs it from
> `contracts/deployments/roax.json`. If you swap chains/contracts you edit it by hand in **both** apps
> (§8).
>
> Android's **native libraries** (`libdogtag_standard.so`, `libcircom_witnesscalc-*.so`) live at
> `apps/android/app/src/main/jniLibs/armeabi-v7a/` and `apps/android/app/src/main/jniLibs/arm64-v8a/`
> — these are the on-device prover + FFI. They are **gitignored** (`apps/.gitignore`,
> `android/app/src/main/jniLibs/**/*.so`) and **not** committed, so a **fresh clone has none of
> them**. The Gradle build does **not** run cargo-ndk; `assembleDebug` only bundles `.so` files that
> already exist in `jniLibs/`. On a fresh clone you must **build them with cargo-ndk before
> `:app:assembleDebug`** (this needs the Rust + Android NDK toolchain — §1b). A dev machine that
> already has the `.so` in its working tree won't rebuild them on a normal app build.

---

## 5. iOS — build & install on a device

The Xcode project is **generated** from `apps/ios/project.yml` by `xcodegen` — do not hand-edit the
generated `DogTag.xcodeproj`. Source-of-truth facts from `project.yml`:

- bundle id `io.liberalize.dogtag`, scheme **`DogTag`**, deployment target **iOS 16.0+**
- `CODE_SIGN_STYLE: Automatic`, `DEVELOPMENT_TEAM: AYDBUX9433`
- links the UniFFI `DogTagFFI.xcframework` (gitignored; regenerated by the FFI pipeline — not part of
  a plain build here)

> **Build the `DogTagFFI.xcframework` first.** It is gitignored and a plain `xcodebuild` will fail to
> link without it. Build the Rust prover static lib for the iOS Simulator (`--features prover`),
> regenerate the Swift bindings, and assemble the framework. The full copy-pasteable sequence (and the
> on-device ZK self-test it powers) is in [`AGENTS.md` → Mobile end-to-end testing (iOS)](../AGENTS.md).

**Step 1 — vendor the zkey** (if you have not already, §4):

```bash
cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey
```

**Step 2 — generate the project:**

```bash
cd apps/ios && xcodegen
# → "Created project at .../apps/ios/DogTag.xcodeproj"
```

**Verify.** `DogTag.xcodeproj` now exists: `ls -d apps/ios/DogTag.xcodeproj`.

**Step 3 — build & install on the plugged-in iPhone.** Either open the project in Xcode and **Run**
(▶) with the device selected as the destination, **or** build from the CLI:

```bash
# Plug in + unlock the iPhone and trust this Mac first.
# Replace <DEVICE_UDID> with the value from: xcrun xctrace list devices
cd apps/ios && xcodebuild -project DogTag.xcodeproj -scheme DogTag \
  -destination 'platform=iOS,id=<DEVICE_UDID>' build
```

After a CLI build, install the resulting `.app` onto the device (Xcode's **Run** does build+install
in one step, which is the simpler path for on-device debugging — prefer it if `xcodebuild` install
gives you trouble).

**Verify.** The app launches on the iPhone; on first use it prompts for **camera** access (QR
scanning) and **Face ID** (wallet/consent signing) — these are declared in `project.yml`
(`NSCameraUsageDescription`, `NSFaceIDUsageDescription`).

**STOP if** the build fails with a **code-signing / "no team" / "failed to register bundle
identifier"** error — the baked `DEVELOPMENT_TEAM` (`AYDBUX9433`) is not your team. **Team fix:** set
your **own** `DEVELOPMENT_TEAM` in `apps/ios/project.yml` (the `settings.base.DEVELOPMENT_TEAM` line),
then **re-run `xcodegen`** so it regenerates the project with your team:

```bash
# After editing DEVELOPMENT_TEAM in apps/ios/project.yml:
cd apps/ios && xcodegen
```

Editing the generated `DogTag.xcodeproj`/`.pbxproj` directly does **not** stick — the next `xcodegen`
overwrites it. The team id must live in `project.yml`.

---

## 6. Android — build & install on a device

Source-of-truth facts from `apps/android/app/build.gradle.kts`: `applicationId io.liberalize.dogtag`,
`compileSdk 36`, `minSdk 26`, `targetSdk 34`, ABIs `armeabi-v7a` + `arm64-v8a`, and `noCompress` for
`zkey`/`graph` (the prover reads them as on-disk paths, so they must not be compressed).

**Step 1 — ensure the SDK path is set** (from §1b):

```bash
# local.properties must point Gradle at the SDK. <SDK_DIR> = /Users/zhenhaowu/Library/Android/sdk here.
grep '^sdk.dir=' apps/android/local.properties || echo "sdk.dir=<SDK_DIR>" > apps/android/local.properties
```

**Step 2 — vendor the zkey** (if not already, §4):

```bash
cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/verification_final.zkey
```

**Step 3 — connect the phone and confirm adb sees it.** Enable **Developer options → USB debugging**
on the phone, plug it in, and accept the on-phone "Allow USB debugging?" prompt.

```bash
~/Library/Android/sdk/platform-tools/adb devices   # → "List of devices attached" + <serial>  device
```

**Verify.** `adb devices` lists exactly one line under the header ending in `device` (not
`unauthorized` / `offline`).

**STOP if** no device is listed — check the USB cable (data, not charge-only), that **USB debugging**
is enabled, and that you accepted the authorization prompt on the phone. If it shows `unauthorized`,
re-plug and accept the prompt; if `offline`, run `adb kill-server` then `adb devices` again.

**Step 4 — build the debug APK:**

```bash
cd apps/android && ./gradlew :app:assembleDebug
# → BUILD SUCCESSFUL; APK at app/build/outputs/apk/debug/app-debug.apk (~115 MB)
```

**Verify.** `ls -l apps/android/app/build/outputs/apk/debug/app-debug.apk` → a ~115 MB file.

**Step 5 — install on the device** (either Gradle's install task or `adb install`):

```bash
cd apps/android && ./gradlew :app:installDebug
# OR, equivalently:
# ~/Library/Android/sdk/platform-tools/adb install -r app/build/outputs/apk/debug/app-debug.apk
```

**Verify.** The app appears and launches on the phone.

**Reset app state** (fresh owner wallet / clear stored prefs incl. `prover_api`, `central_api`):

```bash
~/Library/Android/sdk/platform-tools/adb shell pm clear io.liberalize.dogtag
```

---

## 7. Set `prover_api` in-app (32-bit Android ONLY)

Only do this on a **32-bit-only** Android device (§3). 64-bit iPhones and arm64 Android ignore
`prover_api` entirely — leave it untouched there.

- **Where:** the app's **Settings** (the in-app config screen that surfaces `central_api` /
  `prover_api`; persisted in `SharedPrefs` under key `prover_api`, see `AppConfig.proverApiUrl`).
- **What to set it to:** the **base URL of a running prover-service** (no trailing slash) — i.e. the
  service exposing `POST /prove-verification`. In a LOCAL demo this is the `cloudflared` tunnel in
  front of the prover on port `41875`; for a remote setup it is your remote prover's TLS host. Use
  `<PROVER_TUNNEL_URL>`.

> **WARNING — the baked default is dead.** `AppConfig.DEFAULT_PROVER_API` is currently
> `https://vertical-emails-escape-speech.trycloudflare.com`, a **stale, long-dead trycloudflare
> tunnel**. A 32-bit device that relies on the default will fail to prove. **Always override
> `prover_api` in-app** to a live prover (or recompile `AppConfig.kt` with a current value).

To stand up / tunnel a prover-service and get a live URL, see
[TUNNELING — the prover tunnel](./TUNNELING.md) and
[REMOTE — run a prover-service §8](./REMOTE_DEPLOYMENT.md). (REMOTE does **not** start a
prover-service for you; a remote operator with 32-bit-Android users must run one themselves.)

**Verify.** On a 32-bit device, the groomer **Approve & present** step now produces a proof and the
verification reaches **Verified on-chain**.

**STOP if** it still fails — the URL is wrong/stale or the prover-service is down. Confirm the prover
answers (e.g. it is reachable and `POST /prove-verification` exists), re-tunnel if the URL changed
(trycloudflare URLs are ephemeral — they rotate each run and drop overnight), and re-enter the new
URL.

---

## 8. Rebuild on chain swap

There is **no sync script** that pushes contract config into the apps — **each app bundles its own
copy**, so a chain/contract swap means editing both apps and rebuilding. After you change the on-chain
deployment, do all of the following:

1. **Edit both `roax.json` files** to the new contract addresses:
   - `apps/ios/DogTag/roax.json`
   - `apps/android/app/src/main/assets/roax.json`
2. **If you are changing chains**, also update the baked **RPC constant** in both apps:
   - iOS `apps/ios/DogTag/Models.swift` → `AppConfig.roaxRpc`
   - Android `apps/android/app/src/main/java/io/liberalize/dogtag/data/AppConfig.kt` → `ROAX_RPC`
3. **Re-vendor the production zkey** into both bundles (§4) — a chain swap normally comes with a new
   trusted-setup `verification_final.zkey`:
   ```bash
   cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey
   cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/verification_final.zkey
   ```
4. **Rebuild + reinstall both apps** — iOS per §5, Android per §6.

Until you rebuild **and reinstall**, the phone keeps using the **old** baked addresses/RPC/zkey and
will silently talk to the previous chain. For the full go-live chain-swap checklist (backend, portal,
contracts, ceremony, timelock) see
[PRODUCTION — chain swap §2](./PRODUCTION_DEPLOYMENT.md).

---

## 9. Troubleshooting (mobile subset)

| symptom | likely cause | fix |
|---|---|---|
| iOS build fails: code-signing / "no team" / can't register bundle id | baked `DEVELOPMENT_TEAM AYDBUX9433` is not your team | set your own `DEVELOPMENT_TEAM` in `apps/ios/project.yml`, then re-run `xcodegen` (don't edit the generated project) — §5 |
| `adb devices` shows nothing / `unauthorized` / `offline` | USB debugging off, charge-only cable, or prompt not accepted | enable USB debugging, use a data cable, accept the on-phone prompt; `adb kill-server && adb devices` — §6 |
| 32-bit Android: groomer export fails to make a proof | `prover_api` blank or pointing at a dead tunnel | set `prover_api` to a **live** prover-service URL in-app — §7 |
| 32-bit Android still failing after setting `prover_api` | the baked `DEFAULT_PROVER_API` is a stale trycloudflare URL, or your tunnel rotated/expired | re-tunnel, re-enter the new URL (trycloudflare URLs are ephemeral) — §7, [TUNNELING](./TUNNELING.md) |
| app reaches the **wrong chain** / old contracts after a deploy | apps not rebuilt — `roax.json`/RPC are **baked** | edit both `roax.json` (+ RPC constant), re-vendor zkey, rebuild + **reinstall** — §8 |
| proofs never validate on a fresh checkout | `verification_final.zkey` not vendored (it's gitignored) | copy it into both bundles — §4 |
| app talks to an unexpected vet/groomer host | the host comes **only** from the scanned QR; a stale/wrong QR was scanned | re-scan the correct `/p/` or `/x/` QR from the right portal — §2 |
| stale wallet / stored prefs on Android | leftover app state | `adb shell pm clear io.liberalize.dogtag` — §6 |
