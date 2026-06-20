# Groomer gasless on-device ZK verification — full reset + click-through

End-to-end runbook for the headline flow: a groomer verifies a pet's vaccination with a
**zero-knowledge proof generated on the owner's phone**, where **the owner pays no gas** and the
groomer (relayer) submits every transaction — Tornado-style.

- **True ZK**: the phone generates the Groth16 proof locally and sends only `{proof, publicSignals,
  consent}`. The groomer **never sees the raw vaccination record**.
- **Gasless owner**: the owner only signs off-chain (EdDSA consent, the proof, an EIP-712 bind sig).
  The groomer relayer broadcasts the one-time consent-key bind **and** the verification record.
- **Same ROAX testnet** (chainId 135). Live (new) contracts: VerificationRegistry
  `0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1`, ConsentKeyRegistry
  `0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671` (gasless `bindConsentKeyFor`). Full set in
  `contracts/deployments/roax.json`.

Verified on-chain (gasless, owner balance stayed 0): bind
`0xde83454da66844f10b6b33304cf2821b8ca39c5100c91b9bdb5a833ed413990a`, ZK record
`0x8607f9df8d2c39216a79c1a69943cda7e1c0a4770ec250b7239504137c072f32` (`Verified` emitted,
`consumed(nullifier)=true`). The automated `scripts/e2e-zk.sh` reproduces the full HTTP-relay path.

This builds on `docs/DEMO_CLICKS.md` (admin/vet setup, issue, import). The runbook below is the
**authoritative LOCAL manual-test runbook** for the full proper onboarding flow:

1. **Vets issue dog tags** — the phone creates a self-custodial wallet, then the **vet** "Issue dog
   tag" wizard (operator enters the `ownerIdentity` + pet fields, demo-prefilled) starts a session and
   shows a QR `/p/<token>`; the device scans it, sends its signed wallet address, and the **vet mints
   the `DogTagSBT` on-chain** and returns the credential. There is **no central `/v1/register`** and
   **no admin "Registered devices" / "Mint dog-tag" page** — every host the device talks to comes from
   a scanned QR.
2. **Admin portal = approve + whitelist vet/groomer wallet addresses only** — it does NOT register
   devices or mint dog tags. Issuers AND verifiers onboard via **apply→approve**: the vet is
   whitelisted for issuance record-types, the groomer for `VERIFY:<purpose>` (no `demo-bootstrap`
   casting the VERIFY whitelist).
3. The dog-tag mint requires the vet signer to hold **`DogTagSBT.ISSUER_ROLE`**, granted once by the
   protocol admin.

The only remaining `cast`/script step is **funding** the vet/groomer signers with PLASMA + (once) the
vet **ISSUER_ROLE grant** — both done by `scripts/demo-bootstrap.sh`. **All whitelisting is via the
admin portal.**

---

## 0. One-time: build the mobile apps with the on-device prover

The phone needs the Groth16 prover compiled in (`--features prover`, mopro `circom-prover` +
`rust-witness`) and the proving key bundled. The zkey is a 65 MB copy of
`circuits/build/verification_final.zkey` (gitignored in the apps — copy it in at build time).

```bash
# vendor the prover key into both app bundles (gitignored; ~65 MB each)
cp circuits/build/verification_final.zkey apps/android/app/src/main/assets/verification_final.zkey
cp circuits/build/verification_final.zkey apps/ios/DogTag/verification_final.zkey

# build the FFI lib + regenerate UniFFI bindings WITH the prover feature, then the apps
#   (uses the existing cargo-ndk → jniLibs and xcframework pipeline; build.rs transpiles the
#    circuit witness to native C via rust-witness — needs git + cmake + a C toolchain)
#   Android:
(cd apps/android && ./gradlew :app:assembleDebug)        # → app/build/outputs/apk/debug/app-debug.apk (~115 MB)
#   iOS (simulator example):
(cd apps/ios && xcodebuild -project DogTag.xcodeproj -scheme DogTag \
   -destination 'platform=iOS Simulator,name=iPhone 16' -derivedDataPath build/DD build)
```

Install the APK / run the iOS build on the device you'll demo with.

---

## 1. Reset the local environment (start from scratch)

```bash
scripts/demo-down.sh                 # stop backends + portals
rm -rf .demo                         # clear PIDs/logs and the in-memory genesis state
# wipe the phone app's state so it re-creates a fresh owner wallet:
#   Android: adb shell pm clear io.liberalize.dogtag   (or reinstall the APK)
#   iOS:     delete + reinstall the app
```

