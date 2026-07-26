import SwiftUI
import UniformTypeIdentifiers

struct ProfileScreen: View {
    @Environment(\.dogTagColors) var c
    @EnvironmentObject var theme: ThemeManager
    @ObservedObject private var store = LocalStore.shared
    private let roax = RoaxConfig.load()

    @State private var walletExists = Wallet.exists()
    /// Presence-only (never pulls the entropy into memory); false marks the legacy, phrase-less wallet
    /// whose issuance gate can only be cleared by replacing it.
    @State private var hasExportablePhrase = Wallet.hasExportablePhrase()
    @State private var ethAddr: String? = nil
    @State private var mnemonic: String? = nil
    @State private var walletMsg = ""
    @State private var sheet: ProfileSheet? = nil
    @State private var resetMsg = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Profile").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

                #if DEBUG
                // Debug-only on-device ZK self-test (drives the real Groth16 prover; never in release).
                ZkSelfTestCard()
                #endif

                // ---- Appearance ----
                SectionTitle(text: "Appearance")
                Text("Theme").font(.system(size: 13)).foregroundColor(c.muted)
                HStack(spacing: 10) {
                    ForEach(ThemeId.allCases) { t in
                        let selected = t == theme.themeId
                        Button { theme.themeId = t } label: {
                            ZStack {
                                Circle().fill(t.accent).frame(width: 36, height: 36)
                                if selected {
                                    Circle().stroke(c.onBackground, lineWidth: 3).frame(width: 36, height: 36)
                                    Image(systemName: "checkmark").foregroundColor(.white).font(.system(size: 14, weight: .bold))
                                }
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.plain)
                    }
                }

                Text("Brightness").font(.system(size: 13)).foregroundColor(c.muted)
                Picker("Brightness", selection: $theme.darkPref) {
                    ForEach(DarkPref.allCases) { p in Text(p.label).tag(p) }
                }
                .pickerStyle(.segmented)

                // ---- Embedded wallet ----
                SectionTitle(text: "Embedded wallet")
                VStack(alignment: .leading, spacing: 8) {
                    Text("A self-custodial recovery seed creates each dog tag's private owner proof on this device. The seed is stored in the iOS Keychain (hardware-protected, this-device-only); reveal is biometric-gated.")
                        .font(.system(size: 12)).foregroundColor(c.muted)

                    if !walletExists {
                        walletButton("Create embedded wallet") {
                            Biometric.authenticate(reason: "Authenticate to generate your keys") { ok, e in
                                guard ok else { walletMsg = e ?? "auth failed"; return }
                                do {
                                    let id = try Wallet.create()
                                    walletExists = true
                                    // Genesis persists the entropy, so this wallet's phrase is
                                    // exportable - it is never the legacy shape.
                                    hasExportablePhrase = true
                                    ethAddr = id.ethAddress
                                    mnemonic = id.mnemonic
                                    walletMsg = "Wallet created. Back up your recovery phrase now."
                                } catch {
                                    walletMsg = "create failed: \(error)"
                                    // create() writes the entropy before the seed, so a partial
                                    // failure must not leave the UI asserting either flag.
                                    walletExists = Wallet.exists()
                                    hasExportablePhrase = Wallet.hasExportablePhrase()
                                }
                            }
                        }
                    } else {
                        walletButton("Unlock & show keys") {
                            Biometric.authenticate(reason: "Authenticate to reveal your keys") { ok, e in
                                guard ok else { walletMsg = e ?? "auth failed"; return }
                                do {
                                    let id = try Wallet.load()
                                    ethAddr = id?.ethAddress
                                    walletMsg = "Unlocked."
                                } catch { walletMsg = "unlock failed: \(error)" }
                            }
                        }
                        // Self-custody export: reveal + copy the recovery phrase AND the raw secp256k1
                        // private key (biometric-gated, same Face ID gate as the reveal above; the
                        // secrets are never logged or transmitted).
                        walletButtonSecondary("Export account keys") { beginExport() }
                    }

                    // Tap any value to copy it (full value, not the truncated preview).
                    if let a = ethAddr { CopyRow(label: "Account", value: a) }
                    if let m = mnemonic {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Recovery phrase (24 words)").font(.system(size: 12, weight: .bold)).foregroundColor(c.danger)
                            Text("Write these down and store them offline. Anyone with them controls your wallet.")
                                .font(.system(size: 11)).foregroundColor(c.muted)
                            Text(m).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
                                .hideWhenInactive()
                            HStack(spacing: 10) {
                                CopyButton(title: "Copy phrase", value: m, secret: true)
                                SeedBackupConfirmationButton {
                                    mnemonic = nil
                                    walletMsg = "Recovery phrase hidden. Export it again anytime from “Export account keys”."
                                }
                            }
                        }
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.surfaceVariant))
                    }
                    if !walletMsg.isEmpty { Text(walletMsg).font(.system(size: 12)).foregroundColor(c.muted) }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                // ---- Dog-tags: owner-hidden tags created from scanned vet issuance sessions ----
                SectionTitle(text: "Dog-tags")
                VStack(alignment: .leading, spacing: 6) {
                    let minted = store.pets.filter { !$0.dogTagId.isEmpty && $0.dogTagId.allSatisfy { $0.isNumber } }
                    if minted.isEmpty {
                        Text("No dog tag yet. Scan your vet's dog-tag QR (Scan) to create its private owner proof and anchor the tag.")
                            .font(.system(size: 12)).foregroundColor(c.muted)
                    } else {
                        ForEach(minted) { pet in
                            CopyRow(label: pet.name.isEmpty ? "Pet" : pet.name,
                                    value: pet.dogTagId,
                                    display: "dogTagId \(pet.dogTagId)")
                        }
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                // ---- Network ----
                SectionTitle(text: "Network")
                VStack(alignment: .leading, spacing: 4) {
                    kv("Chain", "ROAX (chainId \(roax.chainId))")
                    kv("DogTagSBT", String(roax.dogTagSbt.prefix(16)) + "…")
                    kv("IssuerRegistry", String(roax.issuerRegistry.prefix(16)) + "…")
                    kv("ProtocolRegistry", roax.protocolRegistry.isEmpty
                       ? "pending deployment" : String(roax.protocolRegistry.prefix(16)) + "…")
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                dangerZone

                Spacer(minLength: 24)
            }
            .padding(20)
        }
        // Deliver the revealed secrets through the sheet's item payload. A prior `.sheet(isPresented:)`
        // read sibling @State that was still nil when SwiftUI first evaluated the sheet body, so the
        // phrase/key never displayed. Dismiss nils the binding, releasing the secrets from memory.
        .sheet(item: $sheet) { route in
            Group {
                switch route {
                case let .export(payload):
                    ExportAccountSheet(
                        mnemonic: payload.mnemonic,
                        privateKeyHex: payload.privateKeyHex,
                        // Swapping the route re-renders this same sheet as the confirmation, so the
                        // hand-off never crosses a dismiss/present animation.
                        onReplaceWallet: { sheet = .danger(.replaceWallet) })
                case let .danger(action):
                    DestructiveConfirmationSheet(action: action) {
                        sheet = nil
                        // Let the sheet finish dismissing before the LAContext prompt - and, after a
                        // replace, before the fresh-phrase card appears underneath it.
                        DispatchQueue.main.async { perform(action) }
                    }
                }
            }
            .environment(\.dogTagColors, c)
        }
    }

    // ---- danger zone -------------------------------------------------------------------------------

    /// Destructive local-data actions, last on the screen so they are never in the way of the
    /// read-only sections above. Export leads, because a user should always be able to get their keys
    /// out before destroying anything.
    @ViewBuilder
    private var dangerZone: some View {
        SectionTitle(text: "Danger zone")
        VStack(alignment: .leading, spacing: 12) {
            Text("These erase data held on THIS device and cannot be undone. Nothing on-chain is deleted - your dog tag and its records stay on-chain with the custodian - but the owner-secret that proves you own them lives only here.")
                .font(.system(size: 12)).foregroundColor(c.muted)

            // A phrase-less wallet can NEVER issue a dog tag, and the reset section is where someone
            // who is stuck comes looking for a way out - so the rescue is surfaced here as well as in
            // the export sheet. It leads, and it is not styled as one of the destructive chores below.
            if walletExists && !hasExportablePhrase {
                VStack(alignment: .leading, spacing: 8) {
                    Label("This wallet can't show you a recovery phrase", systemImage: "exclamationmark.triangle.fill")
                        .font(.system(size: 13, weight: .bold)).foregroundColor(c.onBackground)
                    Text("It was created before phrase export was supported, so DogTag can't ask you to confirm a backup - and issuing a dog tag needs that confirmation. Replacing it with a fresh, phrase-backed wallet is the way to get unblocked. Export your keys first.")
                        .font(.system(size: 12)).foregroundColor(c.muted)
                        .fixedSize(horizontal: false, vertical: true)
                    Button { sheet = .danger(.replaceWallet) } label: {
                        Text("Replace wallet…").font(.system(size: 14, weight: .semibold))
                            .foregroundColor(c.onAccent)
                            .padding(.vertical, 10).padding(.horizontal, 14)
                            .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("dangerRow-replaceWallet")
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 14).fill(c.surface))
            }

            if walletExists {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Get your keys out first").font(.system(size: 13, weight: .semibold))
                        .foregroundColor(c.onBackground)
                    Text("Export the recovery phrase (when available) and the private key before you destroy anything.")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                    walletButtonSecondary("Export account keys") { beginExport() }
                }
                Divider().overlay(c.outline)
            }

            ForEach(dangerActions) { action in
                dangerRow(action)
            }

            if !resetMsg.isEmpty {
                Text(resetMsg).font(.system(size: 12)).foregroundColor(c.onBackground)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.danger.opacity(0.10)))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(c.danger.opacity(0.35), lineWidth: 1))
    }

    /// "Delete wallet" is only offered when there is a wallet to delete; the rest always apply, since
    /// running one over already-empty state is a harmless no-op and hiding them would leave a user
    /// wondering whether stale data is still there.
    private var dangerActions: [DangerAction] {
        (walletExists ? [DangerAction.deleteWallet] : []) + [.deleteDogTags, .deleteRecords, .resetEverything]
    }

    private func dangerRow(_ action: DangerAction) -> some View {
        Button { sheet = .danger(action) } label: {
            HStack(alignment: .top, spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(action.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.danger)
                    Text(action.detail).font(.system(size: 11)).foregroundColor(c.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").font(.system(size: 12)).foregroundColor(c.muted)
                    .padding(.top, 2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("dangerRow-\(action.rawValue)")
    }

    // ---- actions -----------------------------------------------------------------------------------

    /// Reveal the phrase (when the wallet has one) and the private key behind the same fresh biometric
    /// gate the wallet card has always used.
    private func beginExport() {
        Biometric.authenticate(reason: "Authenticate to export your account keys") { ok, e in
            guard ok else { walletMsg = e ?? "auth failed"; return }
            sheet = .export(ExportPayload(
                mnemonic: Wallet.revealMnemonic(),
                privateKeyHex: Wallet.revealPrivateKeyHex()))
        }
    }

    /// Run a confirmed destructive action behind the same LAContext gate as the export flow.
    private func perform(_ action: DangerAction) {
        Biometric.authenticate(reason: action.biometricReason) { ok, e in
            guard ok else { resetMsg = e ?? "auth failed"; return }
            switch action {
            case .replaceWallet:
                replaceWallet()
            case .deleteWallet:
                report(AppReset.deleteWallet(), action: action,
                       success: "Wallet deleted. Create a new one above to start over.")
                forgetWalletUi()
            case .deleteDogTags:
                report(AppReset.deleteDogTags(), action: action,
                       success: "Dog-tags deleted from this device.")
            case .deleteRecords:
                report(AppReset.deleteRecords(), action: action,
                       success: "Records deleted from this device.")
            case .resetEverything:
                report(AppReset.resetEverything(), action: action,
                       success: "Everything on this device was erased. Create a new wallet above to start over.")
                forgetWalletUi()
            }
        }
    }

    /// The legacy-wallet rescue: destroy the phrase-less wallet and immediately surface the fresh
    /// phrase, so the user lands on the "I've saved it" confirmation that unblocks issuance rather
    /// than back at the dead end they started from.
    private func replaceWallet() {
        do {
            let id = try Wallet.replace()
            walletExists = true
            hasExportablePhrase = true
            ethAddr = id.ethAddress
            mnemonic = id.mnemonic
            walletMsg = "New wallet created. Write the 24 words below down, then tap “I've saved it” - issuing a dog tag stays blocked until you do."
            resetMsg = "Wallet replaced. Your previous account and any dog tag bound to it are gone for good."
        } catch {
            // `replace()` deletes before it creates, so a failure lands on one of two very different
            // states. Re-read which one rather than assuming: telling a user their wallet is intact
            // when it has already been removed is the one message that must never be wrong.
            walletExists = Wallet.exists()
            hasExportablePhrase = Wallet.hasExportablePhrase()
            if walletExists {
                resetMsg = "Replace failed: \(error.localizedDescription). Your existing wallet is untouched."
            } else {
                ethAddr = nil
                mnemonic = nil
                walletMsg = ""
                resetMsg = "Replace failed after the old wallet was removed: \(error.localizedDescription). "
                    + "There is no wallet on this device now - use “Create embedded wallet” above."
            }
        }
    }

    /// Drop the wallet values this screen is holding, and re-read whether a wallet still exists rather
    /// than assuming the delete fully succeeded.
    private func forgetWalletUi() {
        walletExists = Wallet.exists()
        hasExportablePhrase = Wallet.hasExportablePhrase()
        ethAddr = nil
        mnemonic = nil
        walletMsg = ""
    }

    /// Report what actually happened. A partial sweep must never read as success: the leftover holds
    /// the same secrets the action promised to destroy.
    private func report(_ outcome: AppReset.Outcome, action: DangerAction, success: String) {
        if let failures = outcome.failureSummary {
            resetMsg = "\(action.title) only partly completed - could not remove \(failures). Try again."
        } else {
            resetMsg = success
        }
    }

    private func walletButton(_ title: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).padding(.vertical, 10).padding(.horizontal, 14)
                .foregroundColor(c.onAccent)
                .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
        }
    }

    /// A quieter, outlined variant of `walletButton` for the secondary export action.
    private func walletButtonSecondary(_ title: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.system(size: 15, weight: .semibold))
                .padding(.vertical, 10).padding(.horizontal, 14)
                .foregroundColor(c.accent)
                .background(RoundedRectangle(cornerRadius: 10).stroke(c.accent, lineWidth: 1.5))
        }
    }

    private func kv(_ k: String, _ v: String) -> some View {
        HStack(alignment: .top) {
            Text(k).font(.system(size: 12)).foregroundColor(c.muted).frame(width: 150, alignment: .leading)
            Text(v).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
            Spacer()
        }
    }
}

