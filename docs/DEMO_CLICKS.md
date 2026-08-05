# DogTag - boot it role by role, from nothing

This is the one file. It takes a machine with nothing installed to every use case the product
supports, **one role at a time**: you bring up a role, use it, and move on to the next.

Every step below is three things and nothing else:

- **who** is acting - the section heading names the hat,
- **do** - the command or the click,
- **means** - what just happened, in a sentence or two.

## The one rule this product is built on

**Each provider deploys and owns its own issuing contract. DogTag cannot do it for them.**
The registrar admits a provider and authorises it; the provider deploys. Neither can issue alone.

That is why the walk below goes ADMIN → PROVIDER → ADMIN → PROVIDER before a single credential
exists. It is a handshake between two parties, not a checklist for one.

| Role | Who this really is | What they do | §  |
|---|---|---|---|
| **OPERATOR** | DogTag, once | Installs the toolchain, confirms the contracts, boots each role | 0 |
| **ADMIN / REGISTRAR** | DogTag's governance key | Registers a provider, approves it, attaches its contract, grants rights | 1, 3 |
| **PROVIDER** | the vet / groomer / government business, on **their own** machine and wallet | **Deploys and owns its own issuing contract**, selects it, admits its own signing keys | 2, 4 |
| **VET / GROOMER / GOVERNMENT** | the same businesses, doing their day job | Issue credentials, check them | 5, 6, 7 |
| **HOLDER** | the pet owner | Receives credentials, consents to verification | 8 |

On one laptop you play all of them. In production they are different organisations on different
machines holding different keys.

## Where else to look

| If you want | Read |
|---|---|
| **to see the product work** | **this file** |
| to install the toolchain | [PREREQUISITES.md](./PREREQUISITES.md) |
| every service, port and flag | [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md) |
| your own server, domains, TLS | [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) |
| to build the phone apps | [MOBILE_BUILD.md](./MOBILE_BUILD.md) |
| to deploy your own contract set | [DEPLOY.md](./DEPLOY.md) |

**No contract address is written into this guide as configuration.** Every one is resolved from
`contracts/deployments/roax.json`, which the deploy writes.

**§12 records what was walked, when, and against which commit**, quoting a few addresses in shortened
form as the historical record. A step marked unwalked says so with its reason - could-not-check is
never reported as pass.

---

# 0. OPERATOR - prepare the machine

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

## 0.2 Confirm the contracts are already deployed

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
live on ROAX and this only confirms it.

## 0.3 Write `contracts/.env`

**Do:** create `contracts/.env` with exactly this:

```
ROAX_RPC=https://devrpc.roax.net
DEMO_MODE=1
GOVERNANCE_PRIVATE_KEY=<the registrar key - see below>
```

**Means:** the boot script sources this file, so it is where the two things the ledger cannot answer
live - which chain to talk to, and the registrar's key. `DEMO_MODE=1` is not cosmetic: without it
the backends refuse to boot on the dev passwords.

**`GOVERNANCE_PRIVATE_KEY` is the key that holds the registrar authorities on the deployed set.** It
is not in this repository and never will be - it is held by whoever operates DogTag, and
`contracts/deployments/roax.json` records only its address, under `admin`. Confirm you have the right
one:

```bash
source contracts/.env                # the file you just wrote
source scripts/lib/ledger.sh
cast wallet address --private-key "$GOVERNANCE_PRIVATE_KEY"   # must equal:
ledger_addr admin
```

**Without it you cannot do §1 or §3** - the admin role refuses to boot rather than start a portal
whose every action would silently produce unsigned calldata instead of a transaction. Everything that
does not need the registrar still works: §5 onwards if a provider is already set up, and §6 to §9 in
full.

> Put **no contract address in this file**. Every protocol address is resolved from the ledger by
> name, and an address here silently overrides it - which is how a stale one survives a redeploy while
> naming a contract that decides nothing. Two more variables get added later, in §4.4, and they are
> the only two that ever belong here.

## 0.4 How you boot a role

**Do:** nothing yet - this is the shape every boot below takes.

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh <role>     # macOS; use your own LAN address
scripts/demo-down.sh <role>                                    # stop just that one
scripts/demo-down.sh                                           # stop everything
```

Roles: `admin` `vet` `groomer` `government` `owner` `prover` `indexer`. No argument boots all of
them, which is fine once you know the product and is the wrong way to meet it.

**Means:** each role starts and stops on its own, records its own process ids, and refuses to start
on a port something already holds. Booting one role never disturbs another.

> **Set `LAN_IP`.** It is stamped into every QR the stack mints, and a phone cannot reach
> `localhost` - that is the phone itself. Its default is a hardcoded address that is almost
> certainly not yours.

---

# 1. ADMIN - register a provider

> In production this is DogTag's governance key, at DogTag. It admits providers to the protocol. It
> does not deploy their contract and cannot admit a signing key to one.

## 1.0 Boot the admin role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh admin
```

