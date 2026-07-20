# MOBILE_OWNER_SECRET - the owner-secret, the device-built tree, and the local recovery file

**Audience:** holder-app developers, and support staff explaining recovery to an owner.
**Scope:** Level-B (owner-unlinkable) tags only.
Level-A tags predate this and are unaffected.

> **This document describes a file that holds a RECOVERY SECRET.**
> `Documents/dogtag-owner-secrets.json` (iOS) / `<noBackupFilesDir>/dogtag-owner-secrets.json.enc`
> (Android) contains the owner-secret for every Level-B tag on the device.
> Anyone who reads it can generate that tag's proofs.
> It is never uploaded, never logged, and never leaves the device: it is excluded from device
> backups and encrypted at rest, so it is a **device-local store, not a cross-device backup**.
> Cross-device recovery is the 24-word phrase plus the credential - see below.

## Why the owner-secret exists, and why the DEVICE must derive it

A Level-B verification emits a `nullifier` on-chain and nothing else about the owner:

```
nullifier = Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)
```

The owner-secret is the only owner-private input in that preimage.
If a server generated it, that server could recompute every nullifier and link them back to the
owner, which is exactly the linkability Level B removes.
So the owner-secret is derived on the device, from the wallet seed, and is never transmitted.

The same reasoning is why the whole tree is built on-device: the app computes the root `R` locally
and hands the issuer only `R`.
The issuer seals it with `DogTagSBTConsent.mintCustodial(dogTagId, R)`, which takes no recipient
argument, so the owner's wallet appears nowhere on-chain.

The issuer-side route for that handoff is **`POST /profiles/issue/custodial-bind { token, root }`**
on the vet stack (M-2), where `token` is the one-time 180s bind token from the operator's QR and
`root` is `rootHex`.
It carries **no wallet and no signature** by design: `mintCustodial` has no recipient, so there is
nothing for a signature to attest, and sending a wallet anyway would hand the server exactly the link
this whole design removes.
The server treats `R` as opaque (it cannot recompute it - it has no seed), anchors it with `issue(R)`
and then seals it.
No shipped app posts to it yet; the device call site is a follow-up.

## What recovery actually needs: the seed AND the credential

Rebuilding a tag on a replacement device needs two inputs, and **both are required**:

1. **The wallet seed** (the 24-word phrase) regenerates the owner-control core: the owner-secret, the
   consent key, and the reserved-leaf salts.
2. **The credential's attribute leaves** supply the attribute values and their salts, which are
   caller-supplied and **not** seed-derivable. They come back from the issued wrapped credential
   itself, which packs every leaf as `"<saltHex>:<tag>:<value>"`.

Seed + credential attributes reproduce the same `R`; the seed alone does **not**.
Never tell an owner the 24-word phrase by itself rebuilds a tag.