/// A labelled value row that copies its FULL value to the clipboard on tap and flashes a brief
/// "Copied" confirmation. The on-screen text may be a shortened preview; the clipboard always gets
/// the whole value. Used for the local account address and dog-tag ids.
private struct CopyRow: View {
    @Environment(\.dogTagColors) var c
    let label: String
    let value: String
    var display: String? = nil
    @State private var copied = false

    private var shown: String {
        if let d = display { return d }
        return value.count > 24 ? "\(value.prefix(12))…\(value.suffix(8))" : value
    }

    var body: some View {
        Button {
            UIPasteboard.general.string = value
            flash()
        } label: {
            HStack(alignment: .top, spacing: 8) {
                Text(label).font(.system(size: 12)).foregroundColor(c.muted)
                    .frame(width: 110, alignment: .leading)
                Text(shown).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
                Spacer(minLength: 4)
                if copied {
                    Text("Copied").font(.system(size: 11, weight: .semibold)).foregroundColor(c.success)
                }
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 12)).foregroundColor(copied ? c.success : c.muted)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func flash() {
        withAnimation { copied = true }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.6) { withAnimation { copied = false } }
    }
}

/// A pill copy button that flashes "Copied". When `secret` is true the value is placed on the
/// clipboard with a short auto-expiry so a recovery phrase does not linger there.
private struct CopyButton: View {
    @Environment(\.dogTagColors) var c
    let title: String
    let value: String
    var secret: Bool = false
    @State private var copied = false

