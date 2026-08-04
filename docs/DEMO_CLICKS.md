# DogTag - from an empty machine to every use case

This guide was written by **performing every step on a live stack against ROAX (chainId 135)**, not by
reading the source and reasoning about it.
Every step marked as walked was performed, and what is written is what appeared on the screen.
Where a step could **not** be walked, it says so in that step, with the reason and with whatever you
would need in order to walk it yourself.

Read that promise precisely, because it is the whole value of this document: an honestly-marked
unwalked step is fine, and an unwalked step written as though it were walked is the defect this guide
exists to avoid.
§10 records exactly what was and was not covered, and when.

**No contract address is written into this guide.** Every one is resolved from
`contracts/deployments/roax.json`, which the deploy writes. A literal pasted into a page like this one
keeps working after a redeploy while naming a contract that decides nothing - which is exactly how the
previous revision came to carry nine dead addresses.

The order below is the order to read it in. Each role picks up the state the previous one left.

| § | Role | What they do |
|---|---|---|
| 0 | - | **Cold start: from an empty machine to a running stack** |
| 1 | **Admin** | Register a provider, and finish it off |
| 2 | **The provider** | Deploy their own contract and select it |
| 3 | **The vet** | Register a pet, issue a credential, anchor it |
| 4 | **The groomer** | Clients, pets, the diary, and checking a credential it did not write |
| 5 | **Government** | Verify somebody else's credential; issue their own |
| 6 | **The owner** | Receive a credential and read it |
| 7 | **Anyone verifying** | The bench, and eleven attempts to defeat it |
| 8 | - | Reference: live reads |
| 9 | - | What this guide does not cover |
| 10 | - | Evidence: what was walked, when, against which commit |

---

## 0. Cold start

**Read §0.1 before anything else.** Two of the steps below fail in ways that look like something else,
and one of them silently starts only a single portal out of five.

### 0.1 What you need on the machine

| | | Checked with |
|---|---|---|
| Rust toolchain | builds six backends | `cargo --version` |
| Node 22 + pnpm 10 | builds and serves five portals | `node --version && pnpm --version` |
| Foundry (`forge`/`cast`/`anvil`) | contract reads, and the deploy | `forge --version` |
| A funded key on ROAX | only if you deploy or drive registrar actions | §0.5 |

The Foundry submodules are **git submodules and a fresh worktree has them empty**, so the first
`forge` command fails on the remappings rather than on anything in the branch:

```
git submodule update --init --recursive contracts/lib/forge-std contracts/lib/openzeppelin-contracts
pnpm install --frozen-lockfile
```

### 0.2 The contracts are ALREADY deployed - confirm that, do not redeploy

`contracts/deployments/roax.json` is the ledger, and it is the only place an address lives. The whole
set is live on ROAX. Confirm it rather than trusting this page:

```bash
source scripts/lib/ledger.sh
RPC=https://devrpc.roax.net
for k in ProviderRegistry DogTagIssuerImpl DogTagIssuerFactory DogTagSBTConsent \
         Groth16VerifierConsent VerificationRegistryConsent ProviderDirectory \
         ServiceDomainResolver ProtocolRegistry; do
  a=$(ledger_addr $k); c=$(cast code $a --rpc-url $RPC)
  printf '%-30s %s %6d bytes\n' "$k" "$a" $(( (${#c} - 2) / 2 ))
done
```

Walked: all nine answer with code - `ProviderRegistry` 21187 bytes, `DogTagIssuerImpl` 4739,
`Groth16VerifierConsent` 1933. Those three numbers are the ones worth reading, because the ledger
states them as the reason the whole set moved: PRs #143 and #144 changed the issuer and the registry,
and every `immutable` that pins either one had to move with them.

**`source scripts/lib/ledger.sh` used to answer EMPTY for every key at a zsh prompt.** `BASH_SOURCE`
is a bash-only array, so under zsh - this repo's default shell - the helper resolved the wrong path
and `sed` failed into its own `2>/dev/null`. What you saw was not a diagnosis but `cast`'s
`invalid value '' for '[TO]': invalid string length`, with nothing anywhere naming the ledger. Fixed
in this branch; if you are on an older checkout, run these commands under `bash`.

And confirm the protocol version is published, because everything the phones resolve hangs off it:

```bash
cast call "$(ledger_addr ProtocolRegistry)" "getDiscoverySet(bytes32)" \
  "$(cast keccak 'dogtag-levelb/1')" --rpc-url https://devrpc.roax.net
```

Walked: a nine-word record whose last word is `1` (active), carrying the same factory, verification
registry, SBT, verifier and provider registry the ledger names.
**The getter is `getDiscoverySet`.** `getContractSet` belongs to an earlier record shape; calling it
reverts at the dispatcher with EMPTY returndata, which reads identically to an unpublished version.

### 0.3 Deploying a set of your own - rehearse on a fork FIRST

Skip this section entirely if you are using the deployed set, which is the normal case.

**`Deploy.s.sol`'s `run()` REWRITES `contracts/deployments/roax.json`** with whatever it just deployed,
including against a local chain. That is not a hypothetical: it was reproduced on this walk, and it
changed twelve lines. So back the ledger up, and check it afterwards rather than remembering to.

```bash
cp contracts/deployments/roax.json /tmp/roax.json.backup
anvil --fork-url https://devrpc.roax.net --port 8777 --silent &
echo $! > /tmp/anvil.pid                       # kill by THIS pid, never by name

cd contracts
export ADMIN=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266      # anvil account 0
export CUSTODIAN=0xa0Ee7A142d267C1f36714E4a8F75612F20a79720  # anvil account 9
export PUBLISHER=$ADMIN TESTNET_DEPLOY=true PUBLISH_TIMELOCK_SECS=0
forge script script/Deploy.s.sol --sig "run()" --rpc-url http://127.0.0.1:8777 \
  --broadcast --legacy --private-key <anvil account 0 key>
```

Walked: `ONCHAIN EXECUTION COMPLETE & SUCCESSFUL`, **12 transactions - 9 CREATEs then 3 `onlyOwner`
registrar-wiring calls on `ProviderRegistry`**, which is exactly the shape the ledger records for the
live deploy.

**`ADMIN` must BE the broadcasting key**, because those three wiring calls are `onlyOwner` on a core the
script has just handed to that address. On a fresh chain that means a rehearsal needs no key from this
repo - any funded account works, and anvil's own are fine. **On live ROAX it means the opposite**: the
deploy must be broadcast by the ledger's `admin` key, and no other. `CUSTODIAN` must not equal `ADMIN` -
the script refuses, because `ownerOf` would then return a key that signs.

**`TESTNET_DEPLOY=true` is a loud opt-in and it is the only way to get a non-default timelock.**
`Deploy.validatePublishTimelock` refuses anything but `DEFAULT_PUBLISH_TIMELOCK` (2 days) without it.
Production is that default plus the absence of the opt-in, and is **never** sniffed from
`block.chainid` - ROAX 135 is itself a live chain, so a `require(block.chainid == 135)` guard once
passed on precisely the deployment it claimed to refuse.

Then publish, in two phases against the registry you just deployed. The four artifact pins are
computed from the committed files rather than pasted:

```bash
R=<repo root>
export PUBLISH_PROTOCOL_REGISTRY=<the ProtocolRegistry the deploy printed>
export PUBLISH_FACTORY=…  PUBLISH_VERIFICATION_REGISTRY=…  PUBLISH_SBT=…
export PUBLISH_VERIFIER=…  PUBLISH_PROVIDER_REGISTRY=…
export PUBLISH_ZKEY_SHA256=0x$(shasum -a 256 $R/circuits/build/consent_final.zkey     | cut -d' ' -f1)
export PUBLISH_WITNESS_MOBILE_SHA256=0x$(shasum -a 256 $R/circuits/build/consent.graph | cut -d' ' -f1)
export PUBLISH_R1CS_SHA256=0x$(shasum -a 256 $R/circuits/build/consent.r1cs           | cut -d' ' -f1)
export PUBLISH_WASM_SHA256=0x$(shasum -a 256 $R/circuits/build/consent_js/consent.wasm | cut -d' ' -f1)
export PUBLISH_ARTIFACTS_URL=https://artifacts.dogtag.io/levelb1
export PUBLISH_MIN_APP_VERSION=1.4.0

forge script script/PublishProtocolVersions.s.sol:PublishProtocolVersionsPropose --sig "run()" …
forge script script/PublishProtocolVersions.s.sol:PublishProtocolVersionsExecute --sig "run()" …
```

Walked: both phases exit 0, three transactions each, and `getDiscoverySet` then returns the nine-word
record with `active = 1` and the fork's own addresses.
**The wait is zero on testnet and two days in production**, so on a testnet registry the two phases run
back to back; phase 2 needs the SAME environment phase 1 had, not just the registry address.