**Means:** `admin-api` on `:39742` and the admin portal on `:39741`. The preflight checks the chain
id and that your governance key holds `WHITELIST_ADMIN`; if it does not, it refuses here rather than
booting a portal whose every action would silently produce unsigned calldata instead of a
transaction.

## 1.1 Sign in

**Do:** open http://localhost:39741 and click **Sign in** (the password is prefilled in demo mode).

**Means:** you land on the Dashboard. Its business counters read 0 until you register someone.

## 1.2 Register the provider

**Do:** **Providers** in the left nav. Read the banner first - it must say *"registrar actions execute
directly"*. Then **Register provider** → **Fill demo data** → **Review** → **Register**.

> If the banner instead says actions route to governance as proposals, your `GOVERNANCE_PRIVATE_KEY`
> is not the registrar key (§0.3). Nothing is broken, but nothing below will reach the chain either -
> each action returns unsigned calldata for someone else to execute. Fix the key and reboot the role.

**Means:** the provider now exists on chain with standing PENDING. `Fill demo data` mints a random
provider id, sets the controller to the published anvil test account whose key §2.3 gives you, and
writes an obviously-fake identity statement; **Review** hashes that statement, and only the hash
reaches the chain.

> **Copy the provider id.** §2 needs it, and it can never be reassigned. The statement text is stored
> nowhere and disappears when you navigate away.

## 1.3 Activate it

**Do:** the new row reads **pending**. Click **Activate**.

**Means:** standing goes PENDING → ACTIVE. Until it does, the provider is inert: every self-service
action refuses, so registering alone grants nothing.

## 1.4 Approve the record types it may create

**Do:** on the same row, click **DOG_PROFILE**. Then click **VACCINATION**.

**Means:** the registrar has said which kinds of contract this provider may deploy. It is per record
type - approving one approves no other. The vet needs both: DOG_PROFILE anchors dog tags,
VACCINATION anchors vaccination records.

---

# 2. PROVIDER - deploy your own contract

> **Switch hats.** You are now the veterinary business. In production you are a different company on
> your own machine with your own wallet. `createIssuer` takes no owner argument, so whoever deploys
> the contract owns it - which is why DogTag cannot do this step for you.

**The provider's self-service page is served by the vet portal**, so you boot the vet role to act as
the provider. On a real deployment the business runs that portal itself; here it is the same laptop.

