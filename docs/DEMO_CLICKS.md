# DogTag - from a bare machine to every use case

This is the one file. It has two phases and you do them in order.

| | | |
|---|---|---|
| **PHASE 1 - SETUP** | §0 - §7 | Stand up every role, one at a time, in dependency order. Nothing is issued or verified here; you are building the system. |
| **PHASE 2 - USE CASES** | §8 - §13 | Six complete journeys on the system you just built. Each one starts and finishes in a single section. |

Every step in both phases is three things and nothing else:

- **who** is acting - the section heading names the hat,
- **do** - the command or the click,
- **means** - what just happened, in a sentence or two.

There is no faster path offered anywhere in this file. Every procedure is written out where it is
needed, including the ones that repeat, because the repetition is the lesson: **every contract goes
through the same handshake, and a reader who is told to "do that part again" learns a shape the
product does not have.**

## The one rule this product is built on

**Each provider deploys and owns its own issuing contract. DogTag cannot do it for them.**
The registrar admits a provider and authorises it; the provider deploys. Neither can issue alone.

That is why setup goes ADMIN → PROVIDER → ADMIN → PROVIDER before a single credential exists. It is
a handshake between two parties, not a checklist for one.

| Role | Who this really is | What they do | § |
|---|---|---|---|
| **OPERATOR** | DogTag, once | Installs the toolchain, establishes the contract set | 0 |
| **ADMIN / REGISTRAR** | DogTag's governance key | Registers a provider, approves it, attaches its contracts, grants rights | 1, 4 |
| **PROVIDER** | the vet / groomer / government business, on **their own** machine and wallet | **Deploys and owns its own issuing contracts**, selects them, admits its own signing keys | 3, 5, 6, 7 |
| **HOLDER** | the pet owner | Holds credentials on their phone, consents to verification | 2, 12 |

On one laptop you play all of them. In production they are different organisations on different
machines holding different keys.

## Where else to look

| If you want | Read |
|---|---|
| **to see the product work** | **this file** |
| to install the toolchain | [PREREQUISITES.md](./PREREQUISITES.md) |
| every service, port and flag | [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md) |
| your own server, domains, TLS | [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) |
| the phone apps in depth | [MOBILE_BUILD.md](./MOBILE_BUILD.md) |
| the deploy and publish runbooks | [DEPLOY.md](./DEPLOY.md), [PROTOCOL_REGISTRY_RUNBOOK.md](./PROTOCOL_REGISTRY_RUNBOOK.md) |

**No contract address is written into this guide as configuration.** Every one is resolved from
`contracts/deployments/roax.json`, which the deploy writes.

**§16 records what was walked, when, and against which commit**, quoting a few addresses in
shortened form as the historical record. A step marked unwalked says so with its reason -
could-not-check is never reported as pass.

---

---

# PHASE 1 - SETUP

Eight sections, §0 to §7. Do them in this order; each one depends on the one before it.

---

# 0. OPERATOR - prepare the machine and establish the contracts

## 0.1 Get the code and the toolchain

**Do:** install these, then clone the repo. Every command in this guide runs from its root.

| | Checked with |
|---|---|
| Rust | `cargo --version` |
| Node 22 + pnpm 10 | `node --version && pnpm --version` |
| Foundry (`forge`/`cast`) | `forge --version` |

```bash
git clone <this repo> dogtag-mono-repo && cd dogtag-mono-repo
git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts
pnpm install --frozen-lockfile
```

**Means:** you have the source and its dependencies. The Foundry libraries are git submodules and a
fresh clone has them empty, so the first `forge` or `cast` command fails on the remappings until you
run that line. [PREREQUISITES.md](./PREREQUISITES.md) owns the install matrix if any of the three is
missing.

## 0.2 Choose your contract set: Option A or Option B

**Do:** decide now, before reading further. The rest of §0 has two versions and you follow exactly
one of them.

| | | Go to |
|---|---|---|
| **OPTION A** | **Use the contract set already deployed on ROAX.** Nothing is deployed; you confirm what is live and point your machine at it. This is the shorter route and the one to take unless you specifically need your own set. | **§0.3** |
| **OPTION B** | **Deploy the whole set from scratch.** Nine contracts, the registrar wiring, and the protocol version published so phones can resolve it. Take this when you are standing up your own chain or your own protocol instance. | **§0.4** |

**Means:** the two options differ only in where the nine protocol contracts come from. Both end at
the same place - a `contracts/deployments/roax.json` naming nine live contracts, which is the only
file in this repo that holds an address - and both rejoin the single path at §0.5. Option A reads a
ledger that already exists; Option B writes a new one.

## 0.3 OPTION A - confirm the deployed set

> Skip to §0.5 if you took Option B.

**Do:**

```bash
source scripts/lib/ledger.sh
RPC=https://devrpc.roax.net
for k in ProviderRegistry DogTagIssuerImpl DogTagIssuerFactory DogTagSBTConsent \
         Groth16VerifierConsent VerificationRegistryConsent ProviderDirectory \
         ServiceDomainResolver ProtocolRegistry; do
  a=$(ledger_addr $k); c=$(cast code $a --rpc-url $RPC)
  printf '%-30s %s %6d bytes\n' "$k" "$a" $(( (${#c} - 2) / 2 ))
done
cast call "$(ledger_addr ProtocolRegistry)" "getDiscoverySet(bytes32)" \
  "$(cast keccak 'dogtag-levelb/1')" --rpc-url $RPC
```

**Means:** all nine protocol contracts answer with code, and the protocol version is published and
active - the last word of that nine-word record is `1`. You are not deploying anything; the set is
live on ROAX and this only confirms it. Go to §0.5.

## 0.4 OPTION B - deploy the whole set from scratch

> Skip to §0.5 if you took Option A.

> **This section was walked on an `anvil --fork-url` fork of ROAX, not against live ROAX**, and the
> reason is worth stating: re-broadcasting to live ROAX would replace the working set - and with it
> the provider that makes the rest of this guide walkable - for no new evidence. On a fork every
> command below genuinely executes against real chain state, so the sequence, the wiring and the
> publish are proven rather than described. §16.2 records exactly what ran.

### 0.4.1 Choose the two keys

**Do:** decide two addresses and hold their keys. They must be different.

| | What it becomes |
|---|---|
| `ADMIN` | the registrar: owns `ProviderRegistry`, admin of the SBT and the registries, and by default the protocol publisher. This is the key that must sign the deploy. |
| `CUSTODIAN` | the neutral holder every dog tag is minted to. It never signs anything. |

**Means:** the deploy refuses if either is the zero address or if they are equal. The custodian must
not be the authority, because `ownerOf` would then return a key that signs - exactly the linkage the
custodial mint exists to remove.

### 0.4.2 Deploy the nine contracts

**Do:**

```bash
export ADMIN=<the registrar address>
export CUSTODIAN=<the neutral custodian address>
export PUBLISH_TIMELOCK_SECS=0        # testnet only; see below
export TESTNET_DEPLOY=true            # required for any value but 2 days
( cd contracts && forge script script/Deploy.s.sol:Deploy \
    --rpc-url <your rpc> --private-key <ADMIN's key> --broadcast --legacy )
```

**Means:** twelve transactions - nine contract creations plus three registrar wiring calls - and the
script writes every resulting address into `contracts/deployments/roax.json`. That ledger is the only
place an address lives; nothing else in this repo transcribes one.

**The order is forced by immutable references, not by preference.** `DogTagIssuerFactory` pins the
issuer implementation and the provider registry at construction, and
`VerificationRegistryConsent` pins that factory as its `rootIndex`. So the sequence must be
`ProviderRegistry` → `DogTagIssuer` (implementation) → `DogTagIssuerFactory` → `DogTagSBTConsent` →
`Groth16VerifierConsent` → `VerificationRegistryConsent` → the two typed resolvers →
`ProtocolRegistry`. That is why replacing one contract cascades into replacing several.