Two things worth knowing rather than discovering:

- **The four pins agree three ways** - `shasum` of the committed artifacts, the `LEVEL_B_V1` descriptor
  in `crates/dogtag-prover-rs/src/artifact.rs`, and the record on chain. Walked: all three identical.
  That is why they are computed here and never transcribed.
- **This script is the FIRST-ROLLOUT shape and sends six transactions.** Do not reach for it to rotate
  one axis: four of the six would re-publish a discovery set nobody asked to move, restamping its
  `publishedAt`. `contracts/script/PinConsentWitnessGraph.s.sol` is the narrow two-transaction shape.

Finally, put the ledger back and **check** rather than trust:

```bash
kill $(cat /tmp/anvil.pid)
cp /tmp/roax.json.backup contracts/deployments/roax.json
git diff --stat contracts/deployments/roax.json      # MUST be empty
```

### 0.4 Addresses are configuration, and the deploy writes it

You do not paste addresses into stacks. `scripts/gen-deployment-env.sh <target>` projects the ledger
onto each stack's variable names and prints to stdout; `scripts/gen-mobile-roax-config.sh` writes the
two gitignored mobile bundles. `scripts/gen-deployment-env.sh --list` names the targets.

The standing sequence after any redeploy is: run `Deploy.s.sol`, re-run those generators, restart -
and for the phones, **rebuild AND reinstall**, because `apps/*/roax.json` is compile-time. The four web
portals are compile-time in the same way: vite inlines every `VITE_*` at build time, so a served portal
keeps its old addresses until it is rebuilt.

`make check-addresses` is what keeps this true rather than a promise that decays. It is bidirectional -
an undeclared file carrying a ledger or retired address fails, and a declared file that no longer
carries one fails too.

### 0.5 `contracts/.env` - what belongs in it, and what must NOT

`scripts/demo-up.sh` sources this file with `set -a`, so **every variable in it overrides the ledger.**
That is the trap: a `contracts/.env` written before the redeploy carries `ISSUER_REGISTRY_ADDR`,
`VERIFICATION_REGISTRY_CONSENT_ADDR` and `SBT_CONSENT_ADDR` from a superseded set, and those beat the
ledger silently. The preflight then dies at the factory↔registry comparison, which reads like a chain
problem and is a stale-file problem.

**Delete every protocol address from it.** What it should carry is only what the ledger cannot answer:

```
ROAX_RPC=https://devrpc.roax.net
DEMO_MODE=1
GOVERNANCE_PRIVATE_KEY=…            # the key for the ledger's `admin`; never commit or echo it
PROFILE_ISSUER_ADDR=…               # a DOG_PROFILE clone - see §0.6
VACCINATION_ISSUER_ADDR=…           # a VACCINATION clone
TRAVEL_CLEARANCE_ISSUER_ADDR=…      # optional, for §5.2
GOV_SIGNER_KEY=…                    # optional, for §5.2
```

`DEMO_MODE=1` is not cosmetic: without it the backends refuse to boot on the dev secrets, with
`FATAL: refusing to boot in production mode: CENTRAL_HMAC_SECRET is set to the insecure dev default`.

### 0.6 The two clone addresses, and why they are not in the ledger

`PROFILE_ISSUER_ADDR` and `VACCINATION_ISSUER_ADDR` are **hard-required** by `demo-up.sh` - it exits
naming them. They are per-provider `DogTagIssuer` clones, deployed by a provider rather than by
`Deploy.s.sol`, so the ledger holds no key for them and there is nothing to resolve.

**A freshly deployed set has none of them**, so on a genuinely cold start you must create them, which
means walking §1 and §2 first. The ledger's `_provisioning` note records which clones the deployment
walk left behind; read that, and confirm each answers `recordType()` before using it:

```bash
cast call <clone> 'recordType()(bytes32)' --rpc-url https://devrpc.roax.net
cast keccak VACCINATION      # must match
```

Walked on this stack: the deployed set carried a VACCINATION clone and a TRAVEL_CLEARANCE clone and
**no DOG_PROFILE clone at all**, so §3.1 had nothing to anchor into until one was created. Creating it
is §1.2 → §2.4 → §1.3 → §2.5, and this walk did exactly that.

**A placeholder boots.** `demo-up.sh` requires the variables to be non-empty but does not check
`PROFILE_ISSUER_ADDR`'s record type, so you can point it at any clone to get the stack up, walk §1 and
§2 to create the real one, and restart. That is the order this walk used.

### 0.7 Decide now whether your data should survive a restart

Out of the box every backend uses an in-memory store: clients, pets, appointments, records and sessions
all vanish when a backend restarts. For one sitting that is fine.

Two things are required for persistence and each fails its own way:

**(a) The binaries must be built WITH the `mongo` cargo feature.** It is not a default, and a binary
built without it does not fall back - it refuses to start:

```
ERROR vet_api: MONGO_URI is set but this binary was built WITHOUT the `mongo` feature;
rebuild with --features mongo or unset MONGO_URI. Refusing to start.
```

`demo-up.sh` adds `--features mongo` itself when `MONGO_URI` is set.

**(b) Each backend needs its OWN database.** The vet and the groomer are the *same binary* with a
different `BUSINESS_TYPE`; point both at one database and the second to boot adopts the first's custody
blob, so two businesses share one signing identity with nothing on any screen saying so. `demo-up.sh`
gives each a distinct database (`dogtag`, `dogtag_vet`, `dogtag_groomer`, `dogtag_government`),
overridable per service with `MONGO_DB_ADMIN` / `MONGO_DB_VET` / `MONGO_DB_GROOMER` /
`MONGO_DB_GOVERNMENT`.

```
MONGO_URI=mongodb://127.0.0.1:27018 scripts/demo-up.sh
```

Verify off the running process rather than assuming - this is the check that cannot lie:

```
ps eww $(lsof -nP -iTCP:41874 -sTCP:LISTEN -t) | tr ' ' '\n' | grep '^MONGO_'
```

### 0.8 Boot

```
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh      # macOS; use your own LAN address
```

**Set `LAN_IP`.** It defaults to a hardcoded address that is almost certainly not yours - walked, the
default is `172.24.230.152` while this machine was `192.168.16.45`, and the default was unreachable.
It is stamped into every QR the stack mints and into government's `deploymentUrl`, so a wrong value
produces QR codes no phone can resolve, with nothing reporting it.

Walked preflight:

```
Preflight (chainId 135, https://devrpc.roax.net):
  chainId               135  ok
  factory               0xA248…113f -> registry 0x1ff6…461f  ok
  admin signer          0x8E27…F4A2 holds WHITELIST_ADMIN  ok
  government            LIVE chain, NO signer -> /issue can only dry_run (no on-chain anchor).
```

That last line is not an error; §5.2 explains it.

| Portal | | Backend | |
|---|---|---|---|
| admin | http://localhost:39741 | admin-api | :39742 |
| vet | http://localhost:41873 | vet-api | :41874 |
| groomer | http://localhost:43617 | groomer-api | :43618 |
| government | http://localhost:44831 | government-api | :44832 |
| owner wallet | http://localhost:45931 | *(no backend, by design)* | |
| | | prover | :41875 |
| | | indexer | :46001 |

