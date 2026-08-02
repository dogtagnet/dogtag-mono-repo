# DogTag - a click-through, one role at a time

This guide was written by walking the product on a live stack against ROAX (chainId 135), not by
reading the source and reasoning about it.
Every step below that is marked as walked was performed, and what is written is what appeared on the
screen.
Where a step could **not** be walked, it says so in that step, with the reason and with whatever you
would need in order to walk it yourself.

Read that promise precisely, because it is the whole value of this document: an honestly-marked
unwalked step is fine, and an unwalked step written as though it were walked is the defect this guide
exists to avoid.
§9 records exactly what was and was not covered, and when.

The order below is the order to read it in.
Each role picks up the state the previous one left.

| § | Role | What they do |
|---|---|---|
| 0 | - | Cold start: from nothing to a running stack |
| 1 | **Admin** | Register and approve a provider |
| 2 | **The provider** | Deploy their own contract, then try to finish setting up |
| 3 | **The vet** | Register a pet, issue a credential, anchor it |
| 4 | **Government** | Verify somebody else's credential; issue their own |
| 5 | **The owner** | Receive a credential and read it |
| 6 | **Anyone verifying** | The bench, and eleven attempts to defeat it |
| 7 | - | Reference: what the four generation-2 reads mean |
| 8 | - | What this guide does not cover |
| 9 | - | Evidence: what was walked, when, against which commit |

---

## 0. Cold start

### 0.1 Bring your checkout to the tip FIRST, and rebuild

**This step is new, and skipping it fails silently.**
A stale checkout boots perfectly: every portal loads, every page you already knew about behaves, and the
one screen that changed answers **404** with nothing anywhere explaining why.

```
git fetch origin && git status -sb | head -1     # confirm you are at or above origin/main
git rev-parse --short HEAD
```

**Which backend actually changed, and why restarting the web servers is not enough.**
The admin registrar surface (§1.2) is a *backend* change - a new `provider_registry.rs`, plus
substantial additions to `chain.rs` and `routes.rs` in `stacks/admin/api`.
The portals are vite dev servers reading your source live, so they pick up web changes on their own;
**`admin-api` is a compiled Rust binary and does not.**
An `admin-api` built before that change serves a portal page whose every call 404s.

`scripts/demo-up.sh` rebuilds the backends for you, so in the normal case the fetch above is the whole
of this step.
Build by hand only if you are not using that script:

```
cargo build --release -p admin-api -p vet-api -p government-api -p indexer-api
```

**Build serially.**
Several parallel release builds of this workspace will bring a laptop to its knees; one `cargo build`
with several `-p` flags is a single build and is what you want.

### 0.2 Decide now whether you want your data to survive a restart

This is a decision, not a default, and it is much cheaper to make before you boot than after.

**Out of the box every backend uses an in-memory store.**
`demo-up.sh` sets no `MONGO_URI`, so clients, pets, appointments, records and sessions all vanish when a
backend restarts.
For a single sitting that is fine.
If you want to walk this guide over two days, or you want the shop data you create to still be there
tomorrow, set up Mongo.

**Two things are required, and each fails in its own way if you miss it.**

**(a) The binaries must be built WITH the `mongo` cargo feature.**
It is not a default feature.
A binary built without it does **not** quietly fall back to the in-memory store - it refuses to start:

```
ERROR vet_api: MONGO_URI is set but this binary was built WITHOUT the `mongo` feature;
rebuild with --features mongo or unset MONGO_URI. Refusing to start.
```

(Walked: that is the literal line, produced by running a non-`mongo` binary with `MONGO_URI` set.)
Loud is the right behaviour - the alternative is a stack that looks healthy while silently writing to
memory - but if you see it, that is what it means.
`demo-up.sh` adds `--features mongo` to its own build automatically when `MONGO_URI` is set.

**(b) Each backend needs its OWN database name, and `MONGO_URI` alone will not give you that.**
The vet and the groomer are the *same binary* with a different `BUSINESS_TYPE`.
Point both at one database and the second to boot adopts the first's custody blob - so two businesses
end up sharing one signing identity, with nothing on any screen saying so.
`demo-up.sh` gives each backend a distinct database (`dogtag`, `dogtag_vet`, `dogtag_groomer`,
`dogtag_government`), overridable per service with `MONGO_DB_ADMIN` / `MONGO_DB_VET` /
`MONGO_DB_GROOMER` / `MONGO_DB_GOVERNMENT`.

Start Mongo and point the stack at it:

```
docker start dogtag-mongo                       # or your own mongod on 27018
MONGO_URI=mongodb://127.0.0.1:27018 scripts/demo-up.sh
```

**Verify by listing the databases, not by assuming.**
After you have created a client or a record:

```
docker exec dogtag-mongo mongosh --quiet \
  --eval 'db.adminCommand("listDatabases").databases.forEach(d=>print(d.name))'
```