> **`PUBLISH_TIMELOCK_SECS` defaults to 2 days and the script refuses any other value unless
> `TESTNET_DEPLOY=true`.** The timelock is the window in which somebody can notice a protocol
> repoint before it takes effect; a zero lets the publisher key repoint the entire declared protocol
> set in one transaction. Use `0` on a development chain so you can deploy, publish and iterate in
> one sitting. Never set it on a production deployment.

### 0.4.3 Publish the protocol version, phase 1 - propose

**Do:** the four artifact hashes below are the committed proving artifacts. Compute them rather than
copying them, so a mismatch is caught here:

```bash
source scripts/lib/ledger.sh
export PUBLISH_PROTOCOL_REGISTRY=$(ledger_addr ProtocolRegistry)
export PUBLISH_FACTORY=$(ledger_addr DogTagIssuerFactory)
export PUBLISH_VERIFICATION_REGISTRY=$(ledger_addr VerificationRegistryConsent)
export PUBLISH_SBT=$(ledger_addr DogTagSBTConsent)
export PUBLISH_VERIFIER=$(ledger_addr Groth16VerifierConsent)
export PUBLISH_PROVIDER_REGISTRY=$(ledger_addr ProviderRegistry)
export PUBLISH_ZKEY_SHA256=0x$(shasum -a 256 circuits/build/consent_final.zkey | cut -d' ' -f1)
export PUBLISH_WITNESS_MOBILE_SHA256=0x$(shasum -a 256 circuits/build/consent.graph | cut -d' ' -f1)
export PUBLISH_R1CS_SHA256=0x$(shasum -a 256 circuits/build/consent.r1cs | cut -d' ' -f1)
export PUBLISH_WASM_SHA256=0x$(shasum -a 256 circuits/build/consent_js/consent.wasm | cut -d' ' -f1)
export PUBLISH_ARTIFACTS_URL=<where you serve the proving artifacts>
export PUBLISH_MIN_APP_VERSION=1.4.0

( cd contracts && forge script \
    script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose \
    --rpc-url <your rpc> --private-key <the publisher key> --broadcast --legacy )
```

**Means:** the version `dogtag-levelb/1` is now staged on two independent axes - the on-chain
contract set and the off-chain proving artifacts - plus the binding between them. Nothing is active
yet. A preflight ran first and refused to stage anything unless the five addresses genuinely agree
with what the deployed contracts say about each other.

### 0.4.4 Publish the protocol version, phase 2 - execute

**Do:** with the same environment still exported, after the timelock has elapsed:

```bash
( cd contracts && forge script \
    script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute \
    --rpc-url <your rpc> --private-key <the publisher key> --broadcast --legacy )
```

**Means:** it prints `dogtag-levelb/1 active true` and the version is live. Phase 2 re-runs the
preflight and additionally checks that the staged bytes still match this environment, because the
one mutable member - the verifier - can be swapped inside the publish window.

> **Phase 2 needs the SAME environment phase 1 had**, not just the registry address. On a production
> deployment that is two days later, plausibly in a different shell: load the same file rather than
> re-typing.

### 0.4.5 Confirm the set answers

**Do:**

```bash
source scripts/lib/ledger.sh
cast call "$(ledger_addr ProtocolRegistry)" "getDiscoverySet(bytes32)" \
  "$(cast keccak 'dogtag-levelb/1')" --rpc-url <your rpc>
```

**Means:** a nine-word record whose last word is `1`. This is the record every phone resolves at
runtime to learn the factory, the verification registry, the SBT, the verifier and the provider
registry. Until it reads `1`, no phone can complete a verification. Go to §0.5.

## 0.5 Write `contracts/.env`

**Do:** create `contracts/.env` with exactly this:

```
ROAX_RPC=https://devrpc.roax.net
DEMO_MODE=1
GOVERNANCE_PRIVATE_KEY=<the registrar key - see below>
```

**Means:** the boot script sources this file, so it is where the two things the ledger cannot answer
live - which chain to talk to, and the registrar's key. `DEMO_MODE=1` is not cosmetic: without it
the backends refuse to boot on the dev passwords.

**`GOVERNANCE_PRIVATE_KEY` is the key that holds the registrar authorities on your contract set.**
Under Option B it is the `ADMIN` key you chose in §0.4.1. Under Option A it is held by whoever
operates DogTag, is not in this repository and never will be, and
`contracts/deployments/roax.json` records only its address, under `admin`. Confirm you have the
right one either way:

```bash
source contracts/.env
source scripts/lib/ledger.sh
cast wallet address --private-key "$GOVERNANCE_PRIVATE_KEY"   # must equal:
ledger_addr admin
```

**Without it you cannot complete §1 or §4**, and the admin role refuses to boot rather than start a
portal whose every action would silently produce unsigned calldata instead of a transaction.

> Put **no contract address in this file** yet. Every protocol address is resolved from the ledger
> by name, and an address here silently overrides it - which is how a stale one survives a redeploy
> while naming a contract that decides nothing. Four variables get added later, in §5.4 and §7.5,
> and they are the only ones that ever belong here.

## 0.6 How you boot a role

**Do:** nothing yet - this is the shape every boot in this phase takes.

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh <role>     # macOS; use your own LAN address
scripts/demo-down.sh <role>                                    # stop just that one
scripts/demo-down.sh                                           # stop everything
```

Roles: `admin` `vet` `groomer` `government` `owner` `prover` `indexer`.

**Means:** each role starts and stops on its own, records its own process ids, and refuses to start
on a port something already holds. Booting one role never disturbs another.

> **Set `LAN_IP`.** It is stamped into every QR the stack mints, and a phone cannot reach
> `localhost` - that is the phone itself. Its default is a hardcoded address that is almost
> certainly not yours.

---

# 1. ADMIN - boot the registrar and register the provider

> In production this is DogTag's governance key, at DogTag. It admits providers to the protocol. It
> does not deploy their contracts and cannot admit a signing key to one.

## 1.1 Boot the admin role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh admin
```

**Means:** `admin-api` on `:39742` and the admin portal on `:39741`. The preflight checks the chain
id and that your governance key holds the registrar authority; if it does not, it refuses here
rather than booting a portal whose every action would silently produce unsigned calldata instead of
a transaction.

## 1.2 Sign in

**Do:** open http://localhost:39741 and click **Sign in** (the password is prefilled in demo mode).

**Means:** you land on the Dashboard. Its business counters read 0 until you register someone.

## 1.3 Register the provider

**Do:** **Providers** in the left nav. Read the banner first - it must say *"registrar actions
execute directly"*. Then **Register provider** → **Fill demo data** → **Review** → **Register**.

> If the banner instead says actions route to governance as proposals, your `GOVERNANCE_PRIVATE_KEY`
> is not the registrar key (§0.5). Nothing is broken, but nothing below will reach the chain either -
> each action returns unsigned calldata for someone else to execute. Fix the key and reboot the role.

**Means:** the provider now exists on chain with standing PENDING. `Fill demo data` mints a random
provider id, sets the controller to the published anvil test account whose key §3.3 gives you, and
writes an obviously-fake identity statement; **Review** hashes that statement, and only the hash
reaches the chain.

> **Copy the provider id.** §3 needs it, and it can never be reassigned. The statement text is
> stored nowhere and disappears when you navigate away.

## 1.4 Activate it

**Do:** the new row reads **pending**. Click **Activate**.

**Means:** standing goes PENDING → ACTIVE. Until it does, the provider is inert: every self-service
action refuses, so registering alone grants nothing.

## 1.5 Approve the record types it may create

**Do:** on the same row, click **DOG_PROFILE**. Then click **VACCINATION**.

**Means:** the registrar has said which kinds of contract this provider may deploy. It is per record
type - approving one approves no other. The vet needs both: DOG_PROFILE anchors dog tags,
VACCINATION anchors vaccination records.

---

# 2. HOLDER - build and install the pet-owner phone app