It ends by printing `UP.` and those URLs. **If it prints only `admin-web` and then hangs, you are on a
checkout from before this branch**: three of the five portal invocations carried a stray blank line
after a trailing `\`, so the continuation died there, `env` ran with no command, and the leftover
`pnpm … dev` ran in the *foreground* and blocked the script forever. Introduced in #141, fixed here.
The tell is `.demo/admin-web.log` containing an environment dump rather than vite output.

**Stopping it: `scripts/demo-down.sh`**, which kills the PIDs the script recorded. Never
`pkill -f target/release/vet-api` or anything of that shape - this monorepo is checked out many times
over and every checkout builds to the same relative path, so a pattern kill reaches whichever instance
it happens to hit, including one somebody else is using.

### 0.9 Check it is actually up

```
for p in 39742 41874 43618 44832 41875 46001; do printf "%-6s " $p; curl -s http://127.0.0.1:$p/health; echo; done
for p in 39741 41873 43617 44831 45931; do printf "%-6s %s\n" $p "$(curl -s -o /dev/null -w '%{http_code}' http://localhost:$p/)"; done
```

Walked: `{"status":"ok"}` from admin, vet, groomer and prover; `200` from all five portals; and from
the indexer `{"backend":"simulated","chainId":null,"ok":true,"simulated":true}`.

**Use `localhost`, not `127.0.0.1`, for the portals** - vite dev servers bind IPv6 only, so a
`127.0.0.1` probe reports a perfectly healthy server as dead.

**`simulated: true` on the indexer is correct for a `demo-up.sh` stack.** It serves a scripted event
history with placeholder transaction hashes; those rows are correctly labelled "not chain-addressable"
and are not offered as explorer links. A live indexer reports `simulated: false`, and with an empty
scope registry answers every oversight query 401 by design. Check `/health` before reporting either as
a bug.

Government's `/health` is the richest and is worth reading in full - walked, it reported
`"backend":"live"`, `"chainId":135`, `"simulated":false`, `"canSign":false` and
`"issuers":{"EU_HEALTH_CERT":null,"TRAVEL_CLEARANCE":null}` on a stock boot. See §5.2.

### 0.10 Unlock custody on the vet and the groomer

**Custody re-locks on every restart of those two backends.** The sealed key survives; the decrypted
seed does not. Nothing signs or issues until you unlock.

Sign in at the vet portal - the operator password is prefilled in demo mode, so just click **Sign in** -
then click **Unlock** in the banner across the top. Both fields in the dialog are prefilled; click
**Unlock and continue**.

Walked: the dialog is titled *"Custody is locked"* and reads *"This action needs the backend signer.
Enter the passphrase to unlock and continue - nothing you have entered is lost."* That last clause is
literal - the prompt appears in place over whatever page you were on, so a half-filled form survives.
Repeat for the groomer portal.

**There is no §0 step for pointing the portals at the contracts any more.** `demo-up.sh` passes every
`VITE_*` address - the provider registry, the factory, both typed resolvers and the content mirror - to
all five portals. A previous revision of this guide told you to kill the dev servers and restart them
by hand with exports; that is obsolete, and doing it now would only re-supply what the script already
supplied.

---

## 1. Admin

### 1.1 Sign in

http://localhost:39741 - the admin password is prefilled in demo mode. Click **Sign in**. You land on
the **Dashboard**.

Walked, on a stock `demo-up.sh` stack, the Dashboard reports:

- Registered businesses **0**, Pending applications **0**, Approved issuers **0**
- *"The oversight indexer is not connected in this environment, so cross-issuer on-chain totals are
  unavailable. Set `INDEXER_API_BASE` on the central backend to enable them."*
- **AUTHORITY: "Authority map unavailable."**

**That last one is expected on the launch set and is not a misconfiguration.** The card needs the
factory's `Ownable2Step` owner, and the launch `DogTagIssuerFactory` is a plain contract with **no
`owner()` at all** - `cast call <factory> 'owner()(address)'` reverts. The endpoint behind the card
answers 200 with `factoryOwner.owner: null`, and the card declines to render a partial map.

Do not read that as "the hosted key holds nothing". Walked, `admin-api`'s own boot log says the
opposite - *"control-plane authority preflight: hosted signer 0x8e27… holds every configured
authority"* - and the chain agrees: `ProviderRegistry.owner()` is the ledger's `admin`, and
`hasRole(DEFAULT_ADMIN_ROLE, admin)` and `hasRole(WHITELIST_ADMIN, admin)` both read `true`.
**The place that tells you whether registrar actions will execute is the Providers page banner** (§1.2).

The indexer line is a *different* condition from the indexer being simulated: `demo-up.sh` does not
pass `INDEXER_API_BASE` to `admin-api` at all, so the Dashboard totals and the Activity page 503 while
the indexer itself is perfectly healthy on :46001.

### 1.2 Register a provider - **Providers** in the left nav

Read the banner at the top first. Walked:

> The hosted admin key holds this registry - registrar actions execute directly.

If it says anything else, your `ADMIN_PRIVATE_KEY` is not the governance signer and every action below
comes back **proposed** rather than mined - carrying unsigned calldata for someone else to execute.
Nothing is broken in that case, but nothing reaches the chain either.

**It is three steps, not two, and the third is the one people miss.** Registration writes standing
`PENDING`, and the registry admits only `ACTIVE`. A provider that is registered and approved but never
activated is inert - and the row says so.

**Step 1 - Register.** Click **Register provider**, then **Fill demo data**. That mints a fresh random
provider id, fills the controller, and writes an obviously-fake identity statement. Click **Review** -
which computes the keccak256 digest and enables the button - then **Register**.

Three things there are worth reading rather than clicking past:

- **The provider id is arbitrary and permanent.** Generated at random, deliberately meaning nothing -
  not the name, not the domain, not a key. Copy it: the provider needs it for §2, and it can never be
  reassigned.
- **The identity statement text is never sent to the backend and is stored nowhere.** Only its digest
  reaches the chain. It stays readable under **Registrar actions this session**, and only until you
  navigate away.
- **Registration refuses placeholder data.** A zero identity digest, schema or hash algorithm is
  rejected by the contract, so a real statement is a precondition rather than a formality.

**Step 2 - Activate.** The new row reads **pending** with the note *"Registered but INERT - every
self-service action refuses until this is Active."* Click **Activate**; the note becomes *"Cleared to
act."*

**Step 3 - Approve a record type.** The row carries one button per record type - **DOG_PROFILE**,
**VACCINATION**, **GROOMING**, **BOARDING**. Click the one you need.

Walked, on the provider the deployment left in place: clicking **DOG_PROFILE** produced
**"Mined: approved DOG_PROFILE"** with the provider id and the transaction hash, and the row's
**APPROVED TO CREATE** cell gained `DOG_PROFILE` beside the record types it already held. A **Withdraw
DOG_PROFILE** button replaced it. The grant is per record type: approving one approves no other.

**Confirm it from the chain rather than from the page.** This is the exact read the provider's Deploy
button is gated on, and it **must be asked as the factory**:

```bash
source scripts/lib/ledger.sh
cast call "$(ledger_addr ProviderRegistry)" "canCreateService(bytes20,bytes32,address)(bool)" \
  <providerId> "$(cast keccak DOG_PROFILE)" <controllerAddr> \
  --from "$(ledger_addr DogTagIssuerFactory)" --rpc-url https://devrpc.roax.net