    var body: some View {
        Button {
            if secret { SecureClipboard.copySecret(value) } else { UIPasteboard.general.string = value }
            withAnimation { copied = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.6) { withAnimation { copied = false } }
        } label: {
            Label(copied ? "Copied" : title, systemImage: copied ? "checkmark" : "doc.on.doc")
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(copied ? c.success : c.accent)
                .padding(.vertical, 8).padding(.horizontal, 12)
                .background(RoundedRectangle(cornerRadius: 10).fill(c.surfaceVariant))
        }
        .buttonStyle(.plain)
    }
}

private struct SeedBackupConfirmationButton: View {
    @Environment(\.dogTagColors) var c
    let onConfirmed: () -> Void

    var body: some View {
        Button {
            guard let seedHex = Wallet.seedHex(), SeedBackup.confirm(seedHex: seedHex) else { return }
            onConfirmed()
        } label: {
            Text("I've saved it")
                .font(.system(size: 12, weight: .semibold)).foregroundColor(c.onAccent)
                .padding(.vertical, 8).padding(.horizontal, 12)
                .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
        }
        .buttonStyle(.plain)
    }
}

/// Carries the revealed secrets into `ExportAccountSheet` via `.sheet(item:)`, so they are delivered
/// by the payload rather than read from sibling `@State` (which SwiftUI evaluates stale on the first
/// presentation). A fresh `id` per reveal re-presents the sheet each time Export is tapped.
private struct ExportPayload: Identifiable {
    let id = UUID()
    let mnemonic: String?
    let privateKeyHex: String?
}