Walked result: `admin config dogtag dogtag_groomer dogtag_vet local`.
Mongo creates a database lazily on first write, so a backend you have not yet written through will
legitimately be absent from that list.
You can also confirm the wiring straight off the running process, which is the check that cannot lie:

```
ps eww $(lsof -nP -iTCP:41874 -sTCP:LISTEN -t) | tr ' ' '\n' | grep '^MONGO_'
```

Walked result: `MONGO_URI=mongodb://127.0.0.1:27018` and `MONGO_DB=dogtag_vet`, with the groomer on
:43618 showing `dogtag_groomer` and the prover on :41875 showing both empty - it is deliberately kept on
the in-memory store, because it holds no shop data.

### 0.3 Boot

```
scripts/demo-up.sh
```

It reads `contracts/.env` for the chain addresses and the governance key, preflights the chain before
starting anything, then builds and starts six backends and five portals.

Walked preflight output:

```
chainId               135  ok
factory               0xED20269E…F140 -> registry 0xAEE54035…6c21  ok
admin signer          0x8E27E117…F4A2 holds WHITELIST_ADMIN  ok
government            LIVE chain, NO signer -> /issue can only dry_run (no on-chain anchor).
```

That last line is not an error; §4.2 explains it and what to do about it.

| Portal | | Backend | |
|---|---|---|---|
| admin | http://localhost:39741 | admin-api | :39742 |
| vet | http://localhost:41873 | vet-api | :41874 |
| groomer | http://localhost:43617 | groomer-api | :43618 |
| government | http://localhost:44831 | government-api | :44832 |
| owner wallet | http://localhost:45931 | *(no backend, by design)* | |
| | | prover | :41875 |
| | | indexer | :46001 |

**Stopping it: `scripts/demo-down.sh`, which kills the PIDs the script recorded.**
Never `pkill -f target/release/vet-api` or anything of that shape.
This monorepo is checked out many times over - the main checkout plus task worktrees - and **every
checkout builds to the same relative path**, so a pattern kill reaches whichever instance it happens to
hit, including one somebody else is using.

### 0.4 Check it is actually up

```
for p in 39742 41874 43618 44832 41875 46001; do printf "%-6s " $p; curl -s http://127.0.0.1:$p/health; echo; done
```

Walked result: `{"status":"ok"}` from the first five, and from the indexer:

```
{"ok":true,"chainId":null,"backend":"simulated","simulated":true}
```

**`simulated: true` is correct for a `demo-up.sh` stack, and is worth understanding**, because it
changes what the oversight surfaces show you.
`demo-up.sh` starts the indexer with `INDEXER_DEMO_MODE=1`: it serves a scripted event history with
placeholder transaction hashes rather than scanning the chain.
Those rows are correctly labelled "not chain-addressable" and are not offered as explorer links.
A live indexer reports `simulated: false`, and if its scope registry is empty it answers every oversight
query `401` by design - so "the oversight page shows 401" and "the oversight page shows scripted rows"
are two different, both-correct stacks.
Check `/health` before reporting either as a bug.

### 0.5 Unlock custody on the vet and the groomer

**Custody re-locks on every restart of those two backends.**
The sealed key survives; the decrypted seed does not.
Nothing signs or issues until you unlock.

Sign in at the vet portal - the operator password is prefilled in demo mode, so just click **Sign in** -
then click **Unlock** in the banner across the top.
Both fields in the dialog are prefilled; click **Unlock and continue**.

Walked: the dialog is titled *"Custody is locked"* and reads *"This action needs the backend signer.
Enter the passphrase to unlock and continue - nothing you have entered is lost."*
That last clause is literal: the prompt appears in place over whatever page you were on, so a half-filled
form survives.
Repeat for the groomer portal.

---

## 1. Admin

### 1.1 Sign in

http://localhost:39741 - the admin password is prefilled in demo mode. Click **Sign in**.

You land on the **Dashboard**.
The block worth reading is **AUTHORITY**: it names, live from the chain, who holds the factory
ownership, `WHITELIST_ADMIN` and `DEFAULT_ADMIN`, and whether the hosted key is that holder.

Walked result: all three read `0x8e27…f4a2` and are marked **hosted**.
That matters for everything below.
When the hosted key holds an authority, a privileged action is broadcast and mines; when it does not,
the same action comes back as a **proposal** carrying unsigned calldata for someone else to execute.
Nothing is broken in that case, but nothing reaches the chain either.

### 1.2 Register a provider - **Providers** in the left nav

This is the admin half of the generation-2 provider journey, and everything in §2 depends on it.

Read the banner at the top of the page first. Walked result:

> The hosted admin key holds this registry - registrar actions execute directly.