> **Switch hats.** You are the pet owner. The phone app is not started by `demo-up.sh`: it carries a
> compiled Rust core and bundled proving artifacts, so it is built and installed like any other
> mobile app.

**This step comes here, before any provider exists, on purpose.** The app is compile-time configured
from the deploy ledger, so it must be built after §0 established the contract set. It does **not**
need to wait for a vet, and §2.1 states the proven property that says why.

## 2.1 The property that decides when you rebuild

**Do:** read this before building, because it is the question every reader has.

**Once the contract set is established, the app is built ONCE. No subsequent vet, groomer or
government onboarding requires a rebuild or a reinstall of any handset.**

**Means:** the app bundles four protocol addresses - `ProtocolRegistry`, `ProviderRegistry`,
`DogTagIssuerFactory`, `DogTagSBTConsent` - and **not one of them is provider-specific**. A
provider's own issuing contract is never bundled and was never known to the app: it is discovered at
runtime by asking the bundled factory `rootIssuer(root)`, which is a write-once on-chain index that
every new contract adds itself to when it anchors. So a vet that joins tomorrow is reachable by a
phone built today.

This was proven, not reasoned: a real contract was deployed and a credential anchored through it
**after** a handset was built, and the phone's entire read chain then resolved it using only the
four bundled addresses. §16.1 records the transactions and the reads.

> **What DOES require a rebuild: replacing the protocol set itself** - that is, running §0.4 again.
> Every full redeploy ends in a mobile rebuild and reinstall. That is a rare operator event, not
> something a provider joining can trigger.

## 2.2 Generate the address bundle and vendor the proving artifacts

**Do:**

```bash
make gen-mobile-config          # writes apps/*/roax.json from the deploy ledger
make vendor-mobile-artifacts    # copies the consent zkey + witness graph into both app bundles
```

**Means:** `gen-mobile-config` projects the ledger onto both app bundles, so no address is ever
typed by hand; `vendor-mobile-artifacts` copies the proving key and witness graph the on-device
zero-knowledge prover needs, checking both against their attested hashes first. Every file they
write - three per platform - is gitignored, so a fresh checkout has none of them and both apps
refuse to build until you run these. **That refusal is the guard, not a project bug.**

> **Run these BEFORE `xcodegen`.** It sweeps `apps/ios/DogTag/`, so regenerating the Xcode project
> first silently drops these files from the bundle's resource list and you ship an app that cannot
> read an address or generate a proof.

## 2.3 iOS - build the native core, then the app

> **You need**, beyond §0.1: a Mac with Xcode installed, `xcodegen` (`brew install xcodegen`), an
> Apple Developer team id, and an iPhone you can plug in and unlock. Do §2.4 instead if you are
> building for Android.

**Do:** build the Rust core for both the device and the simulator, regenerate the Swift bindings,
and assemble the framework:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios     --lib
cargo build -p dogtag-standard-rs --features prover --release --target aarch64-apple-ios-sim --lib
cargo build -p dogtag-standard-rs --features prover --release --lib

gen=$(mktemp -d); cargo run --features uniffi/cli --release --bin uniffi-bindgen -- \
  generate --library target/release/libdogtag_standard.dylib --language swift --out-dir "$gen"
cp "$gen/dogtag_standard.swift" apps/ios/DogTag/dogtag_standard.swift
hdr=$(mktemp -d); cp "$gen/dogtag_standardFFI.h" "$hdr/"
cp "$gen/dogtag_standardFFI.modulemap" "$hdr/module.modulemap"

rm -rf apps/ios/DogTagFFI.xcframework
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libdogtag_standard.a     -headers "$hdr" \
  -library target/aarch64-apple-ios-sim/release/libdogtag_standard.a -headers "$hdr" \
  -output apps/ios/DogTagFFI.xcframework
```

**Means:** `DogTagFFI.xcframework` now carries the credential crypto and the on-device Groth16
prover for both a real iPhone and the simulator. `--features prover` is mandatory: without it the
proving entry point is absent and the app will not link.

**Do:** set your Apple team, generate the project, then build and install:

```bash
# edit apps/ios/project.yml -> settings.base.DEVELOPMENT_TEAM: <YOUR_TEAM_ID>
( cd apps/ios && xcodegen )
open apps/ios/DogTag.xcodeproj   # select the DogTag scheme + your device, press Run
```

**Means:** Xcode builds, signs, installs and launches on the plugged-in iPhone. Edit the team in
`project.yml` and not in the generated project - regenerating overwrites the latter. If the phone
says **Untrusted Developer**, trust your team under **Settings → General → VPN & Device Management**
and relaunch.

## 2.4 Android - build the native core, then the app

> **You need**, beyond §0.1: the Android SDK and NDK, `cargo install cargo-ndk`, `adb`, and a
> connected 64-bit ARM device or emulator. Do §2.3 instead if you are building for iOS.

**Do:**

```bash
export ANDROID_HOME=~/Library/Android/sdk          # macOS default; use your own path
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<your ndk version>
echo "sdk.dir=$ANDROID_HOME" > apps/android/local.properties

cargo ndk -t arm64-v8a -t armeabi-v7a -o apps/android/app/src/main/jniLibs \
  build --release -p dogtag-standard-rs --features prover

( cd apps/android && gradle :app:assembleDebug )
adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk
```

**Means:** the same Rust core, cross-compiled for ARM, plus the APK installed on a connected device.
Gradle does not run `cargo` for you, so the `cargo ndk` line is not optional.

> **A 64-bit ARM device or emulator is required.** The prover ships only as `arm64-v8a` /
> `armeabi-v7a`, so an x86_64 emulator cannot load it.

## 2.5 Confirm the app resolved the set

**Do:** open the app and go to the **Profile** tab.

**Means:** it names the `ProtocolRegistry` it was built with, alongside the provider registry, the
SBT and the chain. Those must match `contracts/deployments/roax.json`. If they do not, the bundle
was generated from a different ledger - re-run §2.2 and rebuild.

---

# 3. VET - deploy your own issuing contracts

> **Switch hats.** You are now the veterinary business. In production you are a different company on
> your own machine with your own wallet. `createIssuer` takes no owner argument, so whoever deploys
> the contract owns it - which is why DogTag cannot do this step for you.

**The provider's self-service page is served by the vet portal**, so you boot the vet role to act as
the provider. On a real deployment the business runs that portal itself; here it is the same laptop.

## 3.1 Boot the vet role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh vet
```

**Means:** `vet-api` on `:41874` and the vet portal on `:41873`. It warns that two contract
addresses are unset and boots anyway - those are the contracts you are about to deploy, so nothing
else could be true yet. Issuing refuses until §5.4 sets them; everything in this section works
without them.

## 3.2 Sign in and create the shop's signing key

**Do:** open http://localhost:41873 and click **Sign in** (prefilled in demo mode). The top of the
page says *Custody is locked*.

- **First run on this machine?** Go to **Setup**, sign in with the admin password (also prefilled),
  and follow the wizard: **genesis** → confirm → unlock. It shows you 24 words once, makes you tick
  that you wrote them down, then asks you to re-type three of them and choose an encryption
  passphrase - so have somewhere to write before you press **Generate 24-word seed**.
- **Custody already exists here?** Click **Unlock**, then **Unlock and continue** - both fields are
  prefilled.

Then open **Signing keys** in the nav and **copy the address in the line "This shop signs with …".**

**Means:** the backend now holds a key it can sign with. The sealed key is stored on disk and the
decrypted one never is, which is why custody re-locks on every restart of this backend.

That page also says *"No issuing contract is configured on this deployment"*, which is correct right
now - you have not deployed one yet. It fills in after §5.4.

> Do this before deploying, even though deploying does not need it: §4.4 has to grant a right to
> that address, and the address does not exist until the key does.

## 3.3 Connect your wallet

**Do:** you need a browser wallet holding the controller address from §1.3. Install one (MetaMask,
Rabby - any EIP-6963 wallet), add ROAX as a custom network, and import the controller's key:

| | |
|---|---|
| Network name | ROAX |
| RPC URL | `https://devrpc.roax.net` |
| Chain ID | `135` |
| Controller key | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |

Then in the vet portal: **Provider self-service** in the nav → **Connect wallet** at the **top right
of the page header** (not in the page itself).

**Means:** this page has no backend, so every action *and every read-only check* is signed by you.
Every control stays disabled until a wallet is connected, and says so.

> That key is the published anvil test account - printed by every `anvil` start, and what
> `Fill demo data` put in the controller field in §1.3 precisely so anyone can act as this provider
> on a disposable testnet. It is public by design and holds testnet funds only. Never put a real key
> in a browser.

## 3.4 Deploy the DOG_PROFILE contract

**Do:** paste your provider id, set **Record type** to `DOG_PROFILE`, leave **Contract number** at
`0`, click **Check what this would deploy**, then **Deploy** and confirm in your wallet.

**Means:** you own a `DogTagIssuer` contract for dog tags. The check shows the exact address before
anything is sent, because the address is computed from the record type, your wallet and that number -
so **Deploy** only ever sends what a check approved, and goes back to disabled if you edit a field.

> **If the check refuses** with *"that one number's address is simply taken"*, this record type has
> already been deployed from this wallet. Each number gives one fixed address and two contracts
> cannot share one, so raise **Contract number** to `1` and check again.

> **Copy the deployed address.** §4.2, §4.3, §4.4, §5.1, §5.2 and §5.4 all need it.

## 3.5 Deploy the VACCINATION contract

**Do:** on the same page, set **Record type** to `VACCINATION`, leave **Contract number** at `0`,
click **Check what this would deploy**, then **Deploy** and confirm in your wallet.

**Means:** you now own a second contract, for vaccination records. The same contract number gives a
different address because the record type is part of the computation, so `0` is correct here even
though you just used it.

> **Copy this address too.** It is a different contract and goes through its own attach, activate,
> grant, select and admit below.

---

# 4. ADMIN - attach and authorise both contracts

> Back to the admin portal, still running from §1. Every action in this section is `onlyOwner` on
> the provider registry: no provider key can perform any of them.

## 4.1 Open the provider's services

**Do:** **Providers** → **Show services** on your provider's row.

**Means:** this is where a provider's contracts are admitted and managed. It is empty until you
attach the first one.

## 4.2 Attach and activate the DOG_PROFILE contract

**Do:** **Attach a contract**. Paste the DOG_PROFILE address from §3.4, **Check**, then **Attach**.
Then click **Activate** on the new service row.

**Means:** DogTag has admitted this contract to that provider's record, and moved it from PENDING to
ACTIVE. The address is the only thing you type: the check works out which factory deployed it and
reads its record type and owner off the chain. Attaching alone grants nothing - a service lands at
PENDING exactly as the provider did.

## 4.3 Attach and activate the VACCINATION contract

**Do:** **Attach a contract** again. Paste the VACCINATION address from §3.5, **Check**, then
**Attach**. Then click **Activate** on that row.

**Means:** the second contract is admitted and active. Every contract goes through this
independently - there is no bulk step, because each one is a separate on-chain record with its own
standing.

## 4.4 Grant the issue right to the vet's signing key

**Do:** click **Issuance capability** on either service row, enter the signer address from §3.2,
pick **Grant**, confirm.

**Means:** that address may now sign issuances. The grant is on the **address**, registry-wide - it
names no service, which is why you do this once and not per contract. That is deliberate, and it is
exactly why issuing needs a second permission that only the provider can give (§5.2 and §5.3).

## 4.5 Confirm the typed resolvers are approved

**Do:** in **Typed resolvers**, confirm both kinds show their address in the success style. Confirm
from the chain if you want certainty:

```bash
source scripts/lib/ledger.sh
PR=$(ledger_addr ProviderRegistry); RPC=https://devrpc.roax.net
cast call $PR 'isResolverApproved(uint8,address)(bool)' 0 "$(ledger_addr ProviderDirectory)"     --rpc-url $RPC
cast call $PR 'isResolverApproved(uint8,address)(bool)' 1 "$(ledger_addr ServiceDomainResolver)" --rpc-url $RPC
```

**Means:** the registrar has allowed these two resolvers fleet-wide, which is its half of the
provider's domain and listing flows. Under Option A both already read `true`; under Option B the
deploy wired them. The provider's half - selecting one - has no button yet (§15).

---

# 5. VET - select your contracts, admit your key, point the backend at them

## 5.1 Make the DOG_PROFILE contract current

**Do:** vet portal → **Provider self-service** → flow 2. Paste the DOG_PROFILE address from §3.4
into **Contract address**, **Check this contract**, then **Make this my current contract**.

**Means:** new dog tags now anchor here. This is the move that makes the chain's `canIssue` finally
answer true for this record type - before it, every other term held and this one did not.

## 5.2 Make the VACCINATION contract current

**Do:** on the same flow, paste the VACCINATION address from §3.5 into **Contract address**,
**Check this contract**, then **Make this my current contract**.

**Means:** the same for vaccination records. The pointer is per record type, so selecting one never
displaces the other.

## 5.3 Admit your signing key to both contracts

**Do:** **Signing keys** in the nav. Connect the wallet that **owns** the contracts, click **Use
this shop's signing key**, then **Admit** and confirm - once for the DOG_PROFILE contract and once
for the VACCINATION contract.

**Means:** issuing needs **two** permissions and this is the second. The registrar granted the right
in §4.4; this list is the contract owner's own, and the protocol admin is deliberately excluded from
writing it - so neither party can put a signer on somebody else's contract alone.

> A freshly deployed contract already admits whoever deployed it. The vet's backend signs with a
> *different* address - its custody signer - so this step is always needed for it, on every contract.

## 5.4 Tell the backend which contracts it anchors into

**Do:** add both addresses to `contracts/.env`, then restart just the vet:

```
PROFILE_ISSUER_ADDR=<the DOG_PROFILE contract from §3.4>
VACCINATION_ISSUER_ADDR=<the VACCINATION contract from §3.5>
```

```bash
scripts/demo-down.sh vet
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh vet
```

**Means:** the backend reads these at boot, so it learns where to anchor only on a restart. The boot
now names both contracts instead of warning about them.

---

# 6. GROOMER - boot the second business

> A groomer verifies and does not issue. Its backend is the same binary as the vet's with
> `BUSINESS_TYPE=groomer`, and the issuance routes are not mounted at all - so it has no Records
> page and no Issue page, and it needs no contract, no attach and no grant.

## 6.1 Boot the groomer role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh groomer
```

Then sign in at http://localhost:43617.

**Means:** a second, independent business on `:43617` / `:43618`, with its own store and its own
signing key. The vet is still running and untouched.

> **Leave custody locked.** The banner will say it is, and nothing this role does needs it: a
> groomer only ever reads the chain, and unlocking exists to let a backend *sign*. This is why the
> groomer's setup is one step and the vet's was five sections - a verifier needs no authority at all.

---

# 7. GOVERNMENT - boot the authority and give it its own contract

> A government authority is a provider like any other here. Nothing about it is privileged, and it
> goes through the identical handshake the vet did: approve the record type, deploy, attach,
> activate, grant, select.

> **On one laptop the authority reuses the provider you registered in §1**, adding a third record
> type to it, and deploys from the provider self-service page the vet portal serves - because that
> is the only surface in the product where a provider deploys, and the government portal has none.
> In production the authority would be its own registered provider, on its own machine, with its own
> wallet; you would then run §1.3 to §1.5 again for it before starting here. Nothing about the steps
> below changes either way.

## 7.1 Boot the government role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh government
```

**Means:** `government-api` on `:44832`, its portal on `:44831`. Its own binary, its own store. The
preflight tells you it can read the chain but not sign, which §7.5 fixes.

## 7.2 ADMIN - approve the TRAVEL_CLEARANCE record type