/// Profile's single sheet route.
///
/// One `.sheet(item:)` rather than one modifier per destination on purpose: the legacy-wallet rescue
/// hands off straight from the export sheet to its confirmation, and swapping this item's identity
/// re-renders the presented sheet in place. Two sibling `.sheet` modifiers would have to dismiss one
/// and present the other across an animation, which drops the second presentation.
private enum ProfileSheet: Identifiable {
    case export(ExportPayload)
    case danger(DangerAction)

    var id: String {
        switch self {
        case let .export(payload): return "export-\(payload.id)"
        case let .danger(action): return "danger-\(action.id)"
        }
    }
}

/// The destructive actions Profile offers, each paired with the copy it must show BEFORE it runs.
///
/// Warning text and behaviour live in one type deliberately: these actions destroy material that no
/// backup can restore, so the description the user confirms against must not be able to drift from
/// what `AppReset` actually does.
private enum DangerAction: String, Identifiable {
    /// The legacy-wallet rescue: swap a wallet whose 24 words cannot be shown for a fresh, phrase-backed
    /// one. Not a "danger zone" chore but the only honest way out of a wallet that can never satisfy
    /// the seed-backup gate - and therefore can never issue a dog tag.
    case replaceWallet
    case deleteWallet
    case deleteDogTags
    case deleteRecords
    case resetEverything