If it says anything else, your `ADMIN_PRIVATE_KEY` is not the governance signer, and every action below
will come back proposed rather than mined.

**It is three steps, not two, and the third is the one people miss.**
Registration writes standing `PENDING`, and the registry admits only `ACTIVE`.
A provider that is registered and approved but never activated is inert - and the page says so, on the
row itself.

**Step 1 - Register.**
Click **Register provider**, then **Fill demo data** in the dialog.
That mints a fresh random provider id, fills the controller, and writes an obviously-fake identity
statement (*"Demo registration - no KYC was performed."*).
Click **Review** - which computes the keccak256 digest of the statement and enables the button - then
**Register**.

Walked result: the page shows **Mined: registration**, the provider id, the transaction hash as an
explorer link, and the identity digest.
On this walk that was provider `0xb876895cf68e76e4c65e181e028143bf199fad7d`, transaction
`0x99b845a608e31afb64d8d0ddfd3fe802775ecb68c1ad763cdd06355aac4a1af5`, block **326156**.

Three things in that dialog are worth reading rather than clicking past:

- **The provider id is arbitrary and permanent.**
  It is generated at random and deliberately means nothing - not the name, not the domain, not a key.
  Copy it: the provider needs it for §2, and it can never be reassigned.
- **The identity statement text is never sent to the backend and is stored nowhere.**
  Only its digest reaches the chain.
  It stays readable under **Registrar actions this session** on that page, and only until you navigate
  away.
- **Registration refuses placeholder data.**
  A zero identity digest, schema or hash algorithm is rejected by the contract, so a real statement is a
  precondition rather than a formality.

**Step 2 - Activate.**
The new row reads **pending**, with the note *"Registered but INERT - every self-service action refuses
until this is Active."*
Click **Activate**.

Walked result: **Mined: standing → active**, and the note becomes *"Cleared to act."*
Transaction `0x4f76331377887b41d7480d85231bc4aa2a25c0d02ff4170bf27c1007b023bbc0`, block **326159**.

**Step 3 - Approve a record type.**
The row carries one button per record type - **DOG_PROFILE**, **VACCINATION**, **GROOMING**,
**BOARDING**.
Click **VACCINATION**.

Walked result: **Mined: approved VACCINATION**, and the **APPROVED TO CREATE** cell changes from
*"Approved for nothing yet."* to `VACCINATION`.
Transaction `0xb6f155a9d25d8b13e2ca64a255c316fcea89b82cdb18f41d19e6403377e66ec8`, block **326162**.
A **Withdraw VACCINATION** button appears once the grant is held.
The grant is per record type: approving VACCINATION does not approve BOARDING.

**Confirm it from the chain rather than from the page.**
This is the exact read the provider's Deploy button is gated on, and it **must be asked as the
factory** - `msg.sender` is part of the answer:

```
PR=0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9      # ProviderRegistry
F2=0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6      # DogTagIssuerFactoryV2
cast call $PR "canCreateService(bytes20,bytes32,address)(bool)" \
  <providerId> $(cast keccak "VACCINATION") <controllerAddr> --from $F2 --rpc-url https://devrpc.roax.net
```

Walked results, all three: `true` for VACCINATION; `false` for BOARDING, which was never approved; and
`false` for VACCINATION when `--from` is omitted.
**That last one is not a failure of your provider.**
With no `from`, `msg.sender` is the zero address, no factory generation matches, and the call answers
`false` for every provider on earth.
The portal passes the factory account on exactly that one read.

> **Data note.** These three transactions are real and permanent on the testnet.
> A provider registered earlier the same day (`0x2ee0cd95…4ecc`) also appears in the list; it was
> created with `cast`, not through this page.

### 1.3 The rest of the admin portal

Opened but not re-walked for this guide: **Activity**, **Issuers / Factory**, **Onboard issuer**,
**Business registry**, **Issuer applications**, **Whitelist**, **Governance**.
Several of them now carry a **Fill demo data** button.
**Verification bench** has a section of its own - §6.

**Activity** reads the oversight indexer, so on a `demo-up.sh` stack it renders the scripted rows
described in §0.4.
The **Dashboard** separately reports *"The oversight indexer is not connected in this environment"* when
`INDEXER_API_BASE` is unset on the central backend, which is a different condition from the indexer being
simulated. Both were seen on this walk.

---

## 2. The provider sets themselves up

**Vet portal → Provider self-service**, or the groomer portal's page of the same name (`/provider` on
either).
There is **no backend on this path at all**: every read and every write is made from the provider's own
wallet, straight to the chain.

### 2.1 First, the refusal you will get on a stock stack

Open the page before configuring anything. Walked result - the whole page is replaced by:

> **Provider self-service is not configured**
> This page reads the generation-2 registry set, and the addresses are not set on this deployment.
> **Nothing about your provider record has been checked.**
> Set: `VITE_PROVIDER_REGISTRY_ADDR`, `VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR`,
> `VITE_SERVICE_DOMAIN_RESOLVER_ADDR`, `VITE_PROVIDER_DIRECTORY_ADDR`.
> There is deliberately no built-in default. The generation-2 contracts are deployed but no client reads
> them yet, so a baked address here would repoint this deployment by accident.

Two things to take from that.
**"Nothing has been checked" is stated rather than implied** - the page does not render an empty provider
record and let you assume it looked.
And **`demo-up.sh` does not set those four variables on purpose**: pointing the shop portals at the
generation-2 set is a deliberate step, not something a demo script should do behind your back.

**To walk the rest of this section, restart the two shop portals with the four addresses set.**
This changes nothing about issuance or verification, which stay on the generation-1 contracts - these
four variables feed only this page:

```
kill $(lsof -nP -iTCP:41873 -sTCP:LISTEN -t)      # the vet portal's vite dev server, by PID
kill $(lsof -nP -iTCP:43617 -sTCP:LISTEN -t)      # the groomer's

led(){ python3 -c "import json;print(json.load(open('contracts/deployments/roax.json'))['$1'])"; }
export VITE_DEMO_MODE=1 \
  VITE_PROVIDER_REGISTRY_ADDR=$(led ProviderRegistry) \
  VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR=$(led DogTagIssuerFactoryV2) \
  VITE_SERVICE_DOMAIN_RESOLVER_ADDR=$(led ServiceDomainResolver) \
  VITE_PROVIDER_DIRECTORY_ADDR=$(led ProviderDirectory) \
  VITE_CONTENT_MIRROR_BASE=http://<your-lan-ip>:46001 \
  VITE_CONTENT_MIRROR_TOKEN=dogtag-indexer-mirror-ingest-demo-token
(cd stacks/vet/web     && ./node_modules/.bin/vite --port 41873 --strictPort &)
(cd stacks/groomer/web && ./node_modules/.bin/vite --port 43617 --strictPort &)
```

Walked result after restarting: the page renders its numbered flows.

### 2.2 The vet and the groomer see different pages, and that is deliberate

Walked, on both portals:

- **Vet**: all four flows - **1. Deploy your own contract**, **2. Choose which contract is current**,
  **3. Your domain**, **4. Your listing**.
- **Groomer**: **only the listing**, and it is not even numbered there - the page's headings are just
  *Your provider record* and *Your listing*.

A groomer verifies and does not issue, so it has no issuing contract at all; flows 1 to 3 are keyed by
one and are not merely hidden but inapplicable.
Flow 4 is keyed by the provider record, so a groomer appears in the directory exactly as a vet does.

### 2.3 Connect the wallet - a real gate, not a formality

Every control on this page, **including the read-only Check buttons**, is disabled until a wallet is
connected.
Filling in a valid, registered, active, approved provider id changes nothing on its own.

That is correct rather than an oversight: with no backend on this path, every action *and every preflight
that reports whether an action would succeed* is made from the provider's own key.
But it does mean this section needs a real browser wallet holding the **controller** address the admin
registered in §1.2, and that account needs testnet gas.

Click **Connect wallet** and pick your wallet.
Walked result once connected: the **Connect wallet** button disappears, and entering the provider id
enables both **Check** buttons.

### 2.4 Flow 1 - deploy your own contract. **Walked; it works.**

Enter your provider id. Leave **Contract number** at `0`. Click **Check what this would deploy**.

Walked result: verdict **Ready**, the predicted contract address shown before anything is committed, and
two checks:

| Check | Result |
|---|---|
| Is your provider record cleared to act? | **PASSED** - *"Your provider record is active."* |
| May this key deploy a contract for your provider and record type? | **PASSED** - *"Approved: your key may deploy this record type for your provider."* |

with the note: *"Ready to deploy. You will own the contract. DogTag then attaches it to your provider
record before you can select it."*

Click **Deploy**.

Walked result: the transaction goes to the generation-2 factory and mines.
On this walk the contract deployed at exactly the address the preview named,
`0x0505Ac77cb3244936d50665A3636090f05Ef0CC1`, in transaction
`0xf67cc7d1cb3c6d599e7f1d13b4c027724bb218f7f31b59fe148e937943a27f73`, block **326280**.
Confirmed independently from the chain: `owner()` is the controller key, `recordType()` equals
`keccak("VACCINATION")`, and the factory's own `isClone` answers `true`.

The plan card then relabels itself:

> **Superseded: these answers were read before your transaction.** A transaction has already been sent
> against this, so the chain may have moved since these answers were read. They are kept below so you
> can see what you checked. Check again before sending another.

The verdict badge is struck through with an amber *"Superseded"* beside it.
The answers stay on screen rather than vanishing, which is the point: what you checked before sending is
exactly what you want to see after sending.