```

Walked: `true` for DOG_PROFILE, VACCINATION and TRAVEL_CLEARANCE; `false` for GROOMING and BOARDING,
which were never approved. **Omit `--from` and it answers `false` for every provider on earth** - with
no `from`, `msg.sender` is the zero address, no factory generation matches, and the whole aggregate
collapses. The portal passes the factory account on exactly that one read.

### 1.3 Finish the provider off - the four actions only DogTag can take

**Do this after the provider has deployed their contract (§2.4).** Everything here is `onlyOwner` on the
provider registry, so no provider key can do any of it.

Expand the provider's row with **Show services** beside its id. That opens **Attached services** - what
this provider has attached, and for each one the five lifecycle terms `canIssue` folds, shown
separately because each has a different fix.

Walked, a healthy service row reads: *Provider active ✓ · Service active ✓ · Factory generation live ✓ ·
Owner confirmed ✓ · An issuer may issue now ✓*, then **holds the issue right (registry-wide)** with the
addresses, then *"The provider has published this as its current VACCINATION service."*

**A withdrawn grant is shown struck through beside the granted ones, and that is deliberate** - "granted
then withdrawn" is a different fact from "never granted", and only the log can tell them apart. Read the
badges, not a flattened text copy: the distinction is carried by styling.

#### 1.3.1 Attach their contract

Click **Attach a contract**, paste the address the provider sent you, press **Check**, then **Attach**.

The address is the only thing you type: the check probes each active factory generation's own `isClone`
to work out which generation deployed it, then reads `recordType()` and `owner()` off the contract.
Walked, it reported *"Checked - ready to attach"* with the factory generation, **record type
DOG_PROFILE** and the owner, plus *"— sent as the expected owner; a mismatch at send time refuses the
transaction rather than attaching to the wrong key."* Then **"Mined: attached 0xd6c3…a9f8"**.

> **The one refusal worth recognising.** A *generation-1* `DogTagIssuer` has no `owner()` at all, so it
> can never be attached however correct the rest of the form is. The check says so in words rather than
> letting the send revert - a property of that contract, not a form error to correct.

#### 1.3.2 Set its standing to Active

Attaching lands the service at PENDING exactly as registering lands a provider there, so **attaching
alone grants nothing**. Click **Activate** on the service row. Walked: **"Mined: service 0xd6c3…a9f8
standing → active"**.

#### 1.3.3 Grant the issue right

Click **Issuance capability** on the service row. Walked, the dialog is headed *"Issue right on this
address, registry-wide"* and says:

> The key that will SIGN issuances. This is the registrar's grant to make and nobody else's: a service
> delegate carries content-write permissions and does not satisfy `canIssue`, so a provider cannot grant
> their own signing key. The grant is on the ADDRESS and names no service, so it reaches EVERY service in
> effective standing - including other providers'.

Enter the address that will sign, pick **Grant**, confirm. Walked: **"Mined: issue right granted to
0x7e3a…436d (every service in standing)"**.

**Read the scope sentence literally.** Since #143 the right is a bitmask on an address (`rightsOf`), not
a per-service grant. That is precisely why issuing takes a *second* check - see §3.0.

#### 1.3.4 Approve the typed resolvers

In **Typed resolvers**, click **Approve / pull** for each kind.

**The panel carries no textual approved/not-approved state** - walked, each row is just the kind, the
address, a copy button and the button, with the address badge rendered in the success style when
approved. Read the chain if you need certainty:

```bash
source scripts/lib/ledger.sh
PR=$(ledger_addr ProviderRegistry)
cast call $PR 'isResolverApproved(uint8,address)(bool)' 0 "$(ledger_addr ProviderDirectory)"      --rpc-url https://devrpc.roax.net
cast call $PR 'isResolverApproved(uint8,address)(bool)' 1 "$(ledger_addr ServiceDomainResolver)"  --rpc-url https://devrpc.roax.net
```

Walked: **both `true`** on the deployed set - the registrar half of §2.6 and §2.7 is already done.
Approving is the whole of the registrar's part; the provider must then **select** the resolver on their
own portal, and that half was **not** done (see §2.6).

> **`resolverApproved()` exists on `ProviderDirectory` only.** `ServiceDomainResolver` has no such
> function and the call reverts with empty returndata; its equivalent is the per-service
> `isAuthoritativeFor(address)`. Ask the core's `isResolverApproved` for both, as above.

> **Verify capability** is the card below and is deliberately *not* part of this sequence. It is keyed
> by PURPOSE and takes no service: an issuer is not implicitly a verifier. Walked, all five purposes -
> `boarding_intake`, `travel_check`, `grooming_intake`, `daycare_access`, `service_animal` - read *"No
> relayer may verify for this purpose."*

### 1.4 The rest of the admin portal

Opened but not re-walked: **Activity**, **Issuers / Factory**, **Onboard issuer**, **Business
registry**, **Issuer applications**, **Governance**. **Verification bench** has a section of its own - §7.

> **The Whitelist page is gone**, and is not coming back. It called `isWhitelistedFor` on the single
> authority, which answers that off an orthogonal axis - a confident `false` for every genuine issuer
> signer - and granted through `whitelistFor`, which that contract does not implement at all. Its
> replacements are **Issuance capability** (§1.3.3) and **Verify capability**, both on the Providers page.

**Activity** reads the oversight indexer and 503s on a `demo-up.sh` stack, because `INDEXER_API_BASE`
is not passed to `admin-api` (§1.1).

---

## 2. The provider sets themselves up

The provider is the business the admin just registered. This is where they deploy their own issuing
contract and select it.

**There is no backend on this path at all**: every read and every write is made from the provider's own
wallet, straight to the chain.

### Read this first: the sequence, and whose move each step is

Making a contract live takes **three moves, and the middle one is DogTag's**:

> provider deploys (`createIssuer`) → **REGISTRAR attaches + activates** (§1.3.1, §1.3.2) → provider
> selects it as current (`repointService`)

`attachService` is `onlyOwner` on the provider registry, so no provider key can send it however
correctly everything else is set up - and `repointService` refuses an address that was never attached.

| Flow | Needs from DogTag first |
|---|---|
| 1. Deploy your own contract | nothing |
| 2. Choose which contract is current | attach (§1.3.1) + standing (§1.3.2) |
| 3. Your domain | the domain resolver approved (§1.3.4) - **already true on the deployed set** |
| 4. Your listing | the directory resolver approved (§1.3.4) - **already true on the deployed set** |

**Issuing needs one more thing after all of that** - two, in fact. See §3.0.

### Before you start

| | What you need | Where it comes from |
|---|---|---|
| 1 | **Your provider id** | §1.2, on the row you registered |
| 2 | **A browser wallet holding the controller address** | the address §1.2 registered as controller |
| 3 | **Testnet gas in that account** | every action here is a transaction you sign |

This page has no backend, so there is no hosted key to act on your behalf - not for the writes, and not
even for the read-only **Check** buttons.

### 2.1 Open the page

Vet portal → **Provider self-service** in the left nav (`/provider`; the groomer portal has the same
item).

**If the whole page is replaced by "Provider self-service is not configured"**, the `VITE_*` addresses
did not reach the portal. On a `demo-up.sh` stack they do - walked, the page rendered all four flows
immediately. If you served the portal some other way, vite inlines `VITE_*` at **build** time, so set
them before building.

### 2.2 The vet and the groomer show different pages

- **Vet**: four flows - deploy, choose, domain, listing.
- **Groomer**: **only the listing**, headed just *Your provider record* and *Your listing*.

A groomer verifies and does not issue, so it has no issuing contract at all; flows 1 to 3 are keyed by
one and are not merely hidden but inapplicable. Flow 4 is keyed by the provider record, so a groomer
appears in the directory exactly as a vet does. **The rest of §2 assumes the vet portal.**

### 2.3 Connect your wallet

**Every control is disabled until a wallet is connected - including the read-only Check buttons.** With
no backend on this path, every action *and every preflight* is made from the provider's own key.

Connect from the **top-right of the portal header**, not the page itself, and make sure it is on ROAX.
Then paste your provider id and set the record type.

**Do the chain check before you deploy, not after.** The **Check** buttons do not test which chain your
wallet is on - those reads go to the portal's own endpoint - so on the wrong chain every check still
passes and only the **Deploy** fails, as a wallet error that does not obviously name the cause.

> **Careful with Fill demo data.** It fills the record type, a domain, a location and the contacts - but
> deliberately **not** the provider id, because a made-up provider id is one that does not exist.

### 2.4 Flow 1 - deploy your own contract

1. Leave **Contract number** at `0`. (It is the CREATE2 salt's only free input: your contract's address
   is computed in advance from the record type, your wallet and this number, so it is the only part of
   the address you get to choose.)
2. Click **Check what this would deploy**. **Deploy stays disabled until you do**, and says so - it only
   ever sends something a check has approved, for the values in the form *at that moment*. Change a
   field afterwards and it goes back to disabled, saying that too.

Walked, for `DOG_PROFILE`: verdict **Ready**, the exact address shown before anything was sent, and two
checks -

| Check | Result |
|---|---|
| Is your provider record cleared to act? | **PASSED** - *"Your provider record is active."* |
| May this key deploy a contract for your provider and record type? | **PASSED** - *"Approved: your key may deploy this record type for your provider."* |

with *"Ready to deploy. You will own the contract. DogTag then attaches it to your provider record
before you can select it."*

3. Click **Deploy** and confirm in your wallet.

Walked: the contract deployed at **exactly** the address the preview named - independently predicted
beforehand with `predictIssuer(bytes32,address,uint96)` and identical. Confirmed from the chain
afterwards: `owner()` is the controller, `recordType()` equals `keccak("DOG_PROFILE")`, the factory's
`isClone` answers `true`, and **`issuanceAllowed(creator)` is already `true`** - the clone seeds its own
list with its deployer (#144).

The plan card then relabels itself *"Superseded: these answers were read before your transaction"*, with
the verdict struck through and an amber *"Superseded"* beside it. The answers stay on screen rather than
vanishing, which is the point.

**Copy the deployed address.** Flows 2 and 3 need it.

**If it refuses** with *"Contract number 0 has already been used"*, you have deployed here before. Each
number produces one fixed address and two contracts cannot share one. Put `1` in **Contract number** and
check again.

### 2.5 Flow 2 - choose which contract is current

**DogTag has to attach and activate your contract first** (§1.3.1, §1.3.2). The page says so in a notice
before you click.

1. Paste the deployed address into **Contract address**.
2. Click **Check this contract**.

Walked **after** §1.3.1 and §1.3.2: verdict **Ready**, with *"Attached to your provider record, and
available to select."* and five checks, all **PASSED** -

| Check | |
|---|---|
| Did the DogTag issuer factory deploy this contract? | PASSED |
| Who owns this contract? | PASSED |
| Has DogTag attached this contract to your provider record? | PASSED |
| May your key select this contract? | PASSED |
| Does this contract's standing still allow changes? | PASSED |

3. Click **Make this my current contract**. Walked: `repointService` mined.

**Before DogTag attaches it**, the third check instead reads **FAILED** - *"Not yet attached. A contract
you have deployed is not listed until DogTag attaches it."* - with the remedy in the product's own
words. That is the expected state if you get ahead of the registrar, not a fault in what you deployed.

**This is the move that makes `canIssue` finally read true.** Walked and confirmed on chain:
`canIssue(clone, controller)` went `false` → `true` with `repointService`, while `canRevoke` and
`isRecognizedIssuer` were already true - the nested ladder folding the current-service pointer, not a
fault to debug.

**Leave the contract address in the field.** Flow 3 reads it.

> **One piece of on-page copy is now stale.** Flow 2's notice still says the attach step *"has no page
> for it yet"*. It does - §1.3.1.

### 2.6 Flow 3 - claim your domain

**The trap on this page:** the **Check the domain record** button is gated on flow 2's **Contract
address** field, not on the Domain field beside it. With step 2 empty the button stays dead however much
you type here. The page says so in amber.

Type a domain and click **Check the domain record**.

**Not walked to completion on this stack.** The registrar half is done - `isResolverApproved(DOMAIN, …)`
reads `true` (§1.3.4) - but the *provider* half is not: the service's `domainResolver` is the zero
address, so nothing is selected. Confirm which half you are missing rather than guessing:

```bash
source scripts/lib/ledger.sh
cast call "$(ledger_addr ProviderRegistry)" \
  'service(address)((bytes20,bytes32,bytes32,address,address,uint64,uint8))' <clone> \
  --rpc-url https://devrpc.roax.net