    var id: String { rawValue }

    var title: String {
        switch self {
        case .replaceWallet: return "Replace wallet"
        case .deleteWallet: return "Delete wallet"
        case .deleteDogTags: return "Delete dog-tags"
        case .deleteRecords: return "Delete records"
        case .resetEverything: return "Reset everything"
        }
    }

    var detail: String {
        switch self {
        case .replaceWallet:
            return "Destroy this wallet and create a fresh one whose 24-word phrase you can actually write down."
        case .deleteWallet:
            return "Wipe the seed, the recovery entropy and the backup confirmation from this device."
        case .deleteDogTags:
            return "Wipe every dog tag held here - owner-secret, profile-tree salts, pet details and photo."
        case .deleteRecords:
            return "Wipe every credential imported on this device."
        case .resetEverything:
            return "Wallet, dog-tags and records - everything above, in one action."
        }
    }

    /// Typed verbatim to confirm. Distinct per action on purpose: one shared word would let muscle
    /// memory confirm a sheet the user did not mean to open.
    var confirmPhrase: String {
        switch self {
        case .replaceWallet: return "REPLACE WALLET"
        case .deleteWallet: return "DELETE WALLET"
        case .deleteDogTags: return "DELETE DOG TAGS"
        case .deleteRecords: return "DELETE RECORDS"
        case .resetEverything: return "RESET EVERYTHING"
        }
    }

    var biometricReason: String {
        switch self {
        case .replaceWallet: return "Authenticate to replace your wallet"
        case .deleteWallet: return "Authenticate to delete your wallet"
        case .deleteDogTags: return "Authenticate to delete your dog-tags"
        case .deleteRecords: return "Authenticate to delete your records"
        case .resetEverything: return "Authenticate to erase your local DogTag data"
        }
    }

