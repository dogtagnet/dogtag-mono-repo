# MOBILE_OWNER_SECRET - the owner-secret, the device-built tree, and the local recovery file

**Audience:** holder-app developers, and support staff explaining recovery to an owner.
**Scope:** Level-B (owner-unlinkable) tags only.
Level-A tags predate this and are unaffected.

> **This document describes a file that holds a RECOVERY SECRET.**
> `Documents/dogtag-owner-secrets.json` contains the owner-secret for every Level-B tag on the
> device.
> Anyone who reads it can generate that tag's proofs.
> It is never uploaded, never logged, and never leaves the device.

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

## The two recovery paths (belt-and-suspenders)

Both must hold, and both are needed: they are **complementary, not redundant**.
Seed derivation regenerates the owner-control core (the owner-secret, the consent key, the
reserved-leaf salts); the local file supplies the attribute values and their salts, which are not
seed-derivable.
Neither path alone reproduces `R`, and `R` is what every existing proof is bound to - so the 24-word
phrase by itself is **not** sufficient to rebuild a tag.

### 1. Seed derivation (primary)

Every owner-control input is a pure function of the wallet seed, bound to `dogTagId`:

| input               | derivation                                                     |
|---------------------|----------------------------------------------------------------|
| owner-secret        | `BLAKE-512("DogTag/owner-secret/v1" ‖ dogTagId ‖ seed)`, wide-reduced mod r |
| consent-key (Ax,Ay) | `BLAKE-512("DogTag/consent-key/babyjubjub/v1" ‖ seed)` (pre-existing)       |
| reserved-leaf salts | `BLAKE-512("DogTag/reserved-leaf-salt/v1" ‖ dogTagId ‖ keyPath ‖ seed)`     |
| owner-address       | the wallet address, itself seed-derived                        |

Restoring the 24-word phrase restores the seed, which regenerates all of the above, which rebuilds
the identical tree and the identical `R`.
Binding to `dogTagId` means one wallet's two tags get independent secrets, so their nullifiers stay
mutually unlinkable.

Deriving from the seed does not weaken unlinkability: the seed never leaves the device and BLAKE-512
is one-way, so the secret stays exactly as opaque to an observer as a random one would be, while a
random secret would be unrecoverable the moment the device was lost.

### 2. The local file (backup)

Seed derivation alone is not sufficient to rebuild the tree, because the tree also commits to the
credential's **attribute** leaves, whose values come from the issuer and whose salts are random.
Those are not derivable from the seed, so they are recorded here alongside the secret.

## The file

- **Path:** `<App Container>/Documents/dogtag-owner-secrets.json` (iOS).
- **Written by:** `apps/ios/DogTag/ProfileTreeStore.swift`.
- **Protection:** written atomically with `.completeFileProtection`, so it is encrypted at rest
  whenever the device is locked.
  A torn write would cost the owner a tag's recoverability, hence the atomic write.
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
  }
]
```

`derivationVersion` is stamped so a future KDF change is detectable rather than silently producing a
different `R`.
It must track `profile_tree::OWNER_SECRET_DOMAIN` in the Rust core.

## Handling rules

- **Never** transmit, log, or include the file in a bug report or analytics payload.
  Only `rootHex` is safe to send, and only to the issuer at issuance.
- Treat it exactly like the 24-word phrase in any UX that offers export or backup.
- It is not iCloud-synced today, matching the Keychain items' `…ThisDeviceOnly` protection class.
  A user who loses the device recovers via the phrase (path 1), not via this file.

## Where the code lives

| piece | path |
|---|---|
| tree + KDF (source of truth) | `crates/dogtag-standard-rs/src/profile_tree.rs` |
| FFI surface | `crates/dogtag-standard-rs/src/ffi.rs` (`buildProfileTreeHex`, `deriveOwnerSecretHex`) |
| iOS store + backup file | `apps/ios/DogTag/ProfileTreeStore.swift` |
| seed accessor | `apps/ios/DogTag/Wallet.swift` (`Wallet.seedHex()`) |

The math is in Rust so the Poseidon parameter set and the reserved-leaf encoding stay pinned in one
place across Rust / TS / circom.
Swift never reimplements it.

## Tests that hold this together

- `crates/dogtag-standard-rs/src/profile_tree.rs` - `owner_secret_and_root_regenerate_from_the_seed`
  is the recovery round-trip: regenerate the secret from the seed, rebuild the tree, assert the same
  `R`.
- `crates/dogtag-standard-rs/tests/profile_tree_parity.rs` - asserts the device builder reproduces
  the `R` that the M2 circuit proved and the M4 registry verified on-chain, so a device-built tree is
  provable rather than merely self-consistent.
- `contracts/test/CustodialIssuance.t.sol` -
  `test_device_built_root_is_what_the_contract_stores_as_profileRoot` mints a real device-built `R`
  and asserts `profileRoot(dogTagId) == R`.

## A note on "existing proofs keep verifying"

`profileRoot` is **write-once** on-chain, so `R` can never be moved for a tag that has been issued.
Recovery is therefore not about keeping old proofs valid (they are bound to an immutable `R` and stay
valid regardless).
It is about being able to generate **new** proofs after a device loss, which needs the full witness:
the owner-secret, the consent key, the salts, and the attribute leaves.
That is why both recovery paths above must hold, not just the secret.

Losing both (no phrase and no file) is unrecoverable by design: per decision D3, the remedy is a
fresh tag with a new tree and a new `R`, not a rebind of the old one.
