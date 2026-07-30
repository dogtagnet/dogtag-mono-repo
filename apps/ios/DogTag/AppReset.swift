import Foundation

/// Every "destroy my local state" action, in one place, so the contract for what each one really
/// costs cannot drift between the UI copy and the code that runs.
///
/// # These actions are device-local, and that is exactly what makes them irreversible
///
/// Nothing here touches the chain. An anchored dog tag and the records issued against it stay
/// on-chain, held by the custodian, and the tag's `profileRoot` (`R`) keeps whatever value it was
/// sealed with at issuance.
///
/// That asymmetry is the whole danger. `profileRoot` is **write-once** on `DogTagSBTConsent`, and
/// `proveConsent` re-derives the owner-secret from the wallet seed on every proof - the stored
/// `ownerSecretHex` is deliberately never handed across that seam (`ScanScreen`). Rebuilding a tag's
/// tree needs BOTH the seed and the credential's attribute salts, and the salts are caller-supplied,
/// not seed-derivable, and live only in `ProfileTreeStore`'s backup-excluded file. Destroy either
/// half and every tag bound to it can never prove consent again. It is not recoverable by
/// re-importing the tag, restoring a device backup, or re-scanning the vet's QR; per decision D3 the
/// only remedy is a fresh custodial issuance under a NEW `dogTagId`.
///
/// So every entry point here must state, before it runs: the on-chain tag survives, the ability to
/// use it does not.
///
/// All functions mutate published store state and must be called on the main thread.
enum AppReset {
    /// The union of a reset's file sweeps plus the non-file items (Keychain, `UserDefaults`) that no
    /// `LocalDataSweep` covers. A non-empty `failures` means the action half-ran and the UI must say
    /// so instead of reporting success.
    typealias Outcome = LocalDataSweep.Outcome

    /// Wipe the embedded wallet: the Keychain seed, the entropy that re-derives the 24 words, and the
    /// recovery-phrase backup confirmation bound to them. Returns the app to its first-run wallet
    /// state (`Wallet.exists()` false, so Profile offers "Create embedded wallet" again).
    ///
    /// Leaves `ProfileTreeStore` and the pet/credential files alone - see the type docs for why that
    /// does NOT make the tags recoverable. Callers wanting a clean slate should use
    /// `resetEverything()`; a wallet-only wipe leaves owner-secret records whose seed is gone, and
    /// re-binding those same `dogTagId`s under a new wallet fails closed (`ScanScreen` rejects a
    /// different owner address for an already-rooted id), which is correct but confusing.
    static func deleteWallet() -> Outcome {
        var outcome = Outcome()
        do {
            try Wallet.deleteKeys()
            outcome.removed.append("wallet seed + recovery entropy (Keychain)")
        } catch {
            outcome.failures.append(.init(name: "wallet seed + recovery entropy (Keychain)", error: error))
        }
        SeedBackup.clear()
        outcome.removed.append("recovery-phrase backup confirmation")
        return outcome
    }

    /// Wipe every dog tag held on this device: the owner-secret store (owner-secrets, `R`, and the
    /// non-derivable attribute salts), the pet rows, and their local photos.
    static func deleteDogTags() -> Outcome {
        var outcome = Outcome()
        outcome.merge(ProfileTreeStore.deleteAll())
        outcome.merge(LocalStore.shared.deleteAllPets())
        outcome.merge(PetPhotoStore.shared.removeAll())
        return outcome
    }

    /// Wipe every imported credential/record. The gentlest action here: the vet issued these and they
    /// stay on-chain, so they can be imported again from a fresh vet QR.
    static func deleteRecords() -> Outcome {
        LocalStore.shared.deleteAllCredentials()
    }

    /// Everything above, in one action: wallet, dog tags, records.
    ///
    /// Deliberately ordered data-first, wallet-last, and the wallet is wiped ONLY once every data
    /// sweep has fully succeeded. The data sweeps are what carry the unrecoverable material (the
    /// attribute salts), so a sweep failure must leave the seed present: that is what keeps the
    /// reported failure actionable - retry, or export the keys again. Wiping the seed anyway would
    /// destroy the one remedy while leaving the leftover it failed to remove readable on disk, so a
    /// partial sweep returns early and the caller must report that the wallet was deliberately kept.
    ///
    /// Appearance preferences (theme, brightness) are NOT touched: they hold no account or pet data,
    /// and silently resetting them would misrepresent this as a factory wipe.
    ///
    /// The stored provider-directory copy (`FileProviderDirectoryCacheStore`) is NOT touched either,
    /// for the same reason: it is one public endpoint's response, holds no account, pet or position
    /// data, and the phone can fetch it again. Folding it in would also put a re-fetchable public
    /// file inside the short-circuit above, where failing to delete it would block the wallet wipe -
    /// a cache file veto-ing the action whose whole point is destroying secrets.
    static func resetEverything() -> Outcome {
        var outcome = Outcome()
        outcome.merge(deleteDogTags())
        outcome.merge(deleteRecords())
        guard outcome.isComplete else { return outcome }
        outcome.merge(deleteWallet())
        return outcome
    }
}