    /// Bullet by bullet, what is destroyed. Phrased as consequences, not file names.
    var destroys: [String] {
        let ownerSecret = "The owner-secret behind every dog tag on this device. R is written ONCE "
            + "on-chain and every proof re-derives that secret from your seed, so those tags can NEVER "
            + "prove consent again. Re-importing a tag will not bring it back - a working tag means a "
            + "brand-new issuance at your vet."
        let salts = "The profile-tree salts for every dog tag here. They are not derivable from any "
            + "recovery phrase and exist nowhere else, so no backup can restore them."
        switch self {
        case .replaceWallet, .deleteWallet:
            return [
                "This wallet's seed and private key. Unless you exported the private key, the account "
                    + "becomes unreachable - DogTag cannot recover it and neither can anyone else.",
                ownerSecret,
            ]
        case .deleteDogTags:
            return [ownerSecret, salts, "The pet details and photo saved for each tag."]
        case .deleteRecords:
            return ["Every credential and record imported on this device."]
        case .resetEverything:
            return [
                "This wallet's seed and private key.",
                ownerSecret,
                salts,
                "Every pet, photo and imported credential held on this device.",
            ]
        }
    }

    /// The counterweight to `destroys`, so the warning is not read as broader than it is.
    var survives: String {
        switch self {
        case .deleteRecords:
            return "Your records stay on-chain, issued by your vet - scan a fresh vet QR to import them "
                + "again. Your wallet and dog-tags are untouched."
        default:
            return "Nothing on-chain is deleted. Your dog tag and its records stay on-chain, held by the "
                + "custodian. What is destroyed is this device's ability to prove you own them."
        }
    }

    /// The one thing `replaceWallet` ADDS, stated up front so it does not read as pure loss.
    var closing: String? {
        switch self {
        case .replaceWallet:
            return "You will then get a fresh 24-word recovery phrase to write down. Issuing a dog tag "
                + "stays blocked until you confirm you have saved it."
        default:
            return nil
        }
    }
}