The demo backends use an **in-memory store** — a restart is a clean slate (custody re-genesis **and**
the vet's issue-session table). On-chain state (the live contracts) persists; the demo creates fresh
signers each run, and the phone re-creates its wallet on the next launch.

## 2. Boot

```bash
scripts/demo-up.sh                   # sets VITE_DEMO_MODE=1; wires the new VR + CKR + CONSENT_KEY_REGISTRY_ADDR
```
Portals: admin `:39741`, vet `:41873`, groomer `:43617`. Backends: admin `:39742`, vet `:41874`,
groomer `:43618`. For a phone on another network, boot with a tunnel:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` (see `docs/DEMO.md` §6).

## 3. Issuer + verifier onboarding via apply → approve (admin portal)

Both the **vet** (issuer) and the **groomer** (verifier) onboard through the same apply→approve flow —
the admin portal **only approves + whitelists wallet addresses**; it does not register devices or mint
dog tags. **No more `demo-bootstrap` casting the VERIFY whitelist.**

1. **Vet** portal → Setup wizard: genesis (24-word seed) → unlock → note the vet **signer address**.
2. **Groomer** portal → Setup wizard: genesis → unlock → note the groomer **signer (relayer) address**.
   The Setup **"Apply for whitelist"** form now has a **verify purposes** field (demo-prefilled
   `grooming_intake/boarding_intake/daycare_access`); the application carries `verifyPurposes`.
3. **Fund** both signers with PLASMA **and grant the vet `DogTagSBT.ISSUER_ROLE`** — the only remaining
   `cast`/script steps (whitelisting is NOT a script anymore):
   ```bash
   # funds 0.5 PLASMA, whitelists the issuance record-types, AND grants ISSUER_ROLE on DogTagSBT
   #   (idempotent: skips the grant if already held) so the vet can mint dog tags in §5:
   scripts/demo-bootstrap.sh <vetSignerAddress>
   # the groomer is a verifier/relayer (no minting) — funding + VERIFY whitelist only:
   scripts/demo-bootstrap.sh <groomerSignerAddress>
   ```
   > **`ISSUER_ROLE` is a trust escalation** — a holder can mint **any** `dogTagId` to **any** address.
   > In production grant it **only to accredited vets**, gated by the admin's accreditation review.
4. **Admin** portal → approve both applications (`approve_application`):
   - **Vet** application → **Approve** → `IssuerRegistry.whitelistFor(recordType, vetSigner)` on-chain per
     issuance record-type (VACCINATION/DOG_PROFILE/…).
   - **Groomer** application → **Approve** → for each `verifyPurpose`, whitelists `VERIFY:<purpose>`
     on-chain: key = `keccak256(abi.encode("VERIFY:", keccak256(label) mod r))` (the purpose label reduced
     mod BN254 `r`), `whitelistFor(verifyKey, groomerRelayer)`. So the groomer is now an authorized
     **verifier** for those purposes — gated separately from issuer roles.

> **Three independent grants — different scope + lifecycle.** Don't conflate them:
> - **`DogTagSBT.ISSUER_ROLE`** — a **one-time protocol grant** to an accredited vet signer. It's a trust
>   escalation: a holder can mint **any** `dogTagId` to **any** address. Granted once (admin "Approve" of
>   an issuer application whose recordTypes include `DOG_PROFILE`, or the idempotent `demo-bootstrap.sh`
>   fallback). Not per-record-type, not per-purpose.
> - **Issuance record-type whitelist** (`IssuerRegistry.whitelistFor(recordType, vetSigner)`) —
>   **per-application, per-record-type** (VACCINATION / DOG_PROFILE / …). Set by the admin "Approve".
> - **`VERIFY:<purpose>` whitelist** (`whitelistFor(verifyKey, groomerRelayer)`) — **per-application,
>   per-purpose** (one entry per `grooming_intake` / `boarding_intake` / `daycare_access`). Gates the
>   groomer relayer's `recordVerificationZK` calls, **separately** from any issuer role. Set by the admin
>   "Approve" of the groomer application.
>
> `demo-bootstrap.sh <signer>` does the off-portal parts in one shot: funds 0.5 PLASMA, whitelists the
> issuance record-types, grants `ISSUER_ROLE` (for a vet), and casts the `VERIFY:<purpose>` whitelist (for
> a groomer relayer). The canonical path for both whitelists + the role is the **admin portal Approve**;
> the script is the idempotent fallback (and the only way to fund gas, which the portal can't do).

The owner's phone wallet still needs **no** funding — it stays gasless (it never sends a transaction;
mint and verify are both gasless for the device; the vet/groomer pay gas).

## 4. The vet issues the dog tag (session + QR → device mints in)

The dog-tag is no longer minted by an admin "Registered devices" page — the **vet** issues it via a
session + QR, mirroring import/export. The device just needs a self-custodial wallet first.

1. **Create the wallet** — phone **Profile → "Create embedded wallet"** → the app generates a
   **24-word seed** and derives the **secp256k1** EVM **walletAddress** (`m/44'/60'/0'/0/0`; stored
   encrypted, §6.4). The phone has **no** "Central API URL" setting — every host it talks to comes from
   a scanned QR.