**Do:** in the admin portal, **Providers** → on the provider's row, click **TRAVEL_CLEARANCE**.

**Means:** the registrar has said this provider may deploy a travel-clearance contract. Approving
DOG_PROFILE and VACCINATION in §1.5 approved nothing else.

## 7.3 PROVIDER - deploy the TRAVEL_CLEARANCE contract

**Do:** vet portal → **Provider self-service** → flow 1, with the wallet from §3.3 connected. Paste
your provider id, set **Record type** to `TRAVEL_CLEARANCE`, leave **Contract number** at `0`, click
**Check what this would deploy**, then **Deploy** and confirm.

**Means:** the authority owns its issuing contract. It is deployed from the provider self-service
page because that page is where a provider deploys - the government portal has no deploy surface,
and the wallet that signs here is the one that will own the contract.

> **Copy the address.** §7.4 and §7.5 need it.

## 7.4 ADMIN - attach, activate and grant

**Do:** admin portal → **Providers** → **Show services** → **Attach a contract**. Paste the
TRAVEL_CLEARANCE address from §7.3, **Check**, **Attach**, then **Activate** on the new row. Then
**Issuance capability** on that row, enter **the address of the wallet that deployed it** (the
controller from §3.3, not the vet's signing key), pick **Grant**, confirm.

**Means:** the same three registrar actions every contract needs. The signer differs from the vet's
case because the government backend signs with the deploying wallet's own key, which §7.5 gives it.

## 7.5 PROVIDER - select it, then point the backend at it

**Do:** vet portal → **Provider self-service** → flow 2. Paste the TRAVEL_CLEARANCE address,
**Check this contract**, then **Make this my current contract**. Then add to `contracts/.env` and
restart the government role:

```
TRAVEL_CLEARANCE_ISSUER_ADDR=<the contract from §7.3>
GOV_SIGNER_KEY=<the private key of the wallet that deployed it>
```

```bash
scripts/demo-down.sh government
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh government
```

**Means:** `/health` flips to `canSign: true`. There is deliberately **no separate admit-your-key
step here**: a contract admits its own deployer automatically, and the government signs with exactly
that wallet, so the second permission is already satisfied. The vet needed §5.3 only because its
backend signs with a key it generated itself, which is a *different* address from the one that
deployed. Confirm either way:

```bash
cast call <the contract> 'issuanceAllowed(address)(bool)' <the signer> --rpc-url https://devrpc.roax.net
```

---

**Setup is complete.** Four roles are running, one provider owns three contracts, and a phone is
installed. Everything below is use.

---

---

# PHASE 2 - USE CASES

Six journeys. Each is complete in itself; do them in order the first time, because use case 2
produces the credential that use cases 3 to 6 all consume.

---

# 8. USE CASE 1 - issue a dog tag

> **Who:** the VET, and - for §8.3 alone - the owner with the phone from §2.

## 8.1 Unlock custody

**Do:** sign in at http://localhost:41873, click **Unlock** in the banner, then **Unlock and
continue**.

**Means:** the restart in §5.4 re-locked it. Nothing signs or issues until you unlock.

## 8.2 Register the pet and mint the QR

**Do:** **Register pet** in the nav → **Fill demo data** → **Start issuance**.

**Means:** a dog tag is allocated and the page shows a QR plus a URL on your `LAN_IP`. The tag now
exists and use case 2 can anchor records against it. What the QR is *for* is §8.3: the owner's
device scans it, folds the profile tree on the phone and posts only the resulting root - the vet
never holds the owner's secret, which is why binding a tag to its owner is the one step in this
guide a desktop cannot perform.

> **Note the dog tag id** (`1` on a fresh set). Use cases 2 and 3 both use it.

You can see exactly what the phone receives:

```bash
curl -s http://127.0.0.1:41874/p/<the token in that URL>
```

## 8.3 Scan it on the phone

**Do:** open the app from §2, scan the QR.

**Means:** the phone builds the profile tree locally, derives the owner secret from its own wallet
seed, and posts only the Merkle root. The dog tag is now bound to a device nobody else can act for.

> **This step has not been walked on a handset** - see §15. Everything up to the QR has, including
> what the phone receives. Use cases 2 to 6 do not depend on it, so a reader without a phone can
> continue from §9.

---

# 9. USE CASE 2 - issue a vaccination credential

> **Who:** the VET.

## 9.1 Issue the record

**Do:** **Issue a record** in the nav → **Fill demo data** → set **Dog tag id** to the one from §8.2
→ **Sign & Issue**.

**Means:** a rabies certificate is built, its Merkle root is anchored on chain by your VACCINATION
contract, and the page reports **Verified on-chain**. A record can be issued while the dog tag is
still waiting for its device bind - the two are anchored independently.

## 9.2 Take the credential out

**Do:** **Records** in the nav lists what this shop has issued, each with an explorer link. To carry
one elsewhere, fetch it. The bearer token is the value of `vet.opToken` in the portal's local
storage (DevTools → Application → Local Storage → `http://localhost:41873`):

```bash
TOKEN=<paste vet.opToken here>
curl -s http://127.0.0.1:41874/records -H "Authorization: Bearer $TOKEN" \
  | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["records"][0]["wrapped_doc"]))'
```

**Means:** that one JSON object *is* the credential - the field is `wrapped_doc`, snake_case, and
the line above pulls it out of the newest record. **Save it**: use cases 3, 4, 5 and 6 each paste a
copy of it.

---

# 10. USE CASE 3 - a second business verifies a credential it did not write

> **Who:** the GROOMER. This is the check that matters: the shop asserting the record is not the
> shop that wrote it, and the two share no database.

## 10.1 Create a client and their pet

**Do:** **Clients** → **New client** → **Fill demo data** → **Create client**.

**Means:** a client record with an inline pet. Every demo value says it is a demo - a receptionist
must never mistake a filled row for a real person.

## 10.2 Open the pet and link the DogTag

**Do:** **Pets** in the nav, click the pet, then under **DogTag** type the dog tag id from §8.2 and
click **Add DogTag**.

**Means:** the tag is linked, and the shop compares the microchip on its own pet record against the
microchip inside a credential *it holds* for that tag. It holds none - the vet's store is a
different business's - so it reports **Microchip not compared** rather than a pass. A mismatch would
refuse the link and would not change any credential's verdict: the credential is still genuine, it
just describes a different animal.

## 10.3 Verify the vet's credential

**Do:** **Ad-hoc verification** → paste the `wrapped_doc` from §9.2 into **Verify credential** →
**Verify credential**.

**Means:** **pass**, with five checks: integrity, on-chain valid, issued, not revoked, and issuer
authorised at issuance. No owner and no phone are involved - it recomputes the document's hash
offline and reads the issuing contract directly.

## 10.4 Book an appointment and publish the diary

**Do:** from the pet page click **Book appointment** → **Fill demo data** → set the **Pet** select
back to the pet → **Book appointment**. Then **Calendar** → **Calendar sync** → **Publish calendar
address**.

**Means:** the booking appears on the calendar, and the shop's diary is published at one secret URL
any calendar app can subscribe to. Treat that URL like a password - it is unauthenticated by
necessity, because a calendar client cannot present a token.

> `Fill demo data` clears the pre-selected pet. That is legal - an appointment need not name one -
> but set it back before booking.

---

# 11. USE CASE 4 - the authority verifies a credential, then issues its own

> **Who:** the GOVERNMENT.

## 11.1 Verify the vet's credential

**Do:** open http://localhost:44831, paste the `wrapped_doc` from §9.2 into **Verify a document
(paste JSON)**, click **Verify**.

**Means:** **✓ VALID**, with every read pinned to one block. The vet issued it, the government
verifies it, and the two share no database - only the chain.

The same check is available with no account at all:

```bash
curl -s -X POST http://127.0.0.1:44832/v1/verify -H 'content-type: application/json' \
  --data '{"wrapped_doc": <the document>}'
```

> *"Issuer authorised at issuance"* asks whether the signer held the right **at the block it
> anchored**, not now. Delisting is forward-only, so a vet that later rotates its key does not
> retroactively void every certificate it ever issued.

## 11.2 Issue a travel clearance

**Do:** in the government portal, **Fill demo data** → **Issue + anchor**.

**Means:** the credential is anchored on chain by the contract §7.3 deployed, and you get a
printable CDC-modelled receipt with a QR pointing at a public status page.

## 11.3 Open the public status page

**Do:**

```bash
curl -s http://127.0.0.1:44832/v1/receipts/<the receiptId>/status
```

**Means:** anyone can check that receipt without an account and without seeing the traveller. The
page carries the status, the record type, validity and the chain reads - and none of the Section A
person data the authenticated record holds.

---

# 12. USE CASE 5 - the owner receives the credential

> **Who:** the HOLDER. No operator, admin or provider can act for you.

## 12.1 Boot the owner wallet

**Do:**

```bash
scripts/demo-up.sh owner
```

**Means:** http://localhost:45931, and **no backend at all** - everything is local to the browser.
This is the only role that needs no chain preflight and no governance key. It is the desktop mirror
of the phone app you installed in §2.

## 12.2 Receive it

**Do:** **Receive**, paste the `wrapped_doc` from §9.2, click **Add to wallet**.

**Means:** the wallet shows **✓ Integrity intact** and **✓ Anchored on-chain**, then every field
that was hashed into the root. The integrity check is offline and the anchor check is a direct chain
read - no server was asked whether this credential is good.

---

# 13. USE CASE 6 - anyone verifies it, and tries to defeat it

> **Who:** ANYONE. Verification is permissionless: it reads the chain and needs no role. The bench
> lives in the admin portal only because that is where it was built.

## 13.1 Run the checks on a real credential

**Do:** admin portal → **Verification bench** → paste the `wrapped_doc` from §9.2 → **Run the
checks**.

**Means:** nine rows, each showing which contract was asked, what it answered and at which block.
Three of them are marked *not in the verdict*: the verifier's verdict covers integrity, on-chain
status and the issuer pillar, and nothing else. Expiry is reported beside it rather than folded in,
because the chain records anchoring and revocation and has no concept of a validity window.

## 13.2 Try to defeat it

**Do:** click the mutation buttons, then **Run the whole catalogue**.

**Means:** each mutation tells one specific lie with your record and re-runs everything, declaring
in advance what will *not* catch it - relabelling the issuer's name is caught by nothing on this
path, because the name is not covered by the Merkle root. The catalogue is eleven complete
fraudulent records with their own scripted chains, each declaring which check must refuse it. **One
of the eleven is a genuine credential that must pass** - without it, a verifier that refused
everything would look perfect.

---

# 14. Reference: reading the live state

```bash
source scripts/lib/ledger.sh
RPC=https://devrpc.roax.net
PR=$(ledger_addr ProviderRegistry)

cast call $PR 'providerCount()(uint256)' --rpc-url $RPC
cast call $PR 'serviceCount()(uint256)'  --rpc-url $RPC

# The two reads that decide whether anything can be issued - both must be true.
cast call $PR 'canIssue(address,address)(bool)' <contract> <signer> --rpc-url $RPC
cast call <contract> 'issuanceAllowed(address)(bool)' <signer> --rpc-url $RPC

# Is a contract the record type it claims?
cast call <contract> 'recordType()(bytes32)' --rpc-url $RPC
cast keccak VACCINATION

# Which contract issued a given credential? (this is how a phone finds it, too)
cast call "$(ledger_addr DogTagIssuerFactory)" 'rootIssuer(bytes32)(address)' <root> --rpc-url $RPC
```

Health, per role:

```bash
for p in 39742 41874 43618 44832 41875 46001; do printf "%-6s " $p; curl -s http://127.0.0.1:$p/health; echo; done
for p in 39741 41873 43617 44831 45931; do printf "%-6s %s\n" $p "$(curl -s -o /dev/null -w '%{http_code}' http://localhost:$p/)"; done
```

> Use `localhost` for the portals, not `127.0.0.1`. Vite dev servers bind IPv6 only, so a
> `127.0.0.1` probe reports a healthy portal as dead.

Two more roles exist that this guide never needs: `indexer` (the oversight event feed, which serves
the portals' Traceability and Oversight pages) and `prover` (a server-side consent prover for phones
that cannot prove locally).

---

# 15. What this guide does not cover, and what it covers but has not walked

**Written but NOT WALKED - the dog-tag device bind (§8.3).** The instruction is the real one and the
backend half of it was driven (the `/p/<token>` endpoint returns exactly what the phone receives),
but no bind has been completed from a handset on any recorded walk, including the device walk in
§16.3. Treat §8.3 as unverified rather than as proven, and note that use case 1 is the only journey
in Phase 2 that needs a phone at all - use cases 2 to 6 are complete without one.

**Completed owner-hidden consent proofs**, including the groomer's **Export on chain**. The phone
app is built and installed in §2 and the discovery anchor is proven to resolve on device (§16.3),
but no Groth16 consent proof has been driven end to end through a verifier.

**Selecting a typed resolver** (the provider's half of §4.5). Both are approved by the registrar;
the provider portal has no button for `setDomainResolver` / `setDirectoryResolver` yet, so flows 3
and 4 of the provider page cannot be completed.

**The microchip MATCH and MISMATCH outcomes** (§10.2). Only the neutral *not compared* state is
reachable, because the other two need the shop to hold a credential for that tag, which arrives
through **Import from user** - and that form wants a customer JWT a receptionist has no way to
obtain.

**The government's `EU_HEALTH_CERT` record type.** No contract for it exists on this deployment, so
`/health` reports it `null` and `/issue` dry-runs rather than anchoring somewhere wrong.

**`scripts/demo-provision-government.sh` does not work against this contract set** and §7 does not
use it. It calls `whitelistFor`, a three-argument `createIssuer`, and the factory's `owner()` - none
of which the launch set implements.

**The Playwright suites.** `make e2e-web` is the launchable entry point. Do not run the specs
directly and unfiltered: several are unmocked live-portal drivers that create real records and
anchor them on chain.

---

# 16. Evidence: what was walked, when, and against what

## 16.1 Build once: a contract deployed AFTER a handset resolves on it, 2026-08-06

Walked on **2026-08-06** against commit **`52175fe`** plus this branch, on the live ROAX set
(chainId 135). This is the evidence for the property §2.1 states.

**The question.** Does a vet, groomer or government onboarding after a phone is built make that
phone stale? If it did, every pet owner would have to update their app whenever a new vet joined.

**What the app actually reads, enumerated from the code.** `scripts/gen-mobile-roax-config.sh`
writes both bundles, and it writes exactly five keys: `chainId`, `ProtocolRegistry`,
`ProviderRegistry`, `DogTagIssuerFactory`, `DogTagSBTConsent`. Running it on this commit produced
byte-identical bundles on the two platforms, carrying those five keys and nothing else, each address
matching the ledger. **Not one of the four addresses is provider-specific.** A provider's own contract is found at runtime: `RecordImporter` (iOS) and its
Kotlin mirror resolve it with `DogTagIssuerFactory.rootIssuer(root)`, then read the record type, the
issuing signer and the governing authority **off the contract that answer names**, never off the
bundle.

**The empirical test.** The newest contract on this set had been deployed at block **339692**, and
the handset in §16.3 was built after that. A brand-new contract was therefore deployed now, and a
credential anchored through it:

| what | transaction | block |
|---|---|---|
| registrar approves a new record type | `0x372147a6…7b0a3ac4` | 352405 |
| **provider deploys its own contract** | `0x5aa2e7ce…6202ac38` | **352407** |
| registrar attaches it | `0x257c90f8…` | 352409 |
| registrar activates it | `0x5c4f945d…` | 352410 |
| provider makes it current | `0x03216a20…737947c6` | 352412 |
| **a root anchored through it** | `0xc0a2f1cc…42b669bf` | **352413** |

**The proof.** The phone's entire read chain was then reproduced using **only the four bundled
addresses** and nothing else:

| step | read | answer |
|---|---|---|
| 1 | `factory.rootIssuer(root)` | the new contract `0xAE05F415…40E5` |
| 2 | `factory.isClone(clone)` | `true` |
| 3 | `clone.recordType()` | matches `keccak256("GROOMING")` |
| 4 | `clone.issuedBy(root)` | `0xf39Fd6e5…2266` |
| 5 | `clone.isValid(root)` | `true` |
| 6 | `clone.registry()` | equals the bundled `ProviderRegistry` |
| 7 | grant fold at the anchoring point `(352413, 1)` | last `RightsSet` at block 340471 sets bit 0 → **AUTHORIZED** |

Every read resolved, against a contract that did not exist when the app was built. **So the answer
is yes: build once.** A provider onboarding cannot make a handset stale, because no provider address
is bundled and the index that finds them is.

**Stated precisely, because the obvious summary is wrong in a way that matters.** The app does *not*
bundle only the discovery anchor and resolve the rest at runtime - that narrowing is designed but
not yet built, and the bundle really does carry four addresses. The conclusion is unaffected, but
the reason is *"no provider-specific address is bundled"*, not *"only the anchor is bundled"*. The
one event that **does** require a rebuild and reinstall of every handset is replacing the protocol
set itself (§0.4), which is an operator action, not something a joining provider can cause.

**Nothing of the existing setup was disturbed**, checked before and after: the provider's
DOG_PROFILE, VACCINATION and TRAVEL_CLEARANCE pointers are byte-identical to what they were, and the
new record type had no contract at all before this walk, so nothing was displaced. The deploy ledger
and `contracts/.env` were not modified.

## 16.2 Option B, the from-scratch deploy, on a fork of ROAX, 2026-08-06

Walked on **2026-08-06** against commit **`52175fe`** plus this branch, on an
`anvil --fork-url https://devrpc.roax.net` fork (chainId 135, forked at block 352426, anvil killed
afterwards by recorded pid).

**Why a fork and not live ROAX.** Re-broadcasting the deploy to live ROAX would replace the working
contract set - and with it the registered provider that makes every other section of this guide
walkable - for no new evidence. On a fork every command genuinely executes against real chain state,
so §0.4 is proven rather than described.

**What ran.** The real deployment sequence, driven through `Deploy.deploy()` - the same function
`Deploy.run()` calls, at the same call depth so forge attributes every sub-call to the broadcasting
key. `run()` itself was deliberately not used, because its `_writeLedger` would overwrite
`contracts/deployments/roax.json`, the live set's own provenance record; the only differences are
that environment read, a printed report, and that ledger write.

**`ONCHAIN EXECUTION COMPLETE & SUCCESSFUL`, 12 transactions** - nine creations plus three registrar
wiring calls. Every contract read back with code: `ProviderRegistry` 21,187 bytes,
`DogTagIssuerFactory` 3,140, `DogTagSBTConsent` 9,670, `Groth16VerifierConsent` 1,933,
`VerificationRegistryConsent` 6,231, `ProtocolRegistry` 13,917.

**The immutable wiring that forces the order** was confirmed on the fork rather than assumed:
`factory.implementation()` is the freshly deployed `DogTagIssuer`, `factory.registry()` is the
freshly deployed `ProviderRegistry`, and `verificationRegistry.rootIndex()` is that same factory.

**The publish, both phases.** Phase 1 printed `Preflight OK: factory/sbt/providerRegistry/verifier
all agree with the registry` and staged all three records. Phase 2 printed `Re-preflight OK`,
`Staged bytes on BOTH axes still match this environment`, and **`dogtag-levelb/1 active true`**.
`getDiscoverySet` then returned a complete nine-word record with `active = 1`, and
`getActiveArtifactSet` decoded `minAppVersion` `1.4.0`.

**The four artifact pins were computed from the committed files, not copied**, and each matched the
value declared in `crates/dogtag-prover-rs/src/artifact.rs`: zkey `f83a111f…`, witness graph
`2f74d26b…`, r1cs `828e2923…`, wasm `482debcf…`.

**Not walked:** an Option B deploy broadcast to a real chain, for the reason above. The walk
harness was deleted and the worktree confirmed clean.

## 16.3 On a real handset, 2026-08-05 - the anchor resolves, and the refusal is real

Walked on **iPhone 15 Pro "KZG"** (device `F2A840C4-DFA6-5C37-AF64-77DBCA7C2B12`, udid
`00008130-001A08E40021401C`), iOS 26.6, against commit **`fd26154`**. The app was built for a real
device and installed - the first rebuild since the contracts were replaced, which is what made any
of this reachable: the previous build bundled a superseded `ProtocolRegistry`, so discovery failed
closed for a reason that was not the real one.

**The anchor.** `apps/ios/DogTag/roax.json` was regenerated from the ledger by
`scripts/gen-mobile-roax-config.sh`, and the build carries **`ProtocolRegistry 0xc385F939…76A60`** -
matching `contracts/deployments/roax.json`. The Profile screen renders it on the device, alongside
`ProviderRegistry 0x1ff6FdCeFf15AC…`, `DogTagSBT 0xFc33A3e702b7d6…` and `ROAX (chainId 135)`.

**A real credential.** A `TRAVEL_CLEARANCE` was issued by a live `government-api` and anchored:

| what | value |
|---|---|
| root | `0x20af931b400e49fd830b8768ef09ff859a98eee23ead39a20be73caf76828fbd` |
| tx | `0x09984f5fae65efcb53c49ffa00e2d3dc67d3ff113fbb0b22c5275a276fc1af68` |
| block | **348342** |
| issuing contract | `0x0Ca65d55…526E3` |

Read back with `cast`: `isValid(root)` = **true**, `issuedBy(root)` = the authorised signer, and
`rootIssuer(root)` = that contract. It was scanned onto the handset from a `/r/<token>` QR and the
phone renders it **VALID** - which additionally means the mandatory issuer-whitelist pillar resolved
on device, since a definite VALID requires it to pass.

**The discovery anchor resolves on device.** Scanning a verifier's `/x/<token>` export QR, selecting
that record and passing Face ID ran `runLevelBFlow`, which produced, on screen:

```
Owner-hidden verification refused: Invalid("claimed chainId 0 does not match anchor chainId 135")
```

That message comes from the catch after `validateDiscovery`, which is reached **only** past the
guard that requires both `getDiscoverySet` and `getActiveArtifactSet` to have returned decodable
records. So both anchor reads succeeded over the bundled endpoint, and the `135` in the message is
the anchor's own value. **The refusal is correct**: the verifier was deliberately running a
simulated chain and honestly advertised `chainId 0`, and the anti-redirect check refused the
mismatch.

Two things this does **not** establish: no Groth16 consent proof was generated (the flow refused
before proving), and no dog-tag bind was performed.

## 16.4 The role-by-role walk, 2026-08-05

Walked on **2026-08-05** against commit **`03b410c`**, on the live ROAX set (chainId 135), starting
from nothing running and bringing up one role at a time.

**The boot.** `contracts/.env` was reduced to the three lines §0.5 names and the vet role was booted
with **no contract addresses at all** - the genuine cold start. It came up, warned about both
contracts by name, and served the portal; `/health` answered `{"status":"ok"}` and the portal `200`.
On that stack the **Provider self-service page rendered all four flows**, which is what makes §3
reachable before any contract exists. Also checked: a second `demo-up.sh vet` is refused by port
with the role's own stop command named; bringing up `admin` alongside left the vet's recorded pids
untouched; and `demo-down.sh admin` left the vet serving.

**Boot-everything still works**, checked separately with a bare `scripts/demo-up.sh` from a clean
machine: all seven roles started, seven pid files, six backends answering `/health` and five portals
`200`, with `indexer` and `prover` among them and neither log carrying a stack trace.
`scripts/demo-down.sh` then stopped all eleven processes and left nothing listening.

**Both branches of §3.2 were walked, including the one a fresh clone hits.** The custody seal was
moved aside to put the vet in genuine first-run state, and the Setup wizard driven through
**genesis → confirm → unlock**: it showed 24 words once, required the written-down acknowledgment,
challenged three of them by position, took an encryption passphrase, and produced a working signer
that **Signing keys** then named. The original seal was restored afterwards and verified
byte-identical by sha256.

**§0** - the Option A check run verbatim under zsh: nine contracts, each with code, and
`getDiscoverySet` returning a nine-word record whose last word is `1`. The key check confirmed the
configured key resolves to the ledger's `admin`.

**§1** - admin booted alone; the Providers banner read *"The hosted admin key holds this registry -
registrar actions execute directly."* The provider row showed standing **active**, *"Cleared to
act."*, and controller `0xf39f…2266`. Record-type approval was driven for real, both directions:

| what | transaction | block |
|---|---|---|
| approve a record type | `0x4e83a73a…78946290` | 349050 |
| withdraw it again | `0x457556c1…b1294c64` | - |

**§3** - the vet portal signed in, custody unlocked, and **Signing keys** named the signer
`0x7e3a6603…0c436d` while honestly reporting *"No issuing contract is configured on this
deployment"*. A wallet holding the controller was connected on ROAX, and the deploy check ran twice:
at contract number `0` it **refused** with *"that one number's address is simply taken"*, and at `1`
it returned **Ready** with the exact address `0x14A09008…DEF3a` computed before anything was sent.

**§5.4 → use cases 1 and 2** - both contract addresses added to `contracts/.env` and the vet role
restarted; the boot then named both instead of warning. Custody re-locked and was unlocked again. A
pet was registered (**dog tag 1**, QR on the real `LAN_IP`), the `/p/<token>` endpoint returned the
block the phone would receive with `issuerClone` equal to the DOG_PROFILE contract, and a record was
issued to **Verified on-chain**:

| what | value |
|---|---|
| root | `0x0b685f56de169c10d233c98f932783305e926381f30ec6ee4f93e2f868d44ee7` |
| tx | `0x4800839b963a78b0f18196c3c387920acb94618042d158b015c0820fa21b4e10` |

Read back independently with `cast`: `rootIssuer(root)` = the VACCINATION contract
`0xdD1533D6…605d57`, `isValid(root)` = **true**, `issuedBy(root)` = `0x7E3A6603…0C436D`, and
`recordType()` = `keccak256("VACCINATION")`.

**Use case 3** - the groomer booted as a second business while the vet kept serving. A client and
pet were created, the pet opened, and dog tag `1` linked, giving the neutral **"Microchip not
compared - this shop holds no credential for that DogTag"**. The credential verified to **Verdict:
pass** with all five pillars and root == recomputed root. **All of it was done with custody LOCKED**,
which is why §6.1 tells you to leave it that way.

**Use case 4** - government booted alone and reported `backend: live`, `chainId: 135`,
`canSign: false`, both issuers `null`. It verified the vet's credential in the portal (**✓ VALID**)
and over the unauthenticated `POST /v1/verify` (`verdict: true`,
`issuerWhitelistState: "passed"`, block-pinned). The claim that a deployer seed satisfies the second
permission was confirmed on chain before relying on it: the contract's `owner()` and the government
signer are the same address, and `issuanceAllowed` and `canIssue` both read **true** with nothing
clicked. After pointing the backend at the contract and restarting, `/health` flipped to
`canSign: true` and a travel clearance anchored:

| what | value |
|---|---|
| root | `0x1c8cb68e2e95fca0a2ff8d61fd26dedf685b284eff3f7f589a0fc22f7fa2daee` |
| tx | `0x67fca6240d3b5fe4a18f6ea02140d62f3004e06d800fc871a6358994d2cc7731` |
| receipt | `W0ZJH1A2EH57` |

The public status page returned `effectiveStatus: VALID`, `simulated: false`. **Checked rather than
assumed:** its HTML twin at `/r/<receiptId>` is 1,739 bytes and contains none of the Section A
person fields, nor even an `@`.

**Use case 5** - the owner wallet booted with no chain preflight and no governance key, took the
credential, and rendered **✓ Integrity intact** and **✓ Anchored on-chain** with every hashed field,
including `DOG TAG ID 1`.

**Use case 6** - the bench on that credential: **nine rows, all Pass**, every read pinned to block
349139. The attack catalogue ran in full: **11 scenarios, 11 matched their declared expectation, 0
divergences** (3 valid, 7 not valid, 1 no verdict), the genuine control among them.

**Two defects this walk found in the boot script, both since fixed.** The government preflight asked
`isWhitelistedFor(recordType, signer)`, which the launch authority answers off its orthogonal VERIFY
axis - so it printed `whitelisted=false` and warned that `issue()` would revert, about a signer
whose `canIssue` and `issuanceAllowed` were both true and which issued successfully minutes later.
It now asks those two directly. And the preflight printed the configured chain endpoint even for a
role that never contacts it, which reads as a check that passed; it now prints that line only when a
check runs.

The wallet was a scripted EIP-6963 provider signing with the published anvil test key, injected into
the vet portal's `index.html` for the walk and reverted afterwards. `contracts/.env` was modified
during the walk and restored byte-identical, confirmed by sha256.

## 16.5 On the desktop stack, 2026-08-04

Walked on **2026-08-04** against commit **`3d3632f`** and, for the Signing keys page, `d0f8cd8`, on
a live stack on ROAX chainId 135.

**The two-layer issuance requirement** (now §5.3) was driven entirely from the browser, and each
step confirmed independently with `cast`:

| what | transaction | block |
|---|---|---|
| **Remove** the vet's signing key | `0xe31c29c7…c20ab86c` | 340437 |
| Issue a record, which is then **refused on chain** | reverted `0xa649bcb3` | - |
| **Admit** it again | `0xd144be4a…d0192c25` | 340452 |
| Issue the same record, which **anchors** | `0xcaaa6cfd…d9397e41` | 340456 |

The refusal is the load-bearing half: between blocks 340437 and 340452 the registry answered
`canIssue == true` while the contract's own list answered `false`, and `cast sig
NotLocallyAllowed()` is `0xa649bcb3` - so the page's warning was a true statement about what the
chain would do.

**Walked end to end on that date:** the boot and every `/health`; the deploy rehearsed on an
`anvil --fork-url` fork of ROAX to `ONCHAIN EXECUTION COMPLETE & SUCCESSFUL` (12 transactions) plus
the two-phase publish, with the ledger restored and verified byte-identical; admin sign-in,
approving a record type, and attach/activate/grant each mined and read back; the provider page's
flow 1 deploying a real DOG_PROFILE contract at exactly the predicted address, and flow 2's
`repointService` taking `canIssue` from false to true; register-pet through to the minted QR; a
record issued to **Verified on-chain** and verified independently (`rootIssuer`, `isValid`,
`issuedBy`, `recordType`); the whole groomer role including the published `.ics` diary fetched and
checked; government verification of the vet's credential both in the portal and over the
unauthenticated `POST /v1/verify`; a government credential issued to **✓ anchored on-chain** with
both public receipt surfaces confirmed to carry no person data; the owner wallet receiving the vet's
credential; and the bench on the genuine credential (nine rows, all Pass) plus the eleven-scenario
catalogue run in full with no divergence.

**The wallet was a scripted EIP-6963 provider signing with the published anvil test key** - the key
the ledger records as this provider's controller precisely so anyone can act as it on a disposable
testnet. The product code exercised is identical: the same connect path, the same preflights, the
same transactions. But no real wallet's own UI was driven, so a wallet-specific problem would not
have shown up.

**There is no CI for any of these paths.** The two mobile workflows are dispatch-only and no
workflow runs the Rust test suite, so a local walk is the only evidence any of this works. If you
change a portal, re-walk the section rather than assuming.