## 2.0 Boot the vet role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh vet
```

**Means:** `vet-api` on `:41874` and the vet portal on `:41873`. It warns that two contract addresses
are unset and boots anyway - those are the contracts you are about to deploy, so nothing else could
be true yet. Issuing refuses until §4.4 sets them; everything in this section works without them.

## 2.1 Sign in

**Do:** open http://localhost:41873 and click **Sign in** (prefilled in demo mode).

**Means:** you are in the vet portal as its operator.

## 2.2 Create or unlock the shop's signing key

**Do:** the top of the page says *Custody is locked*.

- **Seen it before on this machine?** Click **Unlock**, then **Unlock and continue** - both fields
  are prefilled.
- **First run?** There is no key yet. Go to **Setup**, sign in with the admin password (also
  prefilled), and follow the wizard: **genesis** → confirm → unlock. It shows you 24 words once,
  makes you tick that you wrote them down, then asks you to re-type three of them and choose an
  encryption passphrase - so have somewhere to write before you press **Generate 24-word seed**.

Then open **Signing keys** in the nav and **copy the address in the line "This shop signs with …".**

**Means:** the backend now holds a key it can sign with. The sealed key is stored on disk and the
decrypted one never is, which is why custody re-locks on every restart of this backend.

That page also says *"No issuing contract is configured on this deployment"*, which is correct right
now - you have not deployed one yet. It fills in after §4.4.

> Do this before deploying, even though deploying does not need it: §3.3 has to grant a right to that
> address, and the address does not exist until the key does.

## 2.3 Connect your wallet

**Do:** you need a browser wallet holding the controller address from §1.2. Install one (MetaMask,
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
> `Fill demo data` put in the controller field in §1.2 precisely so anyone can act as this provider
> on a disposable testnet. It is public by design and holds testnet funds only. Never put a real key
> in a browser.

## 2.4 Deploy the DOG_PROFILE contract

**Do:** paste your provider id, set **Record type** to `DOG_PROFILE`, leave **Contract number** at
`0`, click **Check what this would deploy**, then **Deploy** and confirm in your wallet.

**Means:** you own a `DogTagIssuer` contract. The check shows the exact address before anything is
sent, because the address is computed from the record type, your wallet and that number - so
**Deploy** only ever sends what a check approved, and goes back to disabled if you edit a field.

> **If the check refuses** with *"that one number's address is simply taken"*, you have deployed this
> record type from this wallet before. Put `1` in **Contract number** and check again - each number
> gives one fixed address, and two contracts cannot share one.

> **Copy the deployed address.** §3.1, §4.1 and §4.4 need it.

---

# 3. ADMIN - attach and authorise the contract

> Back to the admin portal, still running from §1. These four actions are `onlyOwner` on the provider
> registry: no provider key can perform any of them.

## 3.1 Attach it

**Do:** **Providers** → **Show services** on your provider's row → **Attach a contract**. Paste the
address from §2.4, **Check**, then **Attach**.

**Means:** DogTag has admitted this contract to that provider's record. The address is the only thing
you type: the check works out which factory deployed it and reads its record type and owner off the
chain. Until it is attached, the provider cannot select it.

## 3.2 Activate the service

**Do:** click **Activate** on the service row.

**Means:** attaching lands the service at PENDING, exactly as registering did for the provider, so
attaching alone grants nothing.

## 3.3 Grant the issue right

**Do:** click **Issuance capability** on the service row, enter the signer address from §2.2, pick
**Grant**, confirm.

**Means:** that address may now sign issuances. The grant is on the **address**, registry-wide - it
names no service. That is deliberate, and it is exactly why issuing needs a second permission that
only the provider can give (§4.2).

## 3.4 Approve the typed resolvers

**Do:** in **Typed resolvers**, confirm both kinds show their address in the success style. Confirm
from the chain if you want certainty:

```bash
source scripts/lib/ledger.sh
PR=$(ledger_addr ProviderRegistry); RPC=https://devrpc.roax.net
cast call $PR 'isResolverApproved(uint8,address)(bool)' 0 "$(ledger_addr ProviderDirectory)"     --rpc-url $RPC
cast call $PR 'isResolverApproved(uint8,address)(bool)' 1 "$(ledger_addr ServiceDomainResolver)" --rpc-url $RPC
```

**Means:** the registrar has allowed these two resolvers fleet-wide, which is its half of the
provider's domain and listing flows. Both read `true` on the deployed set, so there is nothing to
send. The provider's half - selecting one - has no button yet (§11).

---

# 4. PROVIDER - select it, admit your key, point the backend at it

## 4.1 Make it your current contract

**Do:** vet portal → **Provider self-service** → flow 2. Paste the address from §2.4 into
**Contract address**, **Check this contract**, then **Make this my current contract**.

**Means:** new credentials of that record type now anchor here. This is the move that makes the
chain's `canIssue` finally answer true - before it, every other term held and this one did not.

## 4.2 Admit your signing key to the contract

**Do:** **Signing keys** in the nav. Connect the wallet that **owns** the contract, click **Use this
shop's signing key** under it, then **Admit** and confirm.

**Means:** issuing needs **two** permissions and this is the second. The registrar granted the right
in §3.3; this list is the contract owner's own, and the protocol admin is deliberately excluded from
writing it - so neither party can put a signer on somebody else's contract alone.

> A freshly deployed contract already admits whoever deployed it. The vet's backend signs with a
> *different* address - its custody signer - so this step is always needed for it.

## 4.3 Do §2.4 → §4.2 again for VACCINATION

**Do:** repeat the deploy (§2.4 with **Record type** `VACCINATION`, contract number `0` again), then
the admin's §3.1–§3.3, then §4.1 and §4.2.

**Means:** you now own two contracts - one for dog tags, one for vaccination records. The same
contract number gives a different address because the record type is part of the computation.

## 4.4 Tell the backend which contracts it anchors into

**Do:** add both addresses to `contracts/.env`, then restart just the vet:

```
PROFILE_ISSUER_ADDR=<the DOG_PROFILE contract from §2.4>
VACCINATION_ISSUER_ADDR=<the VACCINATION contract from §4.3>
```

```bash
scripts/demo-down.sh vet
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh vet
```

**Means:** the backend reads these at boot, so it learns where to anchor only on a restart. The boot
now names both contracts instead of warning about them.

---

# 5. VET - register a pet and issue a credential

## 5.0 Unlock custody

**Do:** sign in at http://localhost:41873, click **Unlock** in the banner, then **Unlock and
continue**.

**Means:** the restart in §4.4 re-locked it. Nothing signs or issues until you unlock.

## 5.1 Register the pet

**Do:** **Register pet** in the nav → **Fill demo data** → **Start issuance**.

**Means:** a dog tag is allocated and the page shows a QR plus a URL on your `LAN_IP`. The owner's
device scans it, folds the profile tree on the phone and posts only the resulting root - the vet
never holds the owner's secret, which is why a desktop cannot finish this step (§11).

> **Note the dog tag id** (`1` on a fresh set). §5.2 and §6.3 use it.

You can see exactly what the phone would receive:

```bash
curl -s http://127.0.0.1:41874/p/<the token in that URL>
```

## 5.2 Issue a record

**Do:** **Issue a record** in the nav → **Fill demo data** → set **Dog tag id** to the one from
§5.1 → **Sign & Issue**.

**Means:** a rabies certificate is built, its Merkle root is anchored on chain by your VACCINATION
contract, and the page reports **Verified on-chain**. A record can be issued while the dog tag is
still waiting for its device bind - the two are anchored independently.

## 5.3 Take the credential out

**Do:** **Records** in the nav lists what this shop has issued, each with an explorer link. To carry
one elsewhere, fetch it. The bearer token is the value of `vet.opToken` in the portal's local storage
(DevTools → Application → Local Storage → `http://localhost:41873`):