/// The biometric-gated account-export sheet. Shows a hard security warning, the 24-word recovery
/// phrase in a numbered grid, and the raw secp256k1 private key, each with a copy action
/// (auto-expiring clipboard). The secrets are only ever held in SwiftUI state for this sheet; they
/// are never logged or transmitted.
private struct ExportAccountSheet: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let mnemonic: String?
    let privateKeyHex: String?
    /// Offered only for a LEGACY wallet, whose 24 words cannot be reconstructed here. Routes to the
    /// replace-wallet confirmation.
    let onReplaceWallet: () -> Void

    private var hasPhrase: Bool { !(mnemonic ?? "").isEmpty }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Export account").font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                    Spacer()
                    Button { dismiss() } label: {
                        Image(systemName: "xmark").foregroundColor(c.onBackground)
                            .frame(width: 32, height: 32).background(Circle().fill(c.surfaceVariant))
                    }.buttonStyle(.plain)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Label("Keep this private", systemImage: "exclamationmark.triangle.fill")
                        .font(.system(size: 13, weight: .bold)).foregroundColor(c.danger)
                    Text("Anyone with the recovery phrase or the private key below has FULL control of your wallet. Store them offline. Never share, screenshot, or type them into a website - DogTag will never ask for them.")
                        .font(.system(size: 12)).foregroundColor(c.onBackground)
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 14).fill(c.danger.opacity(0.12)))

                if let m = mnemonic, !m.isEmpty {
                    Text("Recovery phrase (24 words)").font(.system(size: 13, weight: .semibold)).foregroundColor(c.muted)
                    mnemonicGrid(m).hideWhenInactive()
                    HStack(spacing: 10) {
                        CopyButton(title: "Copy recovery phrase", value: m, secret: true)
                        SeedBackupConfirmationButton { dismiss() }
                    }
                    Text("Restores your account in DogTag (or a wallet using the same derivation). The copy clears from the clipboard automatically.")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                } else {
                    // The legacy dead end, stated honestly. There is deliberately NO "I've saved it"
                    // here: the app must never invite a user to confirm they backed up a phrase it
                    // cannot show them. The way out is the replace card at the foot of this sheet.
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Recovery phrase not available").font(.system(size: 13, weight: .semibold))
                            .foregroundColor(c.onBackground)
                        Text("This wallet was created before phrase export was supported, so its 24 words can't be reconstructed on this device.")
                            .font(.system(size: 13)).foregroundColor(c.onBackground)
                        Text("Because DogTag can't show you the phrase, it also can't ask you to confirm you've backed it up - and issuing a dog tag requires that confirmation, since the phrase is the only thing that could regenerate a tag's owner-secret on a replacement phone. Copy the private key below to keep this account, then replace this wallet with a fresh, phrase-backed one.")
                            .font(.system(size: 12)).foregroundColor(c.muted)
                    }
                }

                if let pk = privateKeyHex, !pk.isEmpty {
                    Divider().overlay(c.outline).padding(.vertical, 2)
                    Text("Private key (secp256k1)").font(.system(size: 13, weight: .semibold)).foregroundColor(c.muted)
                    Text(pk)
                        .font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 10).fill(c.surface))
                        .hideWhenInactive()
                    CopyButton(title: "Copy private key", value: pk, secret: true)
                    Text("Full control of your on-chain wallet. Import it into a compatible EVM wallet to restore this exact address (the recovery phrase alone would derive a different one).")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                }

                // Deliberately LAST, below the private key: a legacy wallet's key is the only thing
                // that survives a replace, so the export has to come first on the way down the sheet.
                if !hasPhrase {
                    Divider().overlay(c.outline).padding(.vertical, 2)
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Get unblocked: replace this wallet")
                            .font(.system(size: 14, weight: .bold)).foregroundColor(c.onBackground)
                        Text("Creates a fresh wallet with a 24-word phrase you can write down, and replaces this one. Copy the private key above first if you want to keep this account.")
                            .font(.system(size: 12)).foregroundColor(c.muted)
                        Button { onReplaceWallet() } label: {
                            Text("Replace wallet…").font(.system(size: 14, weight: .semibold))
                                .foregroundColor(c.onAccent)
                                .padding(.vertical, 10).padding(.horizontal, 14)
                                .background(RoundedRectangle(cornerRadius: 10).fill(c.danger))
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(14)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 14).fill(c.surfaceVariant))
                }
                Spacer(minLength: 12)
            }
            .padding(20)
        }
        .background(c.background.ignoresSafeArea())
    }

    private func mnemonicGrid(_ m: String) -> some View {
        let words = m.split(separator: " ").map(String.init)
        return LazyVGrid(
            columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())],
            spacing: 8
        ) {
            ForEach(Array(words.enumerated()), id: \.offset) { i, w in
                HStack(spacing: 4) {
                    Text("\(i + 1)").font(.system(size: 10, weight: .medium)).foregroundColor(c.muted)
                        .frame(width: 18, alignment: .trailing)
                    Text(w).font(.system(size: 13, design: .monospaced)).foregroundColor(c.onBackground)
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 6).padding(.horizontal, 8)
                .background(RoundedRectangle(cornerRadius: 8).fill(c.surface))
            }
        }
    }
}