2. **Vet "Issue dog tag"** — vet portal → **Issue dog tag** → "Fill demo data". The operator enters the
   **`ownerIdentity`** that goes on the `DOG_PROFILE` credential (demo-prefilled) + the pet fields:
   - `ownerIdentity.countryOfIdentification` — e.g. **GB**
   - `ownerIdentity.identification` — gov-ID / passport number, e.g. **P1234567**
   - `ownerIdentity.name` — official name as on the ID, e.g. **Alex Doe**
   - pet fields — name, species, breed, microchip, …
3. **Start** → `POST /profiles/issue/session/start` returns a one-time **token** + a QR
   `<vetHost>/p/<token>` (a 32-hex token, **180s TTL**) and the allocated **`dogTagId`**. The portal
   renders the QR.
4. **Device scans `/p/<token>`** → the app `personal_sign`s (EIP-191) the exact string
   `DogTag wallet registration: <walletAddress lowercased>` and POSTs
   `<vetHost>/profiles/issue/bind { token, walletAddress, signature }` (the signature **proves the
   device owns the address**).
5. **Vet backend mints on-chain**: recovers the signer (asserts `== walletAddress`), builds the
   `DOG_PROFILE` VC with `ownerIdentity` + `ownerAddress` (the device wallet), and calls
   **`DogTagSBT.mint(to=walletAddress, dogTagId, root)`** — which sets `ownerOf[dogTagId] = device`
   **and** `profileRoot[dogTagId] = root`. This needs the vet signer to hold **`ISSUER_ROLE`** (§3.3).
   It responds to the device with `{ wrappedDoc, dogTagId, root, txHash }`.
6. **Device verifies + imports**: checks against the SBT (`profileRoot == root && ownerOf == wallet`) +
   offline integrity, then imports its dog tag. **Gasless for the device** (the vet pays gas). After
   this, `ownerOf(dogTagId) == walletAddress` holds, so the phone can later prove
   `ownerOf(dogTagId) == subject` at verify time.

## 5. Issue the vaccination + import on the phone

- **Issue vaccination**: vet portal → **Issue** → "Fill demo data" (rabies cert, `VACCINATION`). The
  demo-fill populates the cert fields but **leaves `dogTagId` blank** — set it to the **handle from §4**
  (the `dogTagId` the Issue-dog-tag wizard allocated). This must match: on-chain the SBT key is
  `field_of_value(handle)`, and the §6 ZK export checks `ownerOf(field_of_value(dogTagId)) == subject`;
  a mismatch reverts the export with `ERC721NonexistentToken`. (The demo-fill **no longer clobbers** the
  `dogTagId` field — a fixed footgun — but you must still type the matching handle.) → **Sign & Issue**
  (anchors the Merkle root on the clone) → **Create QR** (a short one-time `/r/<token>`).
- **Import on the phone**: scan the `/r/<token>` QR → the app fetches the wrapped doc, verifies
  integrity + on-chain `isValid(root)`, and stores it (with its raw Merkle leaves — the witness source).
  Tap the credential to view the decoded fields.

## 6. Groomer ZK export (the finale)

1. **Groomer** portal → **Export** → purpose **boarding_intake**, mode **ZK (private)** → **Start** →
   renders a one-time **EXPORT** QR — a **token** (`/x/<token>?a=<groomerAddr>`, NOT a JWT) carrying the
   groomer's **relayer** wallet address + host. (The session record holds the purpose/challenge/mode.)