```bash
TOKEN=<paste vet.opToken here>
curl -s http://127.0.0.1:41874/records -H "Authorization: Bearer $TOKEN" \
  | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["records"][0]["wrapped_doc"]))'
```

**Means:** that one JSON object *is* the credential - the field is `wrapped_doc`, snake_case, and the
line above pulls it out of the newest record. **Save it**: §6.5, §7.1, §8.1 and §9.1 each paste a copy
of it.

---

# 6. GROOMER - the shop, and checking a credential it did not write

> A groomer verifies and does not issue. Its backend is the same binary as the vet's with
> `BUSINESS_TYPE=groomer`, and the issuance routes are not mounted at all - so it has no Records page
> and no Issue page.

## 6.0 Boot the groomer role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh groomer
```

Then sign in at http://localhost:43617.

**Means:** a second, independent business on `:43617` / `:43618`, with its own store and its own
signing key. The vet is still running and untouched.

> **Leave custody locked.** The banner will say it is, and nothing in this section needs it: a
> groomer only ever reads the chain, and unlocking exists to let a backend *sign*.

## 6.1 Create a client and their pet

**Do:** **Clients** → **New client** → **Fill demo data** → **Create client**.

**Means:** a client record with an inline pet. Every demo value says it is a demo - a receptionist
must never mistake a filled row for a real person.

## 6.2 Open the pet

**Do:** **Pets** in the nav, then click the pet.

**Means:** a pet is addressable in its own right, with its own profile, owner link, DogTag block and
Credentials block.

## 6.3 Link the DogTag and read the microchip cross-check

**Do:** under **DogTag**, type the dog tag id from §5.1 and click **Add DogTag**.

**Means:** the tag is linked, and the shop compares the microchip on its own pet record against the
microchip inside a credential *it holds* for that tag. It holds none - the vet's store is a different
business's - so it reports **Microchip not compared** rather than a pass. A mismatch would refuse the
link and would not change any credential's verdict: the credential is still genuine, it just
describes a different animal.

## 6.4 Book an appointment and publish the diary

**Do:** from the pet page click **Book appointment** → **Fill demo data** → set the **Pet** select
back to the pet → **Book appointment**. Then **Calendar** → **Calendar sync** → **Publish calendar
address**.

**Means:** the booking appears on the calendar, and the shop's diary is published at one secret URL
any calendar app can subscribe to. Treat that URL like a password - it is unauthenticated by
necessity, because a calendar client cannot present a token.

> `Fill demo data` clears the pre-selected pet. That is legal - an appointment need not name one -
> but set it back before booking.

## 6.5 Check the vet's credential

**Do:** **Ad-hoc verification** → paste the `wrapped_doc` from §5.3 into **Verify credential** →
**Verify credential**.

**Means:** **pass**, with five checks: integrity, on-chain valid, issued, not revoked, and issuer
authorised at issuance. No owner and no phone are involved - it recomputes the document's hash
offline and reads the issuing contract directly. This is the cross-business check that matters: the
shop asserting the record is not the shop that wrote it.

---

# 7. GOVERNMENT - verify somebody else's credential, then issue its own

> A government authority is a provider like any other here. Nothing about it is privileged.

## 7.0 Boot the government role

**Do:**

```bash
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh government
```

**Means:** `government-api` on `:44832`, its portal on `:44831`. Its own binary, its own store. The
preflight tells you it can read the chain but not sign, which §7.2 fixes.

## 7.1 Verify the vet's credential

**Do:** open http://localhost:44831, paste the `wrapped_doc` from §5.3 into **Verify a document
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

## 7.2 Issue a travel clearance

**Do:** the authority needs its own contract, so walk the same handshake once more for record type
`TRAVEL_CLEARANCE`: §1.4 to approve it, §2.4 to deploy it, §3.1–§3.3 to attach, activate and grant,
then §4.1 to select it.

**In §3.3, grant the issue right to the wallet that deployed this contract** - not to the vet's
signer. The government backend signs with that same wallet's key, which is what the next block sets.

Then point the backend at it and restart:

```
TRAVEL_CLEARANCE_ISSUER_ADDR=<the contract from §2.4>
GOV_SIGNER_KEY=<the private key of the wallet that deployed it>
```

```bash
scripts/demo-down.sh government
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh government
```

Then **Fill demo data** → **Issue + anchor**.

**Means:** `/health` flips to `canSign: true` and the credential is anchored on chain. You get a
printable CDC-modelled receipt with a QR pointing at a public status page.

> **There is no §4.2 step here, and that is not an omission.** A contract admits its own deployer
> automatically, so making the government sign with the wallet that deployed its contract satisfies
> the second permission with nothing to click. The vet needs §4.2 only because its backend signs with
> a key it generated itself, which is a *different* address from the one that deployed. Confirm
> either way:
>
> ```bash
> cast call <the contract> 'issuanceAllowed(address)(bool)' <the signer> --rpc-url https://devrpc.roax.net
> ```

## 7.3 Open the public status page

**Do:**

```bash
curl -s http://127.0.0.1:44832/v1/receipts/<the receiptId>/status
```

**Means:** anyone can check that receipt without an account and without seeing the traveller. The
page carries the status, the record type, validity and the chain reads - and none of the Section A
person data the authenticated record holds.

---

# 8. HOLDER - the owner receives the credential

> **Switch hats.** You are the pet owner. No operator, admin or provider can act for you.

## 8.0 Boot the owner wallet

**Do:**

```bash
scripts/demo-up.sh owner
```

**Means:** http://localhost:45931, and **no backend at all** - everything is local to the browser.
This is the only role that needs no chain preflight and no governance key.

## 8.1 Receive it

**Do:** **Receive**, paste the `wrapped_doc` from §5.3, click **Add to wallet**.

**Means:** the wallet shows **✓ Integrity intact** and **✓ Anchored on-chain**, then every field
that was hashed into the root. The integrity check is offline and the anchor check is a direct chain
read - no server was asked whether this credential is good.

---

# 9. ANYONE - the verification bench

> Verification is permissionless. It reads the chain and needs no role. The bench lives in the admin
> portal only because that is where it was built.

## 9.1 Run the checks on a real credential

**Do:** admin portal → **Verification bench** → paste the `wrapped_doc` from §5.3 → **Run the
checks**.

**Means:** nine rows, each showing which contract was asked, what it answered and at which block.
Three of them are marked *not in the verdict*: the verifier's verdict covers integrity, on-chain
status and the issuer pillar, and nothing else. Expiry is reported beside it rather than folded in,
because the chain records anchoring and revocation and has no concept of a validity window.

## 9.2 Try to defeat it

**Do:** click the mutation buttons, then **Run the whole catalogue**.

**Means:** each mutation tells one specific lie with your record and re-runs everything, declaring in
advance what will *not* catch it - relabelling the issuer's name is caught by nothing on this path,
because the name is not covered by the Merkle root. The catalogue is eleven complete fraudulent
records with their own scripted chains, each declaring which check must refuse it. **One of the
eleven is a genuine credential that must pass** - without it, a verifier that refused everything
would look perfect.

---

# 10. Reference: reading the live state

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
```

