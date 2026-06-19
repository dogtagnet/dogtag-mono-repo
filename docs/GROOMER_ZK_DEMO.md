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

This builds on `docs/DEMO_CLICKS.md` (admin/vet setup, issue, import) — only the **new** groomer ZK
**export**, the **mobile prover build**, and the **reset** are detailed here.

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
# wipe the phone app's state so it re-genesises a fresh owner wallet:
#   Android: adb shell pm clear io.liberalize.dogtag   (or reinstall the APK)
#   iOS:     delete + reinstall the app
```

The demo backends use an **in-memory store** — a restart is a clean slate (custody re-genesis).
On-chain state (the live contracts) persists; the demo just creates fresh signers each run.

## 2. Boot

```bash
scripts/demo-up.sh                   # sets VITE_DEMO_MODE=1; wires the new VR + CKR + CONSENT_KEY_REGISTRY_ADDR
```
Portals: admin `:39741`, vet `:41873`, groomer `:43617`. Backends: admin `:39742`, vet `:41874`,
groomer `:43618`. For a phone on another network, boot with a tunnel:
`VET_PUBLIC_URL=https://<sub>.trycloudflare.com scripts/demo-up.sh` (see `docs/DEMO.md` §6).

## 3. Create admin → create vet → approve vet

Follow `docs/DEMO_CLICKS.md §A–B` (passwords are demo-prefilled under `VITE_DEMO_MODE=1`):
1. **Admin** portal → sign in → **Onboard issuer** wizard: register business → submit issuer
   application → **Approve** (on-chain `whitelistFor` per address × recordType).
2. **Vet** portal → Setup wizard: genesis (24-word seed) → unlock → note the vet **signer address**.
3. Fund + whitelist that signer (issuance **and** the groomer relayer's VERIFY purposes):
   ```bash
   scripts/demo-bootstrap.sh <vetSignerAddress>        # funds 0.5 PLASMA + whitelists VACCINATION/DOG_PROFILE/SERVICE_ATTESTATION
   # groomer relayer (do its Setup genesis first to get its signer), then:
   scripts/demo-bootstrap.sh <groomerSignerAddress>   # also whitelists VERIFY:grooming_intake/boarding_intake/daycare_access
   ```
   The owner's phone wallet needs **no** funding — it stays gasless.

## 4. Issue the dog tag + the vaccination

- **Issue dog tag**: the admin/central mints the `DOG_PROFILE` SBT to the **owner's phone wallet
  address** (the `subject`). This must be the same address the phone signs with, so
  `ownerOf(dogTagId) == subject` holds at verify time.
- **Issue vaccination**: vet portal → **Issue** → "Fill demo data" (rabies cert, `VACCINATION`) →
  Sign & Issue (anchors the Merkle root on the clone) → **Create QR** (a short one-time `/r/<token>`).

## 5. Import the vaccination on the phone

Scan the `/r/<token>` QR → the app fetches the wrapped doc, verifies integrity + on-chain
`isValid(root)`, and stores it (with its raw Merkle leaves — the witness source). Tap the credential
to view the decoded fields.

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
     **generates the Groth16 proof locally** (~1–2 s), and signs the one-time EIP-712 consent-key
     **bind** authorization (off-chain).
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
- Future consent-key registry swaps don't need a VR redeploy: `consentKeys` is now timelock-settable
  (`proposeConsentKeys`/`executeConsentKeys`, 2-day).