# fields: providerId, factoryGeneration, recordType, confirmedOwner, domainResolver, ownerEpoch, standing
```

Walked: `domainResolver` is `0x0000…0000` on every attached service. Selecting it is
`setDomainResolver(service, resolver)` and it is the provider's own call.

**Publishing no domain and never having said are kept apart, deliberately** - the resolver's disposition
is `UNSET | NO_DOMAIN | CLAIMED | CLEARED`, so an empty string is never allowed to mean three things.

### 2.7 Flow 4 - publish your listing

Click **Fill demo data** to populate contacts and a location, then **Check what this would publish**.

**Not walked to completion on this stack**, for the same reason as §2.6: the directory resolver is
approved but the provider record's `directoryResolver` is the zero address.

```bash
source scripts/lib/ledger.sh
cast call "$(ledger_addr ProviderRegistry)" \
  'provider(bytes20)((address,address,bool,bool,address,uint64,uint64,uint8))' <providerId> \
  --rpc-url https://devrpc.roax.net
# the FIFTH field is directoryResolver; walked, it is 0x0
```

**`resolverApproved()` on `ProviderDirectory` now reads `true`.** A previous revision of this guide told
you to run that read and expect `false`, and used it to explain why flow 4 stops. That is no longer the
reason - approval is done, selection is not, and the two are different halves. The product's own message
covers both (*"either not approved by DogTag or not selected by your provider record"*), which is why
reading the chain is the only way to tell which.

Two properties worth knowing for when it does open:

- **A blank location publishes no pin at all.** It does not publish `0,0`, which is a real coordinate off
  the coast of Ghana. The page states the consequence plainly.
- **Re-publishing rewrites rather than appends**, which is what stops one provider appearing in two
  places at once.

### 2.8 Where §2 leaves you

A full walk is: §1.2 register, activate and approve → §2.4 the provider deploys → §1.3 DogTag attaches,
activates and grants → §2.5 the provider selects it. **Walked end to end on this stack**, to a live
`canIssue(service, signer) == true` for a brand-new DOG_PROFILE contract; `serviceCount()` went from 1 to 3.

Flows 3 and 4 additionally need the provider to select each resolver, which has no button on this page
yet.

---

## 3. The vet issues a credential

Vet portal, http://localhost:41873. Custody must be unlocked (§0.10).

### 3.0 Before the vet can anchor anything: issuance takes TWO checks

This is the step most likely to stop you, it is not on any screen, and it is the reason a correctly
registered provider with a correctly attached contract can still fail to issue.

`DogTagIssuer.issue` requires **both**:

```
registry.canIssue(address(this), msg.sender)   // the authority's scope-free grant + lifecycle
&& issuanceAllowed[msg.sender]                 // THIS clone's own list
```

Layer 1 is the registrar's **Issuance capability** (§1.3.3). Layer 2 is `setIssuanceAllowed` on the
clone, callable by the clone's `owner()` alone.

**There is no product surface for layer 2.** Walked, and confirmed by grep: `setIssuanceAllowed` appears
in no portal and no backend route. A clone seeds its own list with its **deployer** (#144), so a provider
signing with the key that deployed the contract is already admitted - but the vet backend signs with its
**own custody signer**, which is a different key. That key has to be admitted by hand:

```bash
# From the clone OWNER's key. There is no button for this.
cast send <clone> 'setIssuanceAllowed(address,bool)' <vetCustodySigner> true \
  --private-key <cloneOwnerKey> --rpc-url https://devrpc.roax.net --legacy
```

Find the vet's signer from its own operator-gated route:

```bash
curl -s http://127.0.0.1:41874/issuer/signers -H "Authorization: Bearer <vet.opToken from localStorage>"
```

Walked: `{"activeSigner":"0x7e3a…436d", …}`. Then grant layer 1 in the admin portal (§1.3.3) and send
layer 2 as above. Verify both before trying to issue - a definite `false` on either refuses:

```bash
cast call "$(ledger_addr ProviderRegistry)" 'canIssue(address,address)(bool)' <clone> <signer> --rpc-url $RPC
cast call <clone> 'issuanceAllowed(address)(bool)' <signer> --rpc-url $RPC
```

**`scripts/demo-bootstrap.sh` does NOT do this for you.** Its own header says it is stale against the
launch set: it grants through `whitelistFor`, which `ProviderRegistry` does not implement on the issuance
axis. Do not run it and conclude the grant is in place.

### 3.1 Register the pet - **Register pet** in the nav (`/issue-dog-tag`)

A record attaches to a pet that already has a dog tag, so this comes first, once per pet.

Click **Fill demo data** - walked, it fills owner identity (`Alex Doe`, `GB`, a passport number) and the
pet profile (`Rex`, dog, Labrador Retriever, male, neutered) plus a microchip. Click **Start issuance**.

Walked result:

> **Owner scans to receive their dog tag**
> Dog tag **1** allocated — pending device bind.
> `http://192.168.16.45:41874/p/c80bab85517c3ead86ee26654ec2ae08`
> Owner scans this in the DogTag app to receive their dog tag. Waiting for the device to bind…
> Status: **pending**

Note the host is the `LAN_IP` you set in §0.8. With the default it would be an address no phone can reach.

**This is as far as a desktop can take it, and the reason is architectural rather than a gap.** The
owner's device folds the profile tree and posts the resulting root; the vet never holds the owner's
secret. Completing the bind needs the phone app (§9).

You can still see exactly what the phone would receive:

```
curl -s http://127.0.0.1:41874/p/<token>
```

Walked: JSON carrying `sessionId`, `dogTagId: "1"`, `status: "pending"`, the full pet block, the owner
identity block, the vet-generated `identityLeaves` (each with its own salt), and an `unverifiedClaims`
block whose `issuerClone` was **the DOG_PROFILE clone created in §2** - which is the check that the whole
chain is coherent.

### 3.2 Issue a record - **Issue a record** in the nav

**A record can be issued while the dog tag is still pending its bind** - walked, and worth knowing,
because the two are anchored independently.

Click **Fill demo data** (a rabies certificate, recordType `VACCINATION`), set **Dog tag id** to `1`, and
click **Sign & Issue**.

Walked result:

> **Credential issued** — Record `85b03c74-ee79-42ca-a63f-571500624485`
> **On-chain status: Verified on-chain** — `DogTagIssuer.isValid(root) = true`
> Merkle root: `0x1acc40a0da64d0b150824c88782be58ae1ac64f49504a186766412c1961acbdf`

Confirmed independently from the chain: the transaction succeeded, `rootIssuer(root)` resolves to the
VACCINATION clone, `isValid(root)` is `true`, `issuedBy(root)` is the vet's custody signer, and the
clone's `recordType()` equals `keccak("VACCINATION")`.

**Note the form is filled even when the page text looks empty** - the values live in inputs, which do not
appear in a text dump of the page.

### 3.3 Getting the credential out

The **Records** page lists what this shop has issued, with an explorer link for each. To take a
credential elsewhere - into the owner wallet, or the bench - fetch it from the backend:

```
curl -s http://127.0.0.1:41874/records -H "Authorization: Bearer <vet.opToken from localStorage>"
```

**The field is `wrapped_doc`, snake_case** - walked; a previous revision of this guide called it
`wrappedDoc`. Each row also carries `confirmed_tx_hash`, `block_number` and `explorer_url`.

---

## 4. The groomer