Health, per role:

```bash
for p in 39742 41874 43618 44832 41875 46001; do printf "%-6s " $p; curl -s http://127.0.0.1:$p/health; echo; done
for p in 39741 41873 43617 44831 45931; do printf "%-6s %s\n" $p "$(curl -s -o /dev/null -w '%{http_code}' http://localhost:$p/)"; done
```

> Use `localhost` for the portals, not `127.0.0.1`. Vite dev servers bind IPv6 only, so a
> `127.0.0.1` probe reports a healthy portal as dead.

Two more roles exist that the walk above never needs: `indexer` (the oversight event feed, which
serves the portals' Traceability and Oversight pages) and `prover` (a server-side consent prover for
phones that cannot prove locally).

---

# 11. What this guide does not cover

**Phone surfaces.** The holder app carries a compiled Rust core and bundled proving artifacts and is
not built by `demo-up.sh` - [MOBILE_BUILD.md](./MOBILE_BUILD.md) covers it. Some of it is walked on a
real handset; see §12.2. Specifically **not** walked: the dog-tag device bind (§5.1), and any
completed owner-hidden consent proof, including the groomer's **Export on chain**.

**Selecting a typed resolver** (the provider's half of §3.4). Both are approved by the registrar;
the provider portal has no button for `setDomainResolver` / `setDirectoryResolver` yet, so flows 3
and 4 of the provider page cannot be completed.

**The microchip MATCH and MISMATCH outcomes** (§6.3). Only the neutral *not compared* state is
reachable, because the other two need the shop to hold a credential for that tag, which arrives
through **Import from user** - and that form wants a customer JWT a receptionist has no way to
obtain.

**The government's `EU_HEALTH_CERT` record type.** No contract for it exists on this deployment, so
`/health` reports it `null` and `/issue` dry-runs rather than anchoring somewhere wrong.

**`scripts/demo-provision-government.sh` does not work against this contract set** and §7.2 does not
use it. It calls `whitelistFor`, a three-argument `createIssuer`, and the factory's `owner()` - none
of which the launch set implements.

**The Playwright suites.** `make e2e-web` is the launchable entry point. Do not run the specs
directly and unfiltered: several are unmocked live-portal drivers that create real records and anchor
them on chain.

---

# 12. Evidence: what was walked, when, and against what

## 12.1 The role-by-role walk, 2026-08-05

Walked on **2026-08-05** against commit **`03b410c`** plus this branch, on the live ROAX set
(chainId 135), starting from nothing running and bringing up one role at a time in the order above.

**The boot, which is what this revision changed.** `contracts/.env` was reduced to the three lines
§0.3 names and the vet role was booted with **no contract addresses at all** - the genuine cold start,
which the previous revision could not do. It came up, warned about both contracts by name, and served
the portal; `/health` answered `{"status":"ok"}` and the portal `200`. On that stack the **Provider
self-service page rendered all four flows**, which is what makes §2 reachable before any contract
exists and is the whole reason the ordering could change. Also checked: a second `demo-up.sh vet` is
refused by port with the role's own stop command named; bringing up `admin` alongside left the vet's
recorded pids untouched; and `demo-down.sh admin` left the vet serving.

**Boot-everything still works**, checked separately with a bare `scripts/demo-up.sh` from a clean
machine: all seven roles started, seven pid files, six backends answering `/health` and five portals
`200`, with `indexer` and `prover` - the two roles the walk above never needs - among them and neither
log carrying a stack trace. `scripts/demo-down.sh` then stopped all eleven processes and left nothing
listening.

**Both branches of §2.2 were walked, including the one a fresh clone hits.** The custody seal was
moved aside to put the vet in genuine first-run state, and the Setup wizard driven through
**genesis → confirm → unlock**: it showed 24 words once, required the written-down acknowledgment,
challenged three of them by position, took an encryption passphrase, and produced a working signer
that **Signing keys** then named. The original seal was restored afterwards and verified byte-identical
by sha256.

**§0** - `0.2` run verbatim under zsh: nine contracts, each with code, and `getDiscoverySet` returning
a nine-word record whose last word is `1`. `0.3`'s key check confirmed the configured key resolves to
the ledger's `admin`.

**§1** - admin booted alone; the Providers banner read *"The hosted admin key holds this registry -
registrar actions execute directly."* The provider row showed standing **active**, *"Cleared to act."*,
and controller `0xf39f…2266` - the published anvil account §1.2 and §2.3 name. §1.4 was driven for
real, both directions, on the existing provider:

| what | transaction | block |
|---|---|---|
| approve `GROOMING` | `0x4e83a73a…78946290` | 349050 |
| withdraw `GROOMING` | `0x457556c1…b1294c64` | - |

It was withdrawn again deliberately, to leave the shared set exactly as it was found.

**§2** - the vet portal signed in, custody unlocked, and **Signing keys** named the signer
`0x7e3a6603…0c436d` while honestly reporting *"No issuing contract is configured on this deployment"* -
the state §2.2 describes. A wallet holding the controller was connected on ROAX, and §2.4's check ran
twice: at contract number `0` it **refused** with *"that one number's address is simply taken"* (that
provider deployed there on a previous walk), and at `1` it returned **Ready** with the exact address
`0x14A09008…DEF3a` computed before anything was sent.

**§4.4 → §5** - both contract addresses added to `contracts/.env` and the vet role restarted; the boot
then named both instead of warning. Custody re-locked and was unlocked again, as §5.0 says. A pet was
registered (**dog tag 1**, QR on the real `LAN_IP`), the `/p/<token>` endpoint returned the block the
phone would receive with `issuerClone` equal to the DOG_PROFILE contract, and a record was issued to
**Verified on-chain**:

| what | value |
|---|---|
| root | `0x0b685f56de169c10d233c98f932783305e926381f30ec6ee4f93e2f868d44ee7` |
| tx | `0x4800839b963a78b0f18196c3c387920acb94618042d158b015c0820fa21b4e10` |

Read back independently with `cast`: `rootIssuer(root)` = the VACCINATION contract
`0xdD1533D6…605d57`, `isValid(root)` = **true**, `issuedBy(root)` = `0x7E3A6603…0C436D` (the same
signer the portal named), and `recordType()` = `keccak256("VACCINATION")`.

**§6** - the groomer booted as a second business while the vet kept serving. A client and pet were
created, the pet opened, and dog tag `1` linked, giving the neutral **"Microchip not compared - this
shop holds no credential for that DogTag"**. §6.5 verified the vet's credential to **Verdict: pass**
with all five pillars and root == recomputed root. **All of §6 was done with custody LOCKED**, which is
why §6.0 now tells you to leave it that way - the previous revision asked for an unlock that does
nothing here.

**§7** - government booted alone and reported `backend: live`, `chainId: 135`, `canSign: false`,
both issuers `null`. §7.1 verified the vet's credential in the portal (**✓ VALID**) and over the
unauthenticated `POST /v1/verify` (`verdict: true`, `issuerWhitelistState: "passed"`, block-pinned).
§7.2's claim that the deployer seed satisfies the second permission was confirmed on chain before
relying on it: the contract's `owner()` and the government signer are the same address, and
`issuanceAllowed` and `canIssue` both read **true** with nothing clicked. After pointing the backend at
the contract and restarting, `/health` flipped to `canSign: true` and a travel clearance anchored:

| what | value |
|---|---|
| root | `0x1c8cb68e2e95fca0a2ff8d61fd26dedf685b284eff3f7f589a0fc22f7fa2daee` |
| tx | `0x67fca6240d3b5fe4a18f6ea02140d62f3004e06d800fc871a6358994d2cc7731` |
| receipt | `W0ZJH1A2EH57` |

§7.3's public status page returned `effectiveStatus: VALID`, `simulated: false`. **Checked rather than
assumed:** its HTML twin at `/r/<receiptId>` is 1,739 bytes and contains none of the Section A person
fields, nor even an `@`.

**§8** - the owner wallet booted with no chain preflight and no governance key, took the credential,
and rendered **✓ Integrity intact** and **✓ Anchored on-chain** with every hashed field, including
`DOG TAG ID 1`.

**§9** - the bench on that credential: **nine rows, all Pass**, every read pinned to block 349139. The
attack catalogue ran in full: **11 scenarios, 11 matched their declared expectation, 0 divergences**
(3 valid, 7 not valid, 1 no verdict), the genuine control among them.

**Two defects this walk found in the boot script, both fixed here.** The government preflight asked
`isWhitelistedFor(recordType, signer)`, which the launch authority answers off its orthogonal VERIFY
axis - so it printed `whitelisted=false` and warned that `issue()` would revert, about a signer whose
`canIssue` and `issuanceAllowed` were both true and which issued successfully minutes later. It now
asks those two directly. And the preflight printed the configured chain endpoint even for a role that
never contacts it, which reads as a check that passed; it now prints that line only when a check runs.

**Not re-walked on this pass, with reasons:**

- **Deploying a second contract and repointing to it (§2.4's Deploy, §3.1–§3.3, §4.1).** The check was
  driven to **Ready**, but deploying and then selecting it would move this provider's current
  DOG_PROFILE pointer away from the contract the machine's `contracts/.env` names, breaking a working
  setup on a shared testnet for no new evidence. Those transactions were walked on 2026-08-04 and are
  recorded in §12.3.
- **Registering a brand-new provider (§1.2).** The existing provider was used instead, so the register
  dialog itself was not re-submitted; §1.4 was driven for real against it. §12.3 covers registration.
- **Everything in §11**, for the reasons given there.

The wallet was again a scripted EIP-6963 provider signing with the published anvil test key, injected
into the vet portal's `index.html` for the walk and reverted afterwards; the file is byte-identical to
its committed form. `contracts/.env` was modified during the walk and restored byte-identical,
confirmed by sha256.

## 12.2 On a real handset, 2026-08-05 - the anchor resolves, and the refusal is real

Walked on **iPhone 15 Pro "KZG"** (device `F2A840C4-DFA6-5C37-AF64-77DBCA7C2B12`, udid
`00008130-001A08E40021401C`), iOS 26.6, against commit **`fd26154`**. The app was built for a real
device and installed - the first rebuild since the contracts were replaced, which is what made any of
this reachable: the previous build bundled a superseded `ProtocolRegistry`, so discovery failed
closed for a reason that was not the real one.

**The anchor.** `apps/ios/DogTag/roax.json` was regenerated from the ledger by
`scripts/gen-mobile-roax-config.sh`, and the build carries **`ProtocolRegistry
0xc385F939…76A60`** - matching `contracts/deployments/roax.json`. The
Profile screen renders it on the device, alongside `ProviderRegistry 0x1ff6FdCeFf15AC…`, `DogTagSBT
0xFc33A3e702b7d6…` and `ROAX (chainId 135)`.

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

That message comes from the catch after `validateDiscovery`, which is reached **only** past the guard
that requires both `getDiscoverySet` and `getActiveArtifactSet` to have returned decodable records.
So both anchor reads succeeded over the bundled endpoint, and the `135` in the message is the
anchor's own value. **The refusal is correct**: the verifier was deliberately running a simulated
chain and honestly advertised `chainId 0`, and the anti-redirect check refused the mismatch.

Two things this does **not** establish: no Groth16 consent proof was generated (the flow refused
before proving), and no dog-tag bind was performed.

## 12.3 On the desktop stack, 2026-08-04

Walked on **2026-08-04** against commit **`3d3632f`** and, for the Signing keys page, `d0f8cd8`, on
a live stack on ROAX chainId 135.

**The two-layer issuance requirement** (now §4.2) was driven entirely from the browser, and each step
confirmed independently with `cast`:

| what | transaction | block |
|---|---|---|
| **Remove** the vet's signing key | `0xe31c29c7…c20ab86c` | 340437 |
| Issue a record, which is then **refused on chain** | reverted `0xa649bcb3` | - |
| **Admit** it again | `0xd144be4a…d0192c25` | 340452 |
| Issue the same record, which **anchors** | `0xcaaa6cfd…d9397e41` | 340456 |

The refusal is the load-bearing half: between blocks 340437 and 340452 the registry answered
`canIssue == true` while the contract's own list answered `false`, and `cast sig NotLocallyAllowed()`
is `0xa649bcb3` - so the page's warning was a true statement about what the chain would do.

`scripts/demo-bootstrap.sh` was run against the groomer's signer. It granted the registry-wide right
through `setRights` and three VERIFY purposes through `setVerifierCapability` (each read back
`canVerify == true`), granted the SBT `ISSUER_ROLE`, then **exited 1** naming the Signing keys page,
because the second permission is one the governance key is not allowed to write:

| what | transaction | block |
|---|---|---|
| fund the signer (0.5 PLASMA) | `0x1cf82caa…92db27f2` | 340470 |
| `setRights(signer, RIGHT_ISSUE)` | `0xe63e5171…16cb6c09` | 340471 |
| `grantRole(ISSUER_ROLE, signer)` on the SBT | `0x8ac9df6e…62483f56` | 340472 |
| `setVerifierCapability(grooming_intake, …)` | `0x616cdd61…36e8ba1` | 340473 |
| `setVerifierCapability(boarding_intake, …)` | `0xf0f2f059…7c85bd24` | 340474 |
| `setVerifierCapability(daycare_access, …)` | `0xb5f445b0…35d5b0ae` | 340475 |
| fund again - the second run's one non-idempotent write | `0x154db47d…6805b519` | 340477 |

**Walked end to end on that date:** the boot and every `/health`; the deploy rehearsed on an
`anvil --fork-url` fork of ROAX to `ONCHAIN EXECUTION COMPLETE & SUCCESSFUL` (12 transactions) plus
the two-phase publish, with the ledger restored and verified byte-identical; admin sign-in, approving
a record type, and attach/activate/grant each mined and read back; the provider page's flow 1
deploying a real DOG_PROFILE contract at exactly the predicted address, and flow 2's `repointService`
taking `canIssue` from false to true; register-pet through to the minted QR; a record issued to
**Verified on-chain** and verified independently (`rootIssuer`, `isValid`, `issuedBy`, `recordType`);
the whole groomer role including the published `.ics` diary fetched and checked; government
verification of the vet's credential both in the portal and over the unauthenticated `POST /v1/verify`;
a government credential issued to **✓ anchored on-chain** with both public receipt surfaces confirmed
to carry no person data; the owner wallet receiving the vet's credential; and the bench on the genuine
credential (nine rows, all Pass) plus the eleven-scenario catalogue run in full with no divergence.

**The wallet was a scripted EIP-6963 provider signing with the published anvil test key** - the key
the ledger records as this provider's controller precisely so anyone can act as it on a disposable
testnet. The product code exercised is identical: the same connect path, the same preflights, the
same transactions. But no real wallet's own UI was driven, so a wallet-specific problem would not
have shown up.

**Not walked, with reasons:** everything in §11; and the deploy was rehearsed on a fork rather than
broadcast to live ROAX, because re-broadcasting would replace a working set and invalidate the
ledger's own provenance notes.

**There is no CI for any of these paths.** The two mobile workflows are dispatch-only and no workflow
runs the Rust test suite, so a local walk is the only evidence any of this works. If you change a
portal, re-walk the section rather than assuming.