> **Data note.** This is a real contract on the testnet, owned by the controller key.

### 2.5 Flow 2 - choose which contract is current. **Walked; it stops here.**

Paste the address you just deployed and click **Check this contract**.

Walked result: verdict **Not allowed**, with three checks:

| Check | Result |
|---|---|
| Did the DogTag issuer factory deploy this contract? | **PASSED** - *"The factory's own record confirms it deployed this address."* |
| Who owns this contract? | **PASSED** - *"The contract reports your key as its owner."* |
| Has DogTag attached this contract to your provider record? | **FAILED** - *"Not yet attached. A contract you have deployed is not listed until DogTag attaches it."* |

and the remedy, in the product's own words:

> You own this contract. It becomes part of your listing once DogTag attaches it to your provider
> record - that step is DogTag's, not yours. Send this address to DogTag to attach it to your provider
> record. You can select it here once that is done.

**This is where the provider journey genuinely stops, and the blocker is on the admin side.**
Attaching a contract to a provider record is a registrar action, and **it has no admin surface**: the
registrar page built in §1.2 covers register, activate and approve, and nothing else.
Searched across the portal, backend and shared-client sources, the attach call appears only in
explanatory comments, never as a call - so there is no button to press and no route to post to.
Until that surface exists, a provider can deploy a contract and can go no further.

### 2.6 Flow 3 - your domain. **Walked; refused, for a different reason.**

Type a domain and click **Check the domain record**.

Walked result: verdict **Not allowed**, and the state is reported as *"No domain has been published for
this contract, and none has been declined either. Nobody has said."* - publishing no domain and never
having said are kept apart.
Three checks, all failing:

| Check | Result |
|---|---|
| Is the domain register live for this contract? | **FAILED** - *"the domain register is not approved; this contract has not selected the domain register"* |
| Does this contract's standing still allow changes? | **FAILED** |
| May your key publish a domain for this contract? | **FAILED** |

The first is the root cause: a typed resolver answers nothing until DogTag approves it **and** the
provider selects it.
Neither has happened, and neither has an admin surface.

### 2.7 Flow 4 - your listing. **Walked; four of five checks pass.**

Click **Fill demo data**, then **Check what this would publish**.

Walked result: verdict **Not allowed**, with:

| Check | Result |
|---|---|
| Is the location usable, or deliberately left out? | **PASSED** - *"Location 1.29027, 103.851959 will be published."* |
| Is the provider directory live for your provider record? | **FAILED** - *"The directory is either not approved by DogTag or not selected by your provider record."* |
| May your key publish into your provider record? | **PASSED** |
| What is already published for your provider record? | **PASSED** - *"You have never published contact details…"* |
| Is there anything to publish? | **PASSED** |

Again a single registrar gate. Confirm it yourself - it is one call, and it reads `false`:

```
cast call 0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0 "resolverApproved()(bool)" --rpc-url https://devrpc.roax.net
```

Two properties of this flow are worth knowing for when it does open:

- **A blank location publishes no pin at all.**
  It does not publish `0,0`, which is a real coordinate off the coast of Ghana.
  The page states the consequence plainly: *"You will not appear in the nearby list, because that list is
  built from published locations - and an invented coordinate would put you somewhere you are not."*
- **Re-publishing rewrites rather than appends**, which is what stops one provider appearing in two
  places at once.

### 2.8 Where §2 leaves you

| Flow | Outcome |
|---|---|
| 1. Deploy your own contract | **Works.** Walked to a mined transaction and an independently verified contract. |
| 2. Choose which contract is current | **Blocked** - needs DogTag to attach it; no admin surface exists. |
| 3. Your domain | **Blocked** - the domain register is neither approved nor selected. |
| 4. Your listing | **Blocked** - the provider directory is neither approved nor selected. |

All three blockers are the same shape: a registrar step with no admin surface yet.
None of them is a misconfiguration you can fix from the provider's side, and the product says so on each
screen rather than failing obscurely.

---

## 3. The vet issues a credential

Vet portal, http://localhost:41873. Custody must be unlocked (§0.5).

### 3.1 Register the pet - **Register pet** in the nav

A record attaches to a pet that already has a dog tag, so this comes first, once per pet.

Click **Fill demo data**.
Walked: it fills owner identity (`GB` / `P1234567` / `Alex Doe`) and the pet profile (`Rex`, dog,
Labrador Retriever, male, neutered, born 2021-04-15) plus a microchip.
Click **Start issuance**.

Walked result:

> **Owner scans to receive their dog tag**
> Dog tag **1** allocated - pending device bind.
> `http://192.168.1.123:41874/p/aba919ad45248401c39d62e573c2fd15`
> Owner scans this in the DogTag app to receive their dog tag. Waiting for the device to bind…
> Status: **pending**

