# MOBILE_BUILD — build & install the DogTag apps on real phones

**Goal / you'll end with:** the DogTag iOS app on a real iPhone and the DogTag Android app on a
real Android phone, each correctly configured to talk to the right vet/groomer hosts and the right
chain.

> **Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps.
> Run every command from the repo root (`/Users/zhenhaowu/code/dogtag-mono-repo`) unless a block
> `cd`s somewhere. This doc OWNS the **mobile endpoint-model table** (§2); the LOCAL, REMOTE, and
> PRODUCTION docs link here rather than copying it.

Placeholders used below (define-once):

- `<DEVICE_UDID>` — the iPhone's device id. Replace: `<DEVICE_UDID>` = `xcrun xctrace list devices`
  (or Xcode → Window → Devices and Simulators), copy the UDID of the plugged-in iPhone.
- `<SDK_DIR>` — the Android SDK path (`/Users/zhenhaowu/Library/Android/sdk` on this machine).

---

## 0. Goal + the one diagram

A phone gets its configuration from **two** distinct places. Knowing which is which is the whole
point of this doc - most "it's talking to the wrong thing" bugs are a confusion between them.

```
                         ┌──────────────────────────────────────────────┐
                         │                THE PHONE APP                 │
                         └──────────────────────────────────────────────┘
                                    ▲                    ▲
               SCANNED QR (per scan)│                    │BAKED (bundled in the build)
   ┌────────────────────────────────┘                    └────────────────────────────────────┐
   │  vet host     = QR  /p/<token>              contract addresses = bundled roax.json       │
   │  groomer host = QR  /x/<token>              chain RPC = baked constant                   │
   │  (the app has NO field for these)                       (https://devrpc.roax.net)        │
   │                                             zkey + graph = bundled assets                │
   │                                                          (vendored each build)           │
   └──────────────────────────────────────────────────────────────────────────────────────────┘
```

- **SCANNED QR** — the vet host (issue a dog tag) and the groomer host (export/verify) come **only**
  from the QR the operator's portal renders. The app has no UI field for either host. See §2.
- **BAKED** - contract addresses (`roax.json`), the chain RPC constant, and the owner-hidden consent
  proving artifacts (`consent_final.zkey` + `consent.graph`) are compiled/bundled into the app.
  To change any of them you **edit + rebuild + reinstall** (§8).

There is no in-app endpoint setting left at all.
The former `central_api` ECDSA-fallback preference and the former 32-bit `prover_api` preference are
retired along with the flows that used them (§3, §7); `AppConfig` in both apps now carries only the
baked RPC constant.

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

> **Running the JVM unit tests needs one more thing than building does.** Everything above is for
> `assembleDebug`, which is unchanged. `./gradlew test` additionally needs a **host** `cargo` (plain,
> not `cargo-ndk`): every Gradle `Test` task hard-`dependsOn` a
> `cargo build -p dogtag-standard-rs --features prover --lib`, so the parity tests can call the real
> Rust core over JNA — the `jniLibs/` `.so` are Android-ABI-only and can never load on a dev machine.
> This doc otherwise covers building + installing only; the test suites and their sharp edges are
> owned by AGENTS.md "Build & test".

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

**The vet and groomer hosts come ONLY from the scanned QR.** There is no settings field for them in
either app — whatever host the operator's portal encodes into the `/p/` or `/x/` QR is the host the
phone calls, and nothing else. Contract addresses and the RPC are baked; do not look for them in the
app's settings either.

That is the whole model: the retired `central_api` and `prover_api` preferences no longer exist in
either app (§7), so there is nothing else to configure on the phone.

Per-contract addresses live in `contracts/deployments/roax.json` (and a quick-reference table in
[DEPLOYMENT — address book](./DEPLOYMENT.md)). This doc never transcribes addresses.

---

## 3. Proving: 64-bit vs 32-bit

The Groth16 consent proof for a groomer verification (the owner-hidden export, `/x/` flow) is
generated **on the phone**, from the bundled `consent_final.zkey` + `consent.graph` (§4).

- **64-bit devices** - every iPhone, and any modern **arm64** Android - prove **on-device**.
  Nothing to configure.