2. **Phone** → Scan → resolves `GET /x/<token>` → the export panel:
   - **Verifies the groomer on-chain first**: `IssuerRegistry.isWhitelistedFor(verifyKey(purpose),
     relayer)` — if the verifier isn't a whitelisted groomer, it **hard-stops** ("not an authorized
     groomer") and never discloses anything. (Prod/remote also **DNS-verifies the groomer**:
     `dogtag-verify=<groomerAddr>` via DoH; skipped for the `.local` demo.)
   - Select the vaccination credential → **Approve & present**: the phone signs the EdDSA consent,
     **generates the Groth16 proof** (~1–2 s), and signs the one-time EIP-712 consent-key **bind**
     authorization (off-chain).
     - **64-bit devices** (iPhone, modern arm64 Android) prove **on-device**.
     - A **32-bit-only Android** (`Build.SUPPORTED_64_BIT_ABIS` empty) can't run the on-device
       circom-prover, so it POSTs `{wrappedDoc, consent, eddsaSig}` to the **prover-service**
       (`POST /prove-verification`) and submits the returned proof to the groomer itself — **the groomer
       still never sees the witness**. `demo-up.sh` boots that service on **:41875** (a `vet-api` built
       `--features prover` with `CIRCUITS_BUILD_DIR` set, so the real ArkProver loads; the feature-OFF
       groomer literally has no `/prove-verification` route). The phone targets
       `AppConfig.DEFAULT_PROVER_API` (override via the `prover_api` pref); tunnel it for an off-LAN phone
       with `cloudflared tunnel --url http://localhost:41875` →
       `PROVER_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh`.
   - POSTs `{proof, pubSignals, consent, bind:{subject, keyHash, ownerSig}}` to the groomer host
     (`/v1/verify/consent`, authorized by the one-time export token, consumed on submit).
3. **Groomer backend (relayer)**: validates `pubSignals ↔ session`, then **pays gas** to (a)
   `bindConsentKeyFor` (first time only — the owner authorized it off-chain) and (b)
   `recordVerificationZK` on the VerificationRegistry. The contract checks `msg.sender == relayer`
   (baked into the proof), the VERIFY whitelist, `keyOf(subject) == keyHash`, `ownerOf(dogTagId) ==
   subject`, the Groth16 proof, and consumes the nullifier.
4. **Phone + portal poll** `GET /verify/session/{id}` → **Verified on-chain**, txHash shown. No
   vaccination data was written on chain and the owner spent **zero gas**.

## Verify it worked

- Portal shows **Verified** + a txHash → open it on `https://explorer.roax.net`; you'll see the
  `Verified(dogTagId, relayer, subject, purpose, nullifier, ts)` event and `msg.sender == relayer`.
- `cast call 0x8bA836eCe9a27c43049aCcC26eB5a1579c1FcFA1 'consumed(bytes32)(bool)' <nullifier>` → `true`.
- `cast balance <ownerPhoneWallet>` → unchanged (the owner never paid gas, never sent a tx).

## Automated proof (no phone needed)

`scripts/e2e-zk.sh` drives the entire backend-relay ZK path against the live new contracts: it boots
an isolated groomer backend, genesises + funds a relayer, generates a **real** Groth16 proof
(`scripts/zk/gen_input.mjs` → `prove-stdin`), sets the preconditions via the deployer key, gaslessly
binds the consent key, POSTs the proof to `/v1/verify/consent`, and asserts the on-chain `Verified`
state + the owner's zero balance. Re-runnable (fresh dogTagId/nonce each run).

## Notes
- ZK vs NORMAL: this runbook is the **ZK** path (no data on chain, groomer never sees the record).
  The NORMAL/ECDSA path (`docs/DEMO.md`) discloses the record to the groomer and writes the
  credentialRoot on chain — use ZK for privacy.
- **The dog-tag `ownerIdentity`/`ownerAddress` leaves do NOT feed the ZK proof.** The export proof
  (§6) is over the **VACCINATION** root, not the `DOG_PROFILE` root. The new `DOG_PROFILE` leaves are
  **additive** committed leaves; the owner is bound on-chain via `ownerOf(dogTagId) == subject` + the
  in-circuit consent-key, and leaf assembly is **variable-arity** (≤24 leaves, depth 5), so leaf count
  never changes the circuit's signals or depth. Verified by running the prover — the circuit is
  unchanged.
- Future consent-key registry swaps don't need a VR redeploy: `consentKeys` is now timelock-settable
  (`proposeConsentKeys`/`executeConsentKeys`, 2-day).