**This is as far as a desktop can take it, and the reason is architectural rather than a gap.**
The owner's device folds the profile tree and posts the resulting root; the vet never holds the owner's
secret.
Completing the bind needs the phone app (§8).

You can still see exactly what the phone would receive, which is a useful check that the QR is live:

```
curl -s http://127.0.0.1:41874/p/aba919ad45248401c39d62e573c2fd15
```

Walked result: JSON carrying the session id, `dogTagId: "1"`, `status: "pending"`, the full pet block,
the owner identity block, and the vet-generated `identityLeaves` - each with its own salt - that the
device commits into the root.

### 3.2 Issue a record - **Issue a record** in the nav

**A record can be issued while the dog tag is still pending its bind** - walked, and worth knowing,
because the two are anchored independently.

Click **Fill demo data** (a rabies certificate, recordType `VACCINATION`), set **Dog tag id** to `1`, and
click **Sign & Issue**.

Walked result:

> **Credential issued** - Record `c5b8af75-3341-4146-adc5-d248d054227a`
> **On-chain status: Verified on-chain**
> Merkle root: `0x12679d01c81d71648484697ade0fa531f6ceb413cb26d204556e819c50363b77`
> Tx: `0xc94027287d9107b15fdd0e04402c0e3eab5848b243ebcef50bd299c2be79d740`

That root is genuinely anchored on ROAX.
It is the credential used in §4.1, §5 and §6.

> **Data note.** Dog tag 1 and this record are new data created by this walk.

### 3.3 Getting the credential out

The **Records** page lists what this shop has issued, with an explorer link for each.
To take a credential elsewhere - into the owner wallet, or the bench - fetch it from the backend with
your operator session:

```
curl -s http://127.0.0.1:41874/records -H "Authorization: Bearer <vet.opToken from localStorage>"
```

Walked: returns the record with its `wrappedDoc`, whose `signature.merkleRoot` matches the root above.

---

## 4. Government

Government portal, http://localhost:44831.

### 4.1 Verify somebody else's credential - **walked, and it works**

This is the cross-role check: the vet issued in §3.2, the government verifies here, and the two share no
database.

Paste the wrapped document into **Verify a document (paste JSON)** and click **Verify**.

Walked result on the vet's real credential:

> **✓ VALID**
> integrity: yes · on-chain: yes · issuer authorised at issuance: yes · factory-deployed: yes
> Issuer: **DogTag Vaccination** *(from the issuing contract)*
> This issuer has published no domain on-chain
> chain read at block 326343
> *The document names a different issuer: "Seaport Vet"*

Two things there are worth pausing on.

**"Issuer authorised at issuance" is a historical question, not a current one.**
The page says so: *"whether the signer that anchored this root held the capability AT THAT BLOCK, from
the governing registry's own grant log. Delisting is forward-only, so a since-rotated issuer does not
invalidate what it issued."*
A vet that rotates its key does not retroactively void every certificate it ever issued.

**The issuer-name mismatch is reported, and does not change the verdict.**
The contract's own `name()` is *DogTag Vaccination*; the document claims *Seaport Vet*.
The credential is genuine - the name sits outside the Merkle root, so it is not covered by integrity, and
the page reports the discrepancy beside the verdict rather than folding it in.
§6.2 shows the same property from the attacker's side.

All reads here are gasless, and no owner and no phone are involved: this checks a document, not a
consent.

### 4.2 Issue - **not walked; disabled on a stock stack**

Click **Fill demo data** on the **Issue** page and the button below reads:

> **Issue + anchor** - Disabled - no issuer clone is configured for TRAVEL_CLEARANCE on this deployment.

That matches the preflight warning in §0.3 and `/health`, which reports `canSign: false` and
`issuers: { TRAVEL_CLEARANCE: null, EU_HEALTH_CERT: null }`.
Anchoring a government credential needs three things a plain `demo-up.sh` does not create: a funded
government signer, that signer whitelisted for the record type, and a contract to anchor into.

The remedy is `scripts/demo-provision-government.sh`, which creates whatever is missing and is safe to
re-run.
**It requires `contracts/.env` to exist in the repo root** and refuses with
`ERROR: …/contracts/.env not found` otherwise - which is why this step was not walked here, since this
walk ran from a checkout that deliberately has no such file.

---

## 5. The owner receives the credential

Owner wallet, http://localhost:45931. It has **no backend** - everything is local to the browser.

Go to **Receive**, paste the wrapped document from §3.3, and click **Add to wallet**.

Walked result: the wallet routes to the credential and shows

> 💉 **VACCINATION** · issued by Seaport Vet
> **✓ Integrity intact**
> **✓ Anchored on-chain**