/// The typed confirmation every destructive action passes through.
///
/// A typed phrase rather than a two-tap alert on purpose: these actions destroy material that no
/// backup restores, and the phrase forces the user past the warning text instead of through a button
/// their thumb already knows the position of. The action's own words are what must be typed, so
/// confirming the wrong sheet is not possible by muscle memory.
private struct DestructiveConfirmationSheet: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let action: DangerAction
    let onConfirmed: () -> Void

    @State private var typed = ""
    @FocusState private var fieldFocused: Bool

    private var phraseMatches: Bool {
        typed.trimmingCharacters(in: .whitespacesAndNewlines).uppercased() == action.confirmPhrase
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text(action.title).font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                    Spacer()
                    Button { dismiss() } label: {
                        Image(systemName: "xmark").foregroundColor(c.onBackground)
                            .frame(width: 32, height: 32).background(Circle().fill(c.surfaceVariant))
                    }.buttonStyle(.plain)
                }

                VStack(alignment: .leading, spacing: 10) {
                    Label("This cannot be undone", systemImage: "exclamationmark.triangle.fill")
                        .font(.system(size: 13, weight: .bold)).foregroundColor(c.danger)
                    ForEach(Array(action.destroys.enumerated()), id: \.offset) { _, line in
                        HStack(alignment: .top, spacing: 8) {
                            Text("•").font(.system(size: 12, weight: .bold)).foregroundColor(c.danger)
                            Text(line).font(.system(size: 12)).foregroundColor(c.onBackground)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 14).fill(c.danger.opacity(0.12)))

                VStack(alignment: .leading, spacing: 4) {
                    Text("What stays").font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
                    Text(action.survives).font(.system(size: 12)).foregroundColor(c.onBackground)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 14).fill(c.surfaceVariant))

                if let closing = action.closing {
                    Text(closing).font(.system(size: 12)).foregroundColor(c.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text("Type \(action.confirmPhrase) to confirm")
                        .font(.system(size: 12, weight: .semibold)).foregroundColor(c.onBackground)
                    TextField(action.confirmPhrase, text: $typed)
                        .font(.system(size: 15, design: .monospaced))
                        .foregroundColor(c.onBackground)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                        // The keyboard covers the confirm and Cancel buttons below, so Return has to
                        // get out of the way - otherwise typing the phrase leaves the user with no
                        // visible way to act on it (or to back out).
                        .focused($fieldFocused)
                        .submitLabel(.done)
                        .onSubmit { fieldFocused = false }
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 10).fill(c.surface))
                        .overlay(RoundedRectangle(cornerRadius: 10)
                            .stroke(phraseMatches ? c.danger : c.outline, lineWidth: 1.5))
                        // The sheet title, the danger-zone row and this button all carry the action's
                        // name, so UI tests need identifiers to address the gate unambiguously.
                        .accessibilityIdentifier("dangerConfirmField")
                }

                Button { onConfirmed() } label: {
                    Text(action.title).font(.system(size: 15, weight: .bold))
                        .foregroundColor(phraseMatches ? c.onAccent : c.muted)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 14)
                        .background(RoundedRectangle(cornerRadius: 12)
                            .fill(phraseMatches ? c.danger : c.surfaceVariant))
                }
                .buttonStyle(.plain)
                .disabled(!phraseMatches)
                .accessibilityIdentifier("dangerConfirmButton")

                Button { dismiss() } label: {
                    Text("Cancel").font(.system(size: 14, weight: .semibold)).foregroundColor(c.accent)
                        .frame(maxWidth: .infinity).padding(.vertical, 10)
                }
                .buttonStyle(.plain)

                Spacer(minLength: 12)
            }
            .padding(20)
        }
        // Dragging the sheet content also puts the keyboard away, so the confirm/Cancel buttons are
        // reachable without hunting for the Return key.
        .scrollDismissesKeyboard(.interactively)
        .background(c.background.ignoresSafeArea())
    }
}

/// Covers secret content with an opaque, un-animated overlay whenever the scene leaves `.active`, so
/// the plaintext never lands in the app-switcher snapshot iOS writes to disk on backgrounding. The
/// cover must appear instantly (no fade) or the snapshot could catch a mid-transition frame.
private struct HideWhenInactive: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.dogTagColors) private var c

    func body(content: Content) -> some View {
        let hidden = scenePhase != .active
        return content
            .blur(radius: hidden ? 18 : 0)
            .overlay {
                if hidden {
                    RoundedRectangle(cornerRadius: 10)
                        .fill(c.surfaceVariant)
                        .overlay(
                            Label("Hidden", systemImage: "eye.slash.fill")
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundColor(c.muted)
                        )
                }
            }
            .animation(nil, value: scenePhase)
    }
}

private extension View {
    func hideWhenInactive() -> some View { modifier(HideWhenInactive()) }
}

/// Clipboard helper for sensitive values: copies with a short expiry so the secret does not linger
/// on the pasteboard (and stays off the OS clipboard history for other apps for long).
enum SecureClipboard {
    static func copySecret(_ value: String, ttl: TimeInterval = 90) {
        let item: [String: Any] = [UTType.utf8PlainText.identifier: value]
        UIPasteboard.general.setItems([item], options: [
            .expirationDate: Date().addingTimeInterval(ttl),
            .localOnly: true,
        ])
    }
}