The local file is **not** one of these paths.
It is a device-local store, excluded from device backups (see [The file](#the-file)), so it never
reaches a replacement device.

> **The owner MUST back up their 24-word recovery phrase.**
> It is the only cross-device path to the owner-secret.
> A device lost without a phrase backup means the owner-secret is gone and the tag can never be
> re-proven; per decision D3 the remedy is a fresh tag with a new id and a new `R`, not a rebind.

Because that loss is silent and permanent, the app does not rely on the owner reading the warning:
**owner-secret creation is gated on an explicit confirmation.**
`ProfileTreeStore.buildAndPersist` throws unless `SeedBackup.isConfirmed` - on iOS
`StoreError.seedBackupNotConfirmed` / `SeedBackup.isConfirmed(forSeedHex:)` (`Wallet.swift`), on
Android `SeedBackupNotConfirmedException` / `SeedBackup.isConfirmed(context, seedHex)`
(`wallet/Wallet.kt`) - so a tag cannot be created for an owner who has not affirmed they stored that
wallet's phrase offline.
**The confirm action must be reachable whenever the gate would refuse** - otherwise the gate is not a
prompt but a permanent lockout, since a tag can then never be created on that device again and there
is no on-chain remedy.
iOS reaches it from the wallet-genesis phrase card and again from the account export sheet
(`ProfileScreen.swift`).
Android instead has a STANDING "Recovery phrase backup" card on Profile
(`ui/screens/ProfileScreen.kt`), shown for any existing wallet whose backup is not known-confirmed.
It deliberately does NOT re-display the phrase and offers no reveal path: Android does not persist
BIP-39 entropy, so the 24 words are unrecoverable after genesis by design, and the owner is asserting
against their own written copy.
Binding it to the genesis phrase card alone would have been the lockout: `mnemonic` is set at exactly
one site (the `Wallet.create` success handler), so a user backgrounded before tapping could never
return to it.

Note the Android card treats *undetermined* as unconfirmed and shows the action.
`isConfirmed` is bound to a fingerprint of the seed, so answering it means decrypting the seed under
the biometric-gated Keystore key, which Profile deliberately does not do while composing; the flag is
resolved only from an already-authenticated path (genesis, unlock, or the confirm tap itself).
A cheap "some fingerprint exists in prefs" read must NOT stand in for it - a restored-prefs /
new-Keystore-seed mismatch is exactly what the fingerprint exists to catch, and rendering that as
confirmed would hide the action while the gate refused forever.

It records an assertion, not proof: the app cannot verify a phrase was really written down, and a
determined owner can tap through.
The gate closes the *silent* failure (a tag minted against a phrase the owner never saw), not the
dishonest one.
`SeedBackup` stores a domain-separated SHA-256 fingerprint of the confirmed seed - `UserDefaults` on
iOS, `SharedPreferences` on Android, under the same `DogTag/seed-backup-fingerprint/v1` domain.
The fingerprint is not a secret, and binding the assertion to it prevents a migrated preference
from confirming a new `…ThisDeviceOnly` Keychain seed. Absence or mismatch re-prompts.

### 1. Seed derivation

Every owner-control input is a pure function of the wallet seed, bound to `dogTagId`:

| input               | derivation                                                     |
|---------------------|----------------------------------------------------------------|
| owner-secret        | `BLAKE-512("DogTag/owner-secret/v1" ‖ dogTagId[32B BE] ‖ u64be(0) ‖ seed)`, wide-reduced mod r |
| consent-key (Ax,Ay) | `BLAKE-512("DogTag/consent-key/babyjubjub/v2" ‖ dogTagId[32B BE] ‖ u64be(0) ‖ seed)[0..32]` → `prv2pub` |
| reserved-leaf salts | `BLAKE-512("DogTag/reserved-leaf-salt/v1" ‖ dogTagId[32B BE] ‖ u64be(len(UTF8(keyPath))) ‖ UTF8(keyPath) ‖ seed)` |
| owner-address       | the wallet address, itself seed-derived                        |

Restoring the 24-word phrase restores the seed, which regenerates the owner-control core above -
which, together with the credential's attribute leaves, rebuilds the identical tree and the
identical `R`.
Binding to `dogTagId` means one wallet's two tags get independent secrets - independent owner-secrets
*and* independent consent keys `(Ax, Ay)` - so their nullifiers stay mutually unlinkable and no
owner-control value is shared across a wallet's tags.

Deriving from the seed does not weaken unlinkability: the seed never leaves the device and BLAKE-512
is one-way, so the secret stays exactly as opaque to an observer as a random one would be, while a
random secret would be unrecoverable the moment the device was lost.

### 2. The credential's attribute leaves

Seed derivation alone is not sufficient to rebuild the tree, because the tree also commits to the
credential's **attribute** leaves, whose values come from the issuer and whose salts are random.
Neither is derivable from the seed.
They travel in the issued wrapped credential itself - `wrap_document` packs each leaf as
`"<saltHex>:<tag>:<value>"` - so a holder who still has the credential (or can re-obtain it from the
issuer) has them, and that is the path a replacement device uses.

The local file records them too, but only as a device-local convenience: it is excluded from device
backups, so it is never the thing that carries them across devices.

## The file

Both platforms implement the same contract - device-local, encrypted at rest, excluded from device
backups - with their own OS mechanisms. The iOS ceremony below is not portable, so the Android store
mirrors the guarantees rather than the API; see [The Android file](#the-android-file).

- **Path:** `<App Container>/Documents/dogtag-owner-secrets.json` (iOS).
- **Written by:** `apps/ios/DogTag/ProfileTreeStore.swift`.
- **Protection:** new contents are staged in a `.completeFileProtection` sibling, then atomically
  replace the destination, so the file is encrypted at rest whenever the device is locked.
  The previous protected file is retained for rollback until the replacement is fully committed.
- **Device-local:** flagged `isExcludedFromBackup`, so it stays out of iCloud and Finder/iTunes
  device backups - deliberately at parity with the seed and entropy Keychain items, whose
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` class excludes them the same way.
  `.completeFileProtection` alone would **not** achieve this: it governs at-rest encryption, not
  backup inclusion, and `Documents/` is backed up by default.
  The empty protected staging file is excluded before secret bytes are written, and the flag is
  re-asserted after replacement because metadata preservation is not a documented guarantee.
- **Format:** a JSON array of records, one per tag.

```json
[
  {
    "dogTagIdHex": "0x…",           // canonical dogTagId field the tree is bound to
    "dogTagIdDec": "424242",        // the human-facing decimal id
    "ownerSecretHex": "0x…",        // SECRET - the nullifier's secret leaf. Never transmit.
    "rootHex": "0x…",               // R - the ONLY value the issuer ever sees
    "ownerAddress": "0x…",          // the owner's 20-byte wallet address
    "attributes": [                 // NOT seed-derivable: required to rebuild the tree
      {"keyPath": "credentialSubject.name", "saltHex": "0x…", "tag": 2, "value": "Rex"}
    ],
    "derivationVersion": "DogTag/owner-secret/v1",
    "savedAt": "2026-07-15T00:00:00Z"
    // --- re-issue bookkeeping (M6, D3); all OPTIONAL and DEVICE-LOCAL, absent on a normal tag ---
    // on an ABANDONED record:  "abandonedAt": "2026-07-16T…", "replacedByDogTagIdDec": "515151"
    // on the RE-ISSUED record: "replacesDogTagIdDec": "424242"
  }
]
```

`derivationVersion` is stamped so a future KDF change is detectable rather than silently producing a
different `R`.
It must track `profile_tree::OWNER_SECRET_DOMAIN` in the Rust core.

The three re-issue fields (`abandonedAt`, `replacedByDogTagIdDec`, `replacesDogTagIdDec`) are written
by `ProfileTreeStore.reissue` and are all optional, so a record written before M6 decodes unchanged.
They link an abandoned tag to the fresh tag that replaced it (decision D3), and they stay device-local
like every other field here - never transmit the old<->new link.

### The Android file

- **Path:** `<noBackupFilesDir>/dogtag-owner-secrets.json.enc`.
- **Written by:** `apps/android/app/src/main/java/io/liberalize/dogtag/profile/ProfileTreeStore.kt`.
- **Protection:** AES-256-GCM under a hardware-backed Android Keystore key
  (`dogtag_owner_secrets_key`), the same envelope `Wallet`'s `SeedVault` uses for the seed.
  The file is therefore ciphertext on disk, unlike iOS's plaintext-JSON-under-file-protection.
- **Unlocked-device gate, one capability per rung:** `ProfileTreeStore.ensureKey` generates the key
  down a ladder - `StrongBox + setUnlockedDeviceRequired(true)` → `setUnlockedDeviceRequired(true)`
  alone → plain - so a device only gives up what it actually cannot provide.
  That matters because the two capabilities are independent and fail at very different rates:
  `setIsStrongBoxBacked(true)` throws on every device without a secure element and on all emulators,
  while `setUnlockedDeviceRequired(true)` is the direct analogue of iOS's `.completeFileProtection`
  and is what this store's at-rest story rests on.
  Dropping both together - as the first cut did - silently encrypted recovery secrets under a key
  usable while the phone was locked, on every non-StrongBox device.
  Both capabilities are API 28+, so on API 26-27 only the plain rung exists.
  Reaching the plain rung is `Log.w`-reported rather than accepted in silence, so the weaker state is
  discoverable.
- **Not biometric-gated:** unlike the wallet seed's key, this one is not
  `setUserAuthenticationRequired`.
  `.completeFileProtection` gates on device lock, not on a fresh biometric per read, and this store
  is read on paths (listing a tag, rebuilding a proof) where prompting per access would be a UX
  divergence from iOS rather than parity with it.
  The seed itself keeps its stronger, auth-gated key.
- **Device-local:** written under `Context.getNoBackupFilesDir()`, which Android auto-backup and
  device-to-device transfer both skip.
  The manifest sets `allowBackup="true"` for the rest of the app, so that flag is deliberately *not*
  what this relies on - `noBackupFilesDir` is excluded regardless of it.
- **Writes:** no window ever holds zero readable copies of a store whose attribute salts exist
  nowhere else.
  New contents go to a sibling temp file that is `fsync`ed (`FileOutputStream.getFD().sync()`) before
  anything is moved - a bare `writeBytes` only reaches the page cache, so the rename could otherwise
  be durable while the bytes it points at are not.
  The current store is then *parked* as `dogtag-owner-secrets.json.enc.bak` rather than deleted, the
  staging file is renamed into place, and only then is the backup dropped.
  The `.bak` is the recoverable copy: any failure restores it, and no failure path may delete it,
  since it is a copy of a store that write did not create.
  The staging file is the opposite case and is dropped on every path - that write created it, it
  replaced nothing, and nothing ever reads it back, so keeping it would only accumulate whole
  encrypted copies of the store in a directory nothing sweeps.
  A crash inside the window where only the backup exists is repaired by `load()`, which promotes the
  `.bak` before it will report an absent store - reporting "no records" there would let `upsert`
  rebuild an empty list over the only surviving copy.
  This guarantees a recoverable copy always survives; it is not full power-loss atomicity, which
  would additionally need a directory `fsync` the JVM does not expose.
- **Format:** the same JSON array of records, encrypted.
  The three re-issue fields are **not** yet written on Android - `reissue` (D3) is iOS-only for now;
  see [Platform parity](#platform-parity).

## Platform parity

Android reached parity with iOS's device-side tree build in M-2b.
Both platforms call the SAME compiled Rust core over UniFFI (`buildProfileTreeHex`), so `R` is
byte-for-byte identical for identical inputs by construction rather than by two implementations
agreeing - `apps/android/.../ProfileTreeParityTest.kt` pins that against the shared
`contracts/test/device-profile-root.json` fixture.

| capability | iOS | Android |
|---|---|---|
| device-side tree build + `R` | yes | yes (`ProfileTreeBuilder`) |
| seed-derived owner-secret, persisted | yes | yes (`ProfileTreeStore`) |
| single-owner-triple guard | in the Rust core | Rust core + a fail-fast Kotlin mirror |
| write-once-root conflict check | yes | yes (`OwnerSecretRecords.upsert`) |
| seed-backup gate | yes | yes (`SeedBackup`) |
| recovery round-trip (`verifyRecoverable`) | yes | yes |
| **re-issue (D3, `reissue`)** | yes | **not yet** |

The remaining gap is the re-issue affordance: an Android owner whose owner-secret is permanently lost
has the same on-chain remedy (a fresh custodial issuance under a new `dogTagId`), but the app does not
yet record the abandoned↔fresh bookkeeping that `ProfileTreeStore.reissue` writes on iOS.

## Handling rules

- **Never** transmit, log, or include the file in a bug report or analytics payload.
  Only `rootHex` is safe to send, and only to the issuer at issuance.
- **The one deliberate exception: the `/prove-consent` server-prove fallback.**
  That route exists for devices that cannot run the Groth16 prove locally, and the assembled
  `circuit_input` it receives carries `ownerSecret` and `ownerAddress` by construction.
  The wallet seed still never leaves the device, so a compromised prover operator cannot reach the
  owner's other tags or forge future consents - but it CAN name the owner and link that tag's entire
  verification history, because it can recompute the nullifier.
  Owner-unlinkability therefore holds against a chain observer and against the relayer, and does
  **not** hold against the prover operator.
  On-device proving leaks none of this and remains the default; see the trust-boundary note on
  `ConsentProver::prove` in `stacks/vet/api/src/prover.rs`.
- Treat it exactly like the 24-word phrase in any UX that offers export or backup.
- It is neither iCloud-synced nor included in device backups, matching the Keychain items'
  `…ThisDeviceOnly` protection class.
  A user who loses the device recovers via the phrase plus the credential, never via this file - it
  does not survive the device.

## Where the code lives

| piece | path |
|---|---|
| tree + KDF (source of truth) | `crates/dogtag-standard-rs/src/profile_tree.rs` |
| FFI surface | `crates/dogtag-standard-rs/src/ffi.rs` (`buildProfileTreeHex`, `deriveOwnerSecretHex`) |
| iOS store + device-local file | `apps/ios/DogTag/ProfileTreeStore.swift` |
| Android tree builder + triple guard | `apps/android/app/src/main/java/io/liberalize/dogtag/profile/ProfileTreeBuilder.kt` |
| Android store + device-local file | `apps/android/app/src/main/java/io/liberalize/dogtag/profile/ProfileTreeStore.kt` |
| Android record codec + write-once upsert | `apps/android/app/src/main/java/io/liberalize/dogtag/profile/OwnerSecretRecord.kt` |
| re-issue affordance (D3) | `apps/ios/DogTag/ProfileTreeStore.swift` (`ProfileTreeStore.reissue`) |
| seed accessor | `apps/ios/DogTag/Wallet.swift` (`Wallet.seedHex()`) / `wallet/Wallet.kt` (`Wallet.seedHex(context)`) |
| seed-backup gate | `Wallet.swift` / `wallet/Wallet.kt` (`SeedBackup`), enforced in `ProfileTreeStore.buildAndPersist` |
| confirmation UI | `apps/ios/DogTag/ProfileScreen.swift` / `ui/screens/ProfileScreen.kt` (the "I've saved it" action) |

The math is in Rust so the Poseidon parameter set and the reserved-leaf encoding stay pinned in one
place across Rust / TS / circom.
Swift never reimplements it.

## Tests that hold this together

- `crates/dogtag-standard-rs/src/profile_tree.rs` - `owner_secret_and_root_regenerate_from_the_seed`
  is the recovery round-trip: regenerate the secret from the seed, rebuild the tree, assert the same
  `R`.
- `crates/dogtag-standard-rs/tests/profile_tree_parity.rs` - asserts the device builder's primitives
  reproduce the `R` that the M2 circuit proved and the M4 registry verified on-chain.
- `crates/dogtag-standard-rs/tests/device_recovery_journey.rs` - the REPAIR branch through the FFI:
  seed + credential rebuild the same `R`; a wrong phrase or a lost credential does not.
- `crates/dogtag-standard-rs/tests/device_reissue_journey.rs` - the RE-ISSUE branch (D3): a lost
  owner-secret is recovered by re-issuing a fresh tag under a NEW `dogTagId`, with an independent
  owner-secret and `R` that keep it mutually unlinkable from the abandoned one.
- `apps/android/app/src/test/java/io/liberalize/dogtag/profile/ProfileTreeParityTest.kt` - the Android
  leg: asserts the Kotlin builder reproduces the shared `device-profile-root.json` `R` byte-for-byte
  over the real Rust core, plus the seed round-trip, per-tag independence, and the single-owner-triple
  guard. Pure JVM; `app/build.gradle.kts` builds `dogtag-standard-rs` for the HOST and puts it on
  `jna.library.path` first, because the `jniLibs/` `.so` files are Android-ABI-only and gitignored.
- `apps/android/app/src/test/java/io/liberalize/dogtag/profile/OwnerSecretRecordsTest.kt` - the
  write-once-root invariant and the seed-backup fingerprint binding.
- `apps/android/app/src/test/java/io/liberalize/dogtag/profile/OwnerSecretCodecTest.kt` - the record
  JSON round-trip. A mistyped field name or a mangled attribute type tag would encode fine and decode
  into a DIFFERENT witness, which rebuilds a different `R` - unrecoverable, because `profileRoot` is
  write-once. Needs Robolectric: on the unit-test classpath `org.json` is Android's throwing stub, so
  this runs against the REAL parser the app ships with.
- `apps/android/app/src/test/java/io/liberalize/dogtag/profile/OwnerSecretRecoveryJourneyTest.kt` -
  the two halves as the owner meets them: build a tag on-device, lose the phone, and rebuild the SAME
  `R` from the phrase plus this file. Its negative leg asserts the file ALONE does not rebuild the tag,
  which is what stops "recovery works" from passing while the phrase is being ignored.
- `contracts/test/CustodialIssuance.t.sol` -
  `test_device_built_root_is_what_the_contract_stores_as_profileRoot` mints a real device-built `R`
  and asserts `profileRoot(dogTagId) == R`.

Those are separate legs over different roots: the demo root stored by the contract test is not
itself circuit-proven. Generating a proof over that seed-derived root belongs to M7.

## A note on "existing proofs keep verifying"

`profileRoot` is **write-once** on-chain, so `R` can never be moved for a tag that has been issued.
Recovery is therefore not about keeping old proofs valid (they are bound to an immutable `R` and stay
valid regardless).
It is about being able to generate **new** proofs after a device loss, which needs the full witness:
the owner-secret, the consent key, the salts, and the attribute leaves.
That is why the seed and the credential must BOTH come back, not just the secret.

Losing either (no phrase, or no credential to re-obtain the attribute leaves from) is unrecoverable
by design: per decision D3, the remedy is a fresh tag with a new tree and a new `R`, not a rebind of
the old one.

## The two recovery branches: repair vs re-issue (D3)

Which branch applies is decided by whether the owner-secret can be regenerated:

- **Repair (same tag).**
  The owner still has the phrase AND the credential's attribute leaves.
  Seed + credential rebuild the identical `R`, so the SAME tag keeps working - `profileRoot` is
  write-once and is never moved.
  This is the round-trip in [Tests that hold this together](#tests-that-hold-this-together)
  (`device_recovery_journey.rs`, and `OwnerSecretRecoveryJourneyTest.kt` for the Android leg).
- **Re-issue (fresh tag).**
  The owner-secret is gone for good.
  There is no on-chain repair, so the remedy is a **fresh custodial issuance under a new `dogTagId`
  with a new `R`**: the issuer allocates a fresh id (a burned/abandoned id is retired forever), the
  app builds a fresh tree (`ProfileTreeStore.reissue`, **iOS only** so far - see
  [Platform parity](#platform-parity)), and the issuer seals the new root.
  The abandoned tag is simply left behind - there is no rebind.
  Any credentials another issuer anchored to the abandoned id are **not** carried over (that would
  forge attestation applicability); the owner re-obtains each fresh from its issuer under the new id.
  M-2's `POST /profiles/issue/custodial-bind` supplies the MECHANICAL half of this - a fresh session
  allocates a fresh `dogTagId` and the device posts the new `R`, so a re-issue is just an issuance of
  a new tag - but there is no re-issue-AWARE issuer flow: nothing marks the abandoned tag or links
  old to new, and that link must stay device-local anyway (see below).
  M6 remains the device/app flow and the recovery semantics.

The re-issued tag is **mutually unlinkable** from the abandoned one.
The owner-secret is bound to `dogTagId` ([§1](#1-seed-derivation)), so even the same wallet's fresh
tag derives an independent nullifier secret; the abandoned tag's on-chain nullifiers cannot be
correlated with the re-issued tag's.
A "recovery" that reused the secret, or that surfaced the old<->new link anywhere off the device,
would reintroduce exactly the linkage Level B removes.
`ProfileTreeStore.reissue` MAY record the old<->new link because the store is excluded from device
backups and never transmitted; that link must never reach an on-chain event, a status reason, or an
issuer record.