with every field that was hashed into the root - microchip code and standard, vaccine product code, name
and manufacturer, batch, series, vaccination date, valid-from, valid-until, next-due, authorising vet and
dog tag id - under the heading *"Exactly the values hashed into this credential's on-chain Merkle root."*

**The integrity check is offline and the anchor check is a direct chain read.**
No server was asked whether this credential is good.

The **Fill demo data (vaccination)** and **Fill demo data (travel)** buttons on that page are the
zero-setup path if you have not issued anything yet.
They are the one place in the product where such a button is not gated on demo mode, because the owner
wallet has no demo-mode flag at all.

---

## 6. Verification, and eleven attempts to defeat it

Admin portal → **Verification bench**.
This is the surface for "what exactly is checked, and what is not".

### 6.1 A genuine credential

Paste the §3.2 credential into **Wrapped document JSON** and click **Run the checks**.

Walked result: **Verifier verdict: valid**, with *"Every on-chain read was pinned to block 326311"*, and
nine rows:

| Check | |
|---|---|
| Is the document's content intact - does it still hash to the root it claims? | Pass |
| Was this issued by a contract that genuinely descends from the DogTag factory? | Pass |
| Does the document name the same contract the factory says issued it? | Pass |
| Was the signer that issued this authorised for this record type when it anchored the root? | Pass |
| Is this root actually anchored on-chain by its issuing contract? | Pass |
| Has the issuer revoked this credential? | Pass |
| *not in the verdict* - expiry | Pass |
| *not in the verdict* - registry governs issuer | Pass |
| *not in the verdict* - whitelisted at issuance | Pass |

**The rows marked *not in the verdict* are the point of the page.**
The verifier's verdict covers integrity, on-chain status and the issuer pillar, and nothing else.
Expiry is reported beside it rather than folded in, because the chain records anchoring and revocation
and has no concept of a validity window.
So an expired-but-anchored credential legitimately shows a valid verdict above a red expiry row, and the
page marks which is which rather than leaving you to guess.

### 6.2 Try to break it - the mutation buttons

Each button tells one specific lie with the record you loaded and re-runs everything.

Walked: **"Point it at a different issuer contract"** - the credential claims it was issued by a contract
the attacker controls, which vouches for it.

Result: **Verifier verdict: not valid**, pinned to block 326316. The rows that changed:

- *Does the document name the same contract the factory says issued it?* → **Fail**
- *Was the signer that issued this authorised…?* → **Fail**

while integrity, factory-descent, anchoring and revocation all still **Pass**.
The forgery is caught precisely, not by everything going red at once.

**Read each button's "what will NOT catch this" list as carefully as its result.**
The first mutation, **"Relabel the issuer's name"**, declares *"Designed to be caught by: nothing on this
verification path"* - the same property §4.1 showed from the government's side.
The name is not covered by the Merkle root, so relabelling it is not detected here; the government verify
page reports the discrepancy separately.

### 6.3 The attack catalogue - **Run the whole catalogue**

Eleven complete fraudulent records, each with its own scripted chain, each declaring in advance which
check must refuse it.
These need no loaded record and make no network call: they exercise chain states a live chain cannot be
asked to produce - a signer delisted after it issued, a contract the factory never deployed vouching for
a root, a registry that does not govern the clone.

Walked result: **all eleven matched their declared expectations**, with no divergence.
Verdicts across the eleven: three `valid`, seven `not valid`, one `no verdict`.

**One of the eleven is an honest control - a genuine credential that must verify.**
The page states why it is there: *"without it a catalogue of nothing but frauds would look perfect
against a verifier that refused everything."*
That control passing is what makes the other ten results mean anything.

One scenario deliberately produces **no verdict** rather than a refusal: pointed at an endpoint on the
wrong chain, every on-chain row reports *could not run* and the verdict is withheld.
"The factory has no record of this root" would be an accusation nobody was in a position to make.

---

## 7. Reference: the four generation-2 reads

Eight generation-2 contracts are deployed on ROAX.
**Issuance and verification still run entirely on the generation-1 contracts**, deliberately - so a
generation-1 address in a portal is correct, not stale.
What changed recently is that the admin registrar (§1.2) is now a real reader of one of them, the
provider registry.

Read the live state rather than trusting numbers written here.
All four were run on this walk:

```
cast call 0xf374f4cA5ebBBAFf0dFcE48D8Cda2e47F9D5da01 "generationCount()(uint256)" --rpc-url https://devrpc.roax.net
cast call 0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0 "resolverApproved()(bool)"   --rpc-url https://devrpc.roax.net
cast call 0xD3B121FEaCde93b95288912EAdbB10824550FdBF "boundCloneCount()(uint256)" --rpc-url https://devrpc.roax.net
cast call 0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9 "providerCount()(uint256)"   --rpc-url https://devrpc.roax.net
```

- **`generationCount`** - how many factory generations the provenance router resolves through.
  Two or more means generation 2 has been appended. It read **2** on this walk.