- **32-bit-only Android** - a device with no 64-bit ABI cannot run the on-device Groth16 prover,
  and **currently cannot complete a verification at all**.
  The retired remote `/prove-verification` fallback (and its in-app `prover_api` setting) is gone.
  Its replacement concept is the **consent server-prove fallback**: the backend `POST /prove-consent`
  route exists (a trusted-prover fallback - the prover sees the proof witness, see
  [MOBILE_OWNER_SECRET - Handling rules](./MOBILE_OWNER_SECRET.md#handling-rules)), but the app-side
  wiring for it lands with the mobile-issuance slice.
  Until that lands, use a 64-bit device.

---

## 4. Bundled assets (both apps)

Both apps bundle their own copies of the proving artifacts and a trimmed address file.
Each app needs **one** artifact set: the owner-hidden consent pair, `consent_final.zkey` +
`consent.graph` - the only artifacts the app code loads (`ZkeyAsset.swift` / `ZkeyAsset.kt`).
**Both are committed under `circuits/build/`** and are vendored into each bundle; the bundle copies
stay gitignored so the blobs are never double-committed.

| asset | iOS path | Android path | committed? |
|---|---|---|---|
| `consent_final.zkey` (~25 MB) | `apps/ios/DogTag/consent_final.zkey` | `apps/android/app/src/main/assets/consent_final.zkey` | zkey committed under `circuits/build/`; the bundle copy is gitignored — vendor it |
| `consent.graph` (~1.5 MB) | `apps/ios/DogTag/consent.graph` | `apps/android/app/src/main/assets/consent.graph` | graph committed under `circuits/build/`; the bundle copy is gitignored — vendor it |
| `roax.json` (hand-maintained subset) | `apps/ios/DogTag/roax.json` | `apps/android/app/src/main/assets/roax.json` | yes |
| `testvectors.json` | `apps/ios/DogTag/testvectors.json` | `apps/android/app/src/main/assets/testvectors.json` | yes |

Each bundle copy is a 1:1 copy of the file under `circuits/build/`.
Both sources are committed there, but their bundle copies are gitignored in `apps/.gitignore` (so the
blobs are never double-committed).
**A fresh checkout has none of the four bundle copies (2 files x 2 apps), and the apps will not
prove until you vendor them.** One command does all four:

```bash
make vendor-mobile-artifacts
```

It verifies `consent.graph` against its attested SHA-256 before copying, so an unattested graph is
refused rather than silently signed into a bundle. The equivalent by hand:

```bash
cp circuits/build/consent_final.zkey apps/ios/DogTag/consent_final.zkey
cp circuits/build/consent_final.zkey apps/android/app/src/main/assets/consent_final.zkey
cp circuits/build/consent.graph      apps/ios/DogTag/consent.graph
cp circuits/build/consent.graph      apps/android/app/src/main/assets/consent.graph
```

> **iOS resource wiring.** The committed generated `DogTag.xcodeproj` references the consent pair
> (`consent_final.zkey` + `consent.graph`) as bundle resources - the retired
> `verification_final.zkey`/`verification.graph` references are gone. Because the blobs themselves
> are gitignored, an app build on a fresh checkout fails loudly ("Build input file cannot be found")
> until you vendor them as above - that failure is the guard, not a project bug.
> The §5 flow regenerates the project with `xcodegen`, which sweeps `apps/ios/DogTag/` - so vendor
> the consent pair **before** running `xcodegen`, or the regenerated project silently omits it and
> the installed app cannot prove.
> Android has no such caveat: everything present in `assets/` is bundled.
> Stray copies of the retired verification pair in a working tree are dead weight (~68 MB) and safe
> to delete.

**Rebuilding the witness graph (rarely needed).** `circuits/build/consent.graph` is committed, so a
normal build never rebuilds it - vendor the committed bytes and move on.

It matters only if the frozen `circuits/consent.circom` ever changes. The `.graph` format is produced
by iden3's `build-circuit` binary (NOT the removed `npm run build-circuit` dev setup, and NOT in the
published `circom-witnesscalc` crate):

```bash
# iden3 build-circuit (install per its README); consumes the circom + circomlib includes.
build-circuit circuits/consent.circom circuits/build/consent.graph -l node_modules/circomlib/circuits -l circuits
```

The tool is **not byte-deterministic**, so a rebuild produces a different file even from identical
sources. That is precisely why the graph is committed rather than rebuilt per machine: the committed
bytes are the artifact, and `dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256` attests them.
A deliberate rebuild is an artifact **rotation** - the attested hash and the on-chain
`witnessMobileSha256` must move together. Follow
[ARTIFACT_PIN_RUNBOOK.md](./ARTIFACT_PIN_RUNBOOK.md); do not just overwrite the file.

**Verify.** All four bundle copies exist and are non-trivial in size.

```bash
ls -l apps/ios/DogTag/consent_final.zkey \
      apps/android/app/src/main/assets/consent_final.zkey
# → consent zkey ~25 MB (≈ 24781468 bytes, sha256 f83a111f…c868)
ls -l apps/ios/DogTag/consent.graph \
      apps/android/app/src/main/assets/consent.graph
# → consent.graph ~1.5 MB (the committed graph is 1546215 bytes,
#   sha256 2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793 — attested in-repo by
#   dogtag_prover::artifact::LEVEL_B_V1_WITNESS_GRAPH_SHA256. It is still unpinned ON-CHAIN
#   (witnessMobileSha256 == 0); see ARTIFACT_PIN_RUNBOOK.md for publishing it)
```

**STOP if** any path is missing or 0 bytes - `circuits/build/consent_final.zkey` or the graph is
absent, or the copy failed. Both sources are committed, so an absent one means an incomplete
checkout: restore with `git checkout -- circuits/build`, then re-run `make vendor-mobile-artifacts`.

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
> link without it. Build the Rust prover static lib (`--features prover`), regenerate the Swift bindings,
> and assemble the framework; for a **device** install you need **both** the `aarch64-apple-ios` device
> and `aarch64-apple-ios-sim` simulator slices (a Simulator-only xcframework fails to link on a device).
> The full copy-pasteable device sequence is in
> [`AGENTS.md` → Building the mobile (iOS) holder app](../AGENTS.md); the Simulator-only variant (and the
> on-device ZK self-test it powers) is in [`AGENTS.md` → Mobile end-to-end testing (iOS)](../AGENTS.md).

**Step 1 - vendor the proving artifacts** (the consent pair - if you have not already, §4).
This MUST happen **before** Step 2: `xcodegen` only wires resources that exist in the folder at
generation time (see the §4 caveat).

```bash
cp circuits/build/consent_final.zkey apps/ios/DogTag/consent_final.zkey
cp circuits/build/consent.graph      apps/ios/DogTag/consent.graph
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

**Step 2 - vendor the proving artifacts** (the consent pair - if not already, §4):

```bash
cp circuits/build/consent_final.zkey apps/android/app/src/main/assets/consent_final.zkey
cp circuits/build/consent.graph      apps/android/app/src/main/assets/consent.graph
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

**Reset app state** (fresh owner wallet / clear all stored app state):

```bash
~/Library/Android/sdk/platform-tools/adb shell pm clear io.liberalize.dogtag
```

> **DESTRUCTIVE for issued tags - this is not just a prefs reset.** `pm clear` wipes the package's
> whole internal storage, which includes `noBackupFilesDir` and therefore the owner-secret store
> `dogtag-owner-secrets.json.enc` (see [MOBILE_OWNER_SECRET](./MOBILE_OWNER_SECRET.md)). The
> owner-secret itself is seed-derivable, but each tag's **attribute values and salts are not** — they
> exist nowhere else on the device, and without them `R` cannot be rebuilt, so every tag on
> that phone becomes permanently unprovable. `profileRoot` is write-once on-chain, so there is no
> repair; the remedy is a fresh issuance under a new `dogTagId` (D3), which Android does not yet
> implement. Safe on a dev phone holding no tags. Before clearing one that does, re-obtain
> each credential from its issuer so the attribute leaves can be restored.

---

## 7. Set `prover_api` in-app (32-bit Android ONLY)

**Retired - there is nothing to set here any more.**
The in-app `prover_api` preference and the remote `/prove-verification` prover-service it pointed at
are gone: `AppConfig` no longer carries a prover URL (or a `central_api`), and the app has no
settings screen for endpoints.
On every 64-bit device the app proves on-device with no configuration (§3).
The replacement concept for a device that cannot prove locally is the **consent server-prove
fallback** - the backend `POST /prove-consent` route exists, but its mobile wiring lands with the
mobile-issuance slice.
Until that lands, a 32-bit-only Android cannot prove; see §3.

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
   trusted-setup `consent_final.zkey` (and, if the circuit changed, a rebuilt `consent.graph`):
   ```bash
   cp circuits/build/consent_final.zkey apps/ios/DogTag/consent_final.zkey
   cp circuits/build/consent_final.zkey apps/android/app/src/main/assets/consent_final.zkey
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
| 32-bit-only Android: export cannot produce a proof | the device cannot run the on-device prover, and the consent server-prove fallback is not wired into the app yet | use a 64-bit device; the mobile fallback wiring lands with the mobile-issuance slice - §3 |
| app reaches the **wrong chain** / old contracts after a deploy | apps not rebuilt — `roax.json`/RPC are **baked** | edit both `roax.json` (+ RPC constant), re-vendor zkey, rebuild + **reinstall** — §8 |
| proofs never validate on a fresh checkout | `consent_final.zkey`/`consent.graph` not vendored (the bundle copies are gitignored) | `make vendor-mobile-artifacts` - §4 |
| iOS app builds but cannot prove | consent pair vendored **after** `xcodegen`, so the regenerated project never bundled it | vendor the pair, re-run `xcodegen`, rebuild - §4 caveat, §5 |
| app talks to an unexpected vet/groomer host | the host comes **only** from the scanned QR; a stale/wrong QR was scanned | re-scan the correct `/p/` or `/x/` QR from the right portal — §2 |
| stale wallet / stored prefs on Android | leftover app state | `adb shell pm clear io.liberalize.dogtag` - §6. **Also destroys the owner-secret store, stranding every tag on that phone**; read the warning in §6 first |