Groomer portal, http://localhost:43617. Sign in - the operator password is prefilled - and unlock custody
exactly as in §0.10.

**The groomer runs the same backend binary as the vet with `BUSINESS_TYPE=groomer`, and that is visible
in the nav.** Walked, the groomer's nav is: Dashboard, Calendar, Appointments, Clients, Pets, All
verifications, Groomers, Reports, Marketing, Import from user, Ad-hoc verification, Setup, Provider
self-service, Settings.

**There is no Records page and no Issue page** - a groomer is whitelisted to VERIFY, not to issue, and
the issuance routes are not even mounted on its backend.

### 4.1 Create a client, with their pet

**Clients → New client → Fill demo data → Create client.**

Walked: the demo values are unmistakably fake - `Demo Client (sample)`, `+65 6000 0000`,
`demo.client@example.com`, `1 Demo Street, #01-01, Singapore 000000`, notes *"Demo record - not a real
person."* - with an inline pet block filled as `Demo Dog (sample)`, Golden Retriever, female, and a
microchip. **Add pet** adds further pet blocks to the same client. The client then appears in the list
under NAME / CONTACT / PETS.

The pet block carries its own note: *"Pet details only. A DogTag is linked from the pet's own page, where
what linking does - and what it does not do - is spelled out. Any tag already linked stays as it is."*

### 4.2 The pet is addressable in its own right

**Pets** lists every pet the shop knows, under PET / SPECIES-BREED / OWNER / DOGTAG. Walked, the new pet
read `Demo Dog (sample) · Dog · Golden Retriever · Demo Client (sample) · **No tag**`.

Click the pet to open its detail page. Walked: the profile, the microchip, the owner as a link, and
**Edit** / **Book appointment** / **Delete**, plus a DogTag block and a Credentials block.

### 4.3 Link a DogTag, and watch the microchip cross-check

Under **DogTag**, type the tag id and click **Add DogTag**. Walked with tag `1` - the one the vet
allocated in §3.1 - the row became *"1 linked to this pet's record"* with a **Remove from this record**
action, and the cross-check reported:

> **Microchip not compared**
> This shop holds no credential for that DogTag, so there is nothing to compare the microchip against.

**That neutral state is the one to understand, because it is the common one - and note WHY it fired
here.** The groomer's store is separate from the vet's, so issuing a credential in §3.2 puts nothing in
this shop's hands. The check compares the microchip on the shop's own pet record against the microchip
inside a credential *this shop holds* for that tag; with none held there is nothing to compare, so it
says so rather than reporting a pass. A mismatch refuses the link rather than changing any credential's
verdict - the credential is still genuine, it just describes a different animal. Getting a credential
into the shop is what **Import from user** (§4.6) is for.

The same page states the role boundary plainly: *"This portal cannot mint a DogTag or revoke a
credential. A groomer is whitelisted to VERIFY, not to issue — which is exactly what makes a vaccination
check here worth anything, since the shop asserting the record cannot also be the shop that wrote it."*

### 4.4 Book an appointment

From the pet page click **Book appointment** - it carries the client and pet through in the URL - then
**Fill demo data**, then **Book appointment**.

**One thing to correct by hand: Fill demo data clears the pre-selected pet.** Walked and confirmed
directly: the form URL carried `?clientId=…&petId=…`, and after the demo fill the **Pet** select read
*No specific pet*. That is legal - an appointment need not name a pet - but set it back before booking if
you want the verification tied to one.

Walked after setting it back: booked as *Demo full groom (sample)*, status **Scheduled**, with
CLIENT / PET / GROOMER (*Demo Groomer*) / BOOKED all populated and the notes *"Demo booking - not a real
appointment."* The detail page also carries **Send to client**, the per-booking handoff.

### 4.5 See it on the calendar, and publish the diary

**Calendar** shows Day / Week / Month with a **Today** control. Walked: the booking appeared in the week
view on its day, showing the time and the client.

**Calendar sync** publishes the shop's diary at one web address that Google, Apple or Luma can subscribe
to. Click **Publish calendar address**; it mints a secret URL ending `.ics` and states the trade-off:

> **Treat this address like a password.** Anyone who has it can read your whole schedule — client names,
> pets and times — without logging in. That is what lets a calendar app subscribe without an account.

Walked: fetching that URL returned a real iCalendar document carrying
`X-WR-CALNAME:Pampered Paws — appointments`, a 15-minute refresh interval
(`REFRESH-INTERVAL;VALUE=DURATION:PT15M` and `X-PUBLISHED-TTL:PT15M`), and a `VEVENT` for the booking
with its `DTSTART`, `STATUS:CONFIRMED` and a `SUMMARY` of
`Demo full groom (sample) - Demo Dog (sample) / Demo Client (sample)`.

The route is unauthenticated by necessity - a calendar client cannot present a bearer - so the 32-byte
secret in the path is the whole gate. It is revocable and rotatable from that page.

The same page imports an `.ics` the other way; the file is parsed **in the browser**, which is what makes
times in any timezone come across exactly.

### 4.6 Check a credential the shop did not write

**Ad-hoc verification** (`/verify`) has two surfaces answering different questions.

**Verify credential** takes a pasted wrapped document and needs no owner and no phone: it recomputes
integrity and reads the issuing contract directly on ROAX. Walked with the credential from §3.2:

> **Verdict: pass** · Valid · VACCINATION
> Integrity **Yes** · On-chain valid **Yes** · Issued **Yes** · Revoked **No** · Issuer authorised at
> issuance **Yes**
> Root and Recomputed root identical · Issuer clone · **Issuing signer (from chain)** · Issued at

The signer is read from the chain (`issuedBy`) rather than typed in; the optional **Expected issuer
signer** field can only tighten the result. *(The button is labelled **Verify credential**; a previous
revision of this guide called it "Check credential status".)*

**Export on chain** is the other question - the owner-hidden consent flow, where the owner approves on
their phone and the shop learns only what the owner chose to share. It starts with a purpose (walked:
*Grooming intake — rabies status*) and **Start export**, which mints a QR. **Completing it needs the
phone app**, so it is not walked here; §9 lists it.

**Import from user** is the third route, and the one that would populate §4.3's cross-check. It wants a
customer JWT that a receptionist has no way to obtain, so it was not walked.

**All verifications** is the shop's own durable history - purpose, record type, status, transaction and
timestamps. It deliberately stores no credential PII.

### 4.7 Prove to yourself the data survives

If you set up Mongo in §0.7, confirm it rather than trusting it. Restart the groomer backend **by PID**:

```
kill $(lsof -nP -iTCP:43618 -sTCP:LISTEN -t)      # then re-run scripts/demo-up.sh
```

Three distinct outcomes worth telling apart: the **shop data survives**; the **operator session survives
too** (sessions live in the same store), so no re-login; and **custody re-locks**, because the sealed key
persists and the decrypted seed does not. On the default in-memory store all three are lost instead, and
nothing warns you - which is why §0.7 is a decision to make before you start.

---

## 5. Government

Government portal, http://localhost:44831.

### 5.1 Verify somebody else's credential - **walked, and it works**

This is the cross-role check: the vet issued in §3.2, the government verifies here, and the two share no
database.

Paste the wrapped document into **Verify a document (paste JSON)** and click **Verify**.

Walked result on the vet's real credential:

> **✓ VALID**
> integrity: yes · on-chain: yes · issuer authorised at issuance: yes · factory-deployed: yes
> Issuer **Seaport Vet** *(from the document — the issuing contract could not be read)*
> The on-chain domain claim could not be read
> *This document carries no issuer identity inside its Merkle root, so its issuer domain could not be
> cross-checked.*

The JSON beneath it carries `issuerWhitelistState: "passed"`, `issuerWhitelisted: true` and a
`blockNumber` - every read pinned to one block.

Two things there are worth pausing on.

**"Issuer authorised at issuance" is a historical question, not a current one.** The page says so:
*"whether the signer that anchored this root held the capability AT THAT BLOCK, from the governing
registry's own grant log. Delisting is forward-only, so a since-rotated issuer does not invalidate what
it issued."* A vet that rotates its key does not retroactively void every certificate it ever issued.

**The issuer name now comes from the document, because the contract has none.** A generation-2 clone's
`name()` is empty by construction, so the page reports *"the issuing contract could not be read"* and
`onchainNameAvailable` is `false`. A previous revision of this guide recorded an issuer-name *mismatch*
being reported here; on the launch set there is no on-chain name to disagree with, so
`documentNameDiffers` is `false` and no mismatch is shown. The property that the name sits outside the
Merkle root is unchanged - §7.2 shows it from the attacker's side.

All reads here are gasless, and no owner and no phone are involved: this checks a document, not a consent.