- **`resolverApproved`** - false means the registrar has not approved the typed directory resolver, so
  every directory store stays empty and §2.7 cannot proceed. It read **false** on this walk.
- **`boundCloneCount`** - zero means no issuing contract has ever bound a domain on the superseded domain
  registry. It read **zero**.
- **`providerCount`** - how many providers exist. It read **1** before §1.2 and **2** after.

To see which providers exist, read the registry's own logs.
**Use raw JSON-RPC, not `cast logs`** - the latter renders extra rows for the same query, so an empty
result from it is weak evidence for a strong claim:

```
curl -s -X POST https://devrpc.roax.net -H 'content-type: application/json' --data \
  '{"jsonrpc":"2.0","id":1,"method":"eth_getLogs","params":[{"address":"0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9","fromBlock":"0x0","toBlock":"latest"}]}'
```

The same query against the generation-2 factory (`0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6`) tells you
whether any provider contract has ever been deployed through it.
Before this walk it had emitted nothing; §2.4 changed that.

---

## 8. What this guide does not cover

**Anything needing the phone app.**
The DogTag holder app carries a compiled Rust core and bundled proving artifacts and is not built by
`demo-up.sh`; `docs/MOBILE_BUILD.md` covers it. That means:

- **Completing the dog-tag bind (§3.1).**
  The QR is minted and resolves correctly; the device half was not performed.
- **The owner-hidden consent proof.**
  The owner proves consent on-device in zero knowledge; there is no desktop equivalent.
  Android additionally needs an arm64 device or emulator, because the prover ships arm64-only.
- **Nearby provider search, the offline provider cache, and the directions handoff.**
  All are phone surfaces.
  The rendered result rows additionally cannot be verified on a dev machine at all, because the directory
  host is a fixed production constant with no debug override.

**Government issuance (§4.2)** - disabled on a stock stack; needs `scripts/demo-provision-government.sh`,
which needs `contracts/.env` in the repo root.

**The three blocked provider flows (§2.5, §2.6, §2.7)** - each needs a registrar action that has no admin
surface.

**The groomer's shop CRM** - clients, pets, appointments, the calendar and its `.ics` feed, and the
microchip cross-check.
The groomer portal was opened and its provider page walked (§2.2); its CRM was not.

**A note on the Playwright suites.**
Do not run them unfiltered.
Several are unmocked live-portal drivers that create real records and anchor them on chain.

---

## 9. Evidence: what was walked, when, and against what

Walked on **2026-08-02**, against commit **`657bcde`**, on a live stack on ROAX chainId 135, booted from
`scripts/demo-up.sh` with `MONGO_URI` set.
Every result quoted above as walked was observed in a browser or over `curl`/`cast` on that date.

**Walked end to end:**

- **§0** - the build; the `demo-up.sh` boot including its chain preflight; `/health` on all six backends;
  per-service Mongo databases confirmed both by listing the databases and by reading the live process
  environment; the `mongo`-feature refusal reproduced by running a non-`mongo` binary with `MONGO_URI`
  set; custody unlock through the in-place prompt.
- **§1** - admin sign-in; the Dashboard authority block; and the full three-step registrar flow, which
  mined three transactions at blocks **326156**, **326159** and **326162**.
  `canCreateService` was re-confirmed from the chain afterwards in all three of its forms.
- **§2** - the unconfigured refusal; the four-flow page after configuring; the vet/groomer difference;
  the wallet gate; flow 1 deployed a real contract (block **326280**) whose owner, record type and
  factory provenance were verified independently on chain; flows 2, 3 and 4 each run to their verdict and
  their failing check.
- **§3** - register pet through to the minted QR, with the `/p/` endpoint resolved to confirm what the
  phone would receive; issue a record through to **Verified on-chain**.
- **§4.1** - government verification of the vet's credential, block-pinned, including the issuer-name
  discrepancy.
- **§5** - the owner wallet receiving that same credential and reporting integrity and anchoring.
- **§6** - the bench on the genuine credential (nine rows); one mutation applied and caught; the eleven
  scenario catalogue run in full with no divergence.
- **§7** - all four `cast` reads.

**Not walked, each with its reason stated in place:** everything in §8.

**One caveat about §2, stated because it affects how much weight to put on it.**
The wallet used for the provider flows was a scripted EIP-6963 provider signing with the well-known
public anvil test key, not a consumer wallet extension.
The product code exercised is identical - the same connect path, the same preflights, the same
transactions - but the wallet's own UI was not exercised, so a wallet-specific problem would not have
shown up.

**There is no CI for any of these paths.**
The two mobile workflows are dispatch-only and no workflow runs the Rust test suite, so a local walk is
the only evidence any of this works.
If you change a portal, re-walk the section rather than assuming.