The same check is available unauthenticated over HTTP, which is what the mandatory issuer-whitelist
pillar guards:

```bash
curl -s -X POST http://127.0.0.1:44832/v1/verify -H 'content-type: application/json' \
  --data '{"wrapped_doc": <the document>}'
```

**The field is `wrapped_doc`**; a camelCase key answers 422 naming it.

### 5.2 Issue - **walked, but NOT with the provisioning script, which no longer works**

**On a stock stack this is disabled and it says so.** `/health` reports `canSign: false` and
`issuers: { TRAVEL_CLEARANCE: null, EU_HEALTH_CERT: null }`, matching the preflight warning in §0.8.
Anchoring needs three things a plain `demo-up.sh` does not create: a funded government signer, that
signer authorised, and a contract to anchor into.

**`scripts/demo-provision-government.sh` is broken against the launch set, in three independent ways** -
walked by reading it rather than by running it:

| It calls | The launch set |
|---|---|
| `IssuerRegistry.whitelistFor(bytes32,address)` | `ProviderRegistry` does not implement it on the issuance axis |
| `createIssuer(string,bytes32,address)` | the factory's signature is `createIssuer(bytes20,bytes32,uint96)` |
| checks the factory's `owner()` | the launch factory has no `owner()` and the call reverts |

So do it the way §1 and §2 already describe, and point the backend at the result:

1. A TRAVEL_CLEARANCE clone must exist, be attached and active, and its intended signer must satisfy
   **both** layers of §3.0. Confirm with `canIssue` and `issuanceAllowed`.
2. Put the clone and the signing key in `contracts/.env` and restart:

```
TRAVEL_CLEARANCE_ISSUER_ADDR=<the clone>
GOV_SIGNER_KEY=<a key satisfying both layers of §3.0>
```

**Restart the government backend afterwards** - it reads those at boot. Walked, `/health` then flipped to
`canSign: true` with the clone named under `issuers.TRAVEL_CLEARANCE` and the signer echoed.

**Then issue.** **Fill demo data** → **Issue + anchor**. Walked result:

> **✓ anchored on-chain**
> root `0x161590867c61…`
> Receipt **PH2G6TST0WY4** - *Official CDC-modeled travel-clearance receipt (printable / phone-showable).*
> **View / print receipt →**
> Public status page (PII-free, what the QR encodes): `http://localhost:44831/r/PH2G6TST0WY4`
> **Hand the credential to the owner:** Create QR · Copy wrapped document

Confirmed independently from the chain: the transaction succeeded, `isValid(root)` on the
TRAVEL_CLEARANCE clone answers `true`, and `issuedBy(root)` is the configured government signer.

**The public status page is the part worth opening.** It is unauthenticated and deliberately PII-free -
the whole point being that the QR on a printed receipt can be checked by anyone without exposing the
traveller:

```
curl -s http://127.0.0.1:44832/v1/receipts/<receiptId>/status
```

Walked: `effectiveStatus: VALID`, the record type, `validUntil`, an `issuanceDate` derived from the
chain, the root, the issuing contract, `chainId: 135`, `simulated: false`, an explorer link and a
`checkedAt`. Its HTML twin at `/r/<receiptId>` renders the same. **Checked rather than assumed:** the
rendered page contains none of the Section A person fields (importer name, id number, date of birth,
email) that the authenticated record carries.

> **`EU_HEALTH_CERT` has no clone on this deployment** and `/health` reports it `null`. Leave
> `EU_HEALTH_CERT_ISSUER_ADDR` unset - the API then reports that issuer as null and `/issue` dry-runs
> rather than anchoring somewhere wrong.

---

## 6. The owner receives the credential

Owner wallet, http://localhost:45931. It has **no backend** - everything is local to the browser.

Go to **Receive**, paste the wrapped document from §3.3, and click **Add to wallet**.

Walked: the wallet routes to `/credential/<root>` and shows

> 💉 **VACCINATION** · issued by Seaport Vet
> **✓ Integrity intact**
> **✓ Anchored on-chain**

with every field that was hashed into the root - microchip code and standard, vaccine product code, name
and manufacturer, batch, series, vaccination date, valid-from, valid-until, next-due, authorising vet and
dog tag id - under *"Exactly the values hashed into this credential's on-chain Merkle root."*, then a
**Cryptographic identity** block carrying the Merkle root and the issuer clone.

**The integrity check is offline and the anchor check is a direct chain read.** No server was asked
whether this credential is good.

The **Fill demo data (vaccination)** and **Fill demo data (travel)** buttons on that page are the
zero-setup path if you have not issued anything yet. They are the one place in the product where such a
button is not gated on demo mode, because the owner wallet has no demo-mode flag at all.

---

## 7. Verification, and eleven attempts to defeat it

Admin portal → **Verification bench**. This is the surface for "what exactly is checked, and what is not".

### 7.1 A genuine credential

Paste the §3.2 credential into **Wrapped document JSON** and click **Run the checks**.

Walked: **Verifier verdict: valid**, with *"Every on-chain read was pinned to block 339784"*, and nine
rows, **all Pass**:

| Check | |
|---|---|
| Is the document's content intact - does it still hash to the root it claims? | Pass |
| Was this issued by a contract that genuinely descends from the DogTag factory? | Pass |
| Does the document name the same contract the factory says issued it? | Pass |
| Was the signer that issued this authorised for this record type when it anchored the root? | Pass |
| Is this root actually anchored on-chain by its issuing contract? | Pass |
| Has the issuer revoked this credential? | Pass |
| Is this credential still within its validity window? *(not in the verdict)* | Pass |
| Is the registry this client is configured with the one that gates this issuer? *(not in the verdict)* | Pass |
| Was that signer authorised at the moment this root was anchored? *(not in the verdict)* | Pass |

**The rows marked *not in the verdict* are the point of the page.** The verifier's verdict covers
integrity, on-chain status and the issuer pillar, and nothing else. Expiry is reported beside it rather
than folded in, because the chain records anchoring and revocation and has no concept of a validity
window. So an expired-but-anchored credential legitimately shows a valid verdict above a red expiry row,
and the page marks which is which rather than leaving you to guess.

Each row also shows the evidence it rests on - which contract was asked, what it answered, and at which
block.

### 7.2 Try to break it - the mutation buttons

Each button tells one specific lie with the record you loaded and re-runs everything. **Read each
button's "what will NOT catch this" list as carefully as its result.** The first mutation, *"Relabel the
issuer's name"*, declares *"Designed to be caught by: nothing on this verification path"* - the name is
not covered by the Merkle root, so relabelling it is not detected here.

### 7.3 The attack catalogue - **Run the whole catalogue**

Eleven complete fraudulent records, each with its own scripted chain, each declaring in advance which
check must refuse it. These need no loaded record and make no network call: they exercise chain states a
live chain cannot be asked to produce - a signer delisted after it issued, a contract the factory never
deployed vouching for a root, a registry that does not govern the clone.

Walked: **all eleven matched their declared expectations**, with no divergence. Verdicts across the
eleven: three `valid`, seven `not valid`, one `no verdict`.

**One of the eleven is an honest control - a genuine credential that must verify.** The page states why
it is there: *"without it a catalogue of nothing but frauds would look perfect against a verifier that
refused everything."* That control passing is what makes the other ten results mean anything.

One scenario deliberately produces **no verdict** rather than a refusal: pointed at an endpoint on the
wrong chain, every on-chain row reports *could not run* and the verdict is withheld. "The factory has no
record of this root" would be an accusation nobody was in a position to make.

---

## 8. Reference: live reads

Read the live state rather than trusting numbers written here, and resolve every address from the ledger.

```bash
source scripts/lib/ledger.sh
RPC=https://devrpc.roax.net
PR=$(ledger_addr ProviderRegistry)

cast call $PR 'providerCount()(uint256)' --rpc-url $RPC
cast call $PR 'serviceCount()(uint256)'  --rpc-url $RPC
cast call $PR 'isResolverApproved(uint8,address)(bool)' 0 "$(ledger_addr ProviderDirectory)"     --rpc-url $RPC
cast call $PR 'isResolverApproved(uint8,address)(bool)' 1 "$(ledger_addr ServiceDomainResolver)" --rpc-url $RPC
cast call "$(ledger_addr ProviderDirectory)" 'resolverApproved()(bool)' --rpc-url $RPC
```

Walked on this stack: `providerCount` **1**; `serviceCount` **1 → 3** across this walk; both
`isResolverApproved` reads **true**; `ProviderDirectory.resolverApproved()` **true**.

The two rights reads are the ones that decide whether anything can be issued:

```bash
cast call $PR 'rightsOf(address)(uint256)' <signer> --rpc-url $RPC          # bit 0 = RIGHT_ISSUE
cast call $PR 'canIssue(address,address)(bool)' <clone> <signer> --rpc-url $RPC
cast call <clone> 'issuanceAllowed(address)(bool)' <signer> --rpc-url $RPC
```

**Mask the bit; never compare the whole word.** Bit 0 is the only settable bit today, so "the word equals
1" and "bit 0 is set" agree on every mask the contract can currently emit - which is exactly what would
let a whole-word comparison survive review until a second right is allocated.

To see the grant history, read the log. **Use raw JSON-RPC, not `cast logs`** - the latter renders extra
rows for the same query, so an empty result from it is weak evidence for a strong claim:

```bash
curl -s -X POST $RPC -H 'content-type: application/json' --data \
 '{"jsonrpc":"2.0","id":1,"method":"eth_getLogs","params":[{"address":"'"$PR"'","topics":["0xbc9c679fe541a4f3fcf5f2887c4adcd6e7703f7ea9d0933b8862662f8290af7f"],"fromBlock":"0x0","toBlock":"latest"}]}'
```

That topic0 is `RightsSet(address,uint256)`. Walked: three entries, the last of which withdraws a grant -
which is why the admin panel renders that address struck through rather than dropping it.

---

## 9. What this guide does not cover

**Anything needing the phone app.** The DogTag holder app carries a compiled Rust core and bundled
proving artifacts and is not built by `demo-up.sh`; `docs/MOBILE_BUILD.md` covers it. That means:

- **Completing the dog-tag bind (§3.1).** The QR is minted and resolves correctly; the device half was
  not performed.
- **The owner-hidden consent proof**, including the groomer's **Export on chain** (§4.6). The owner
  proves consent on-device in zero knowledge; there is no desktop equivalent.
- **Nearby provider search, the offline provider cache, and the directions handoff.** All are phone
  surfaces, and the rendered result rows additionally cannot be verified on a dev machine at all, because
  the directory host is a fixed production constant with no debug override.

**Selecting either typed resolver (§2.6, §2.7).** Both are approved by the registrar; neither is selected
by the provider, and the provider portal has no button for `setDomainResolver` / `setDirectoryResolver`.

**The microchip MATCH and MISMATCH outcomes (§4.3).** Only the neutral *not compared* state was walked.
Reaching the other two needs the shop to hold a credential for that tag, which arrives through **Import
from user** - and that form wants a customer JWT a receptionist has no way to obtain.

**The government's `EU_HEALTH_CERT` record type.** It has no clone on this deployment and `/health`
reports it `null`.

**A note on the Playwright suites.** `make e2e-web` is the launchable entry point and stands up what the
specs need. Do **not** run the specs directly and unfiltered: several are unmocked live-portal drivers
that create real records and anchor them on chain, and `vite preview` honours `server.proxy`, so serving
a portal on a port of your own does not give you a backend of your own.

---

## 10. Evidence: what was walked, when, and against what

Walked on **2026-08-04**, against commit **`3d3632f`** plus this branch's two fixes, on a live stack on
ROAX chainId 135, booted from `scripts/demo-up.sh`.
Every result quoted above as walked was observed in a browser or over `curl`/`cast` on that date.

**Walked end to end:**

- **§0** - the boot, including the chain preflight; `/health` on all six backends and all five portals;
  the `demo-up.sh` line-continuation defect reproduced (`.demo/admin-web.log` containing an environment
  dump, the script hung, four portals never started) and fixed; the `ledger.sh` zsh defect reproduced
  (`ledger_addr` empty for every key under zsh, `cast` answering `invalid value ''`) and fixed, then
  re-verified across bash/zsh × `set -u` × repo-root/subdirectory, plus the cwd-walk path, the
  outside-a-repo loud failure and the `DOGTAG_LEDGER` override; the hardcoded `LAN_IP` confirmed
  unreachable and overridden; all nine deployed contracts confirmed by code size; `getDiscoverySet`
  decoded and confirmed active.
- **§0.3** - the deploy **rehearsed on an `anvil --fork-url` fork of ROAX**: `Deploy.s.sol run()` to
  `ONCHAIN EXECUTION COMPLETE & SUCCESSFUL` with 12 transactions (9 CREATE + 3 registrar calls), then the
  two-phase publish to a nine-word `getDiscoverySet` with `active = 1` and a `circuitId` identical to the
  live one. The ledger clobber was reproduced, then restored and verified byte-identical by sha256 and by
  an empty `git diff`. The four artifact pins were confirmed to agree three ways.
- **§1** - admin sign-in; the Dashboard's "Authority map unavailable" traced to the factory having no
  `owner()`, cross-checked against `admin-api`'s own preflight log and three chain reads; the Providers
  banner; approving a record type (**Mined: approved DOG_PROFILE**); `canCreateService` re-confirmed from
  the chain in all three forms including the `--from`-less trap; **§1.3** attach, activate and grant the
  issue right, each mined and each read back.
- **§2** - the four-flow page rendering with addresses supplied by `demo-up.sh`; flow 1 checked to
  **Ready** and deployed a real DOG_PROFILE contract at exactly the predicted address, with `owner()`,
  `recordType()`, `isClone` and the #144 creator-seed all verified independently; flow 2 checked to
  **Ready** with five passing checks and `repointService` mined, taking `canIssue` from false to true.
  Flows 3 and 4 were **not** completed - see §9.
- **§3** - the two-layer issuance requirement established by chain reads and by grepping for a surface
  that does not exist; `setIssuanceAllowed` sent by hand from the clone owner; register-pet through to
  the minted QR with the `/p/` endpoint resolved; a record issued to **Verified on-chain**, then verified
  independently (`rootIssuer`, `isValid`, `issuedBy`, `recordType`).
- **§4** - the whole groomer role: the nav enumerated; a client and pet created; the pet opened; DogTag
  `1` linked and the microchip cross-check's neutral state observed; an appointment booked (with the
  Fill-demo-data pet reset confirmed against the URL that carried the pet through) and seen on the week
  view; the `.ics` diary published and **fetched**, confirming its calendar name, refresh interval and
  the booking's own `VEVENT`; and **Verify credential** run on the vet's credential to **Verdict: pass**
  with all five pillars.
- **§5.1** - government verification of the vet's credential, both in the portal and over the
  unauthenticated `POST /v1/verify`, block-pinned, with `issuerWhitelistState: "passed"`.
- **§5.2** - the government backend wired to the existing TRAVEL_CLEARANCE clone **without** the broken
  provisioning script, `/health` confirmed at `canSign: true`, a credential issued to **✓ anchored
  on-chain** and verified independently (`isValid`, `issuedBy`, receipt status 0x1 at block 339837), and
  both public receipt surfaces fetched and checked to carry no Section A person data.
- **§6** - the owner wallet receiving the vet's credential and reporting integrity and anchoring.
- **§7** - the bench on the genuine credential (nine rows, all Pass, block-pinned); the eleven-scenario
  catalogue run in full with **no divergence** (3 valid, 7 not valid, 1 no verdict).
- **§8** - every read in that section.

**Not walked, each with its reason stated in place:** everything in §9, plus:

- **`scripts/demo-provision-government.sh` was established as broken by READING it, not by running it.**
  Its three incompatibilities with the launch set are each visible in its source, and §5.2 was completed
  by the manual route instead, so running it would only have created noise on the testnet.
- **The deploy was rehearsed on a fork, not broadcast to live ROAX.** Re-broadcasting would replace a
  working set and the provisioned provider that makes §2 walkable at all, and would invalidate the
  ledger's own provenance notes. The live set was verified read-only instead. This is an honestly-marked
  unwalked step, not a gap.

**One caveat about §2, stated because it affects how much weight to put on it.** The wallet used for the
provider flows was a scripted EIP-6963 provider signing with the well-known **public** anvil test key -
the key the ledger records as the walk provider's controller precisely so anyone can act as it on a
disposable testnet. The product code exercised is identical (the same connect path, the same preflights,
the same transactions), but the wallet's own UI was not exercised, so a wallet-specific problem would not
have shown up.

**A correction worth recording, because it is the failure mode this guide is about.** Reading the admin
service panel through a flattened text dump made a *withdrawn* issue-right holder look like a current
one, and that was written down as a defect before being checked. It is not one: the withdrawn address
renders struck through beside the granted one, and the distinction is carried entirely by styling. Read
the rendered element, not a text extraction, whenever a distinction is visual.

**There is no CI for any of these paths.** The two mobile workflows are dispatch-only and no workflow runs
the Rust test suite, so a local walk is the only evidence any of this works. If you change a portal,
re-walk the section rather than assuming.
