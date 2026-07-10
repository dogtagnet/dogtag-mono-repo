import SwiftUI
import UniformTypeIdentifiers

struct ProfileScreen: View {
    @Environment(\.dogTagColors) var c
    @EnvironmentObject var theme: ThemeManager
    @ObservedObject private var store = LocalStore.shared
    private let roax = RoaxConfig.load()

    @State private var walletExists = Wallet.exists()
    @State private var ethAddr: String? = nil
    @State private var consentAx: String? = nil
    @State private var consentKeyHash: String? = nil
    @State private var mnemonic: String? = nil
    @State private var walletMsg = ""
    @State private var showExport = false
    @State private var exportMnemonic: String? = nil
    @State private var exportPrivKey: String? = nil

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
                    Text("A self-custodial key: BIP-39 seed → secp256k1 wallet + a distinct BabyJubjub consent key (derived in Rust). The seed is stored in the iOS Keychain (hardware-protected, this-device-only); reveal is biometric-gated.")
                        .font(.system(size: 12)).foregroundColor(c.muted)

                    if !walletExists {
                        walletButton("Create embedded wallet") {
                            Biometric.authenticate(reason: "Authenticate to generate your keys") { ok, e in
                                guard ok else { walletMsg = e ?? "auth failed"; return }
                                do {
                                    let id = try Wallet.create()
                                    walletExists = true
                                    ethAddr = id.ethAddress
                                    consentAx = id.consent.axHex
                                    consentKeyHash = id.consent.keyHashHex
                                    mnemonic = id.mnemonic
                                    walletMsg = "Wallet created. Back up your recovery phrase now."
                                } catch { walletMsg = "create failed: \(error)" }
                            }
                        }
                    } else {
                        walletButton("Unlock & show keys") {
                            Biometric.authenticate(reason: "Authenticate to reveal your keys") { ok, e in
                                guard ok else { walletMsg = e ?? "auth failed"; return }
                                do {
                                    let id = try Wallet.load()
                                    ethAddr = id?.ethAddress
                                    consentAx = id?.consent.axHex
                                    consentKeyHash = id?.consent.keyHashHex
                                    walletMsg = "Unlocked."
                                } catch { walletMsg = "unlock failed: \(error)" }
                            }
                        }
                        // Self-custody export: reveal + copy the recovery phrase AND the raw secp256k1
                        // private key (biometric-gated, same Face ID gate as the reveal above; the
                        // secrets are never logged or transmitted).
                        walletButtonSecondary("Export account keys") {
                            Biometric.authenticate(reason: "Authenticate to export your account keys") { ok, e in
                                guard ok else { walletMsg = e ?? "auth failed"; return }
                                exportMnemonic = Wallet.revealMnemonic()
                                exportPrivKey = Wallet.revealPrivateKeyHex()
                                showExport = true
                            }
                        }
                    }

                    // Tap any value to copy it (full value, not the truncated preview).
                    if let a = ethAddr { CopyRow(label: "Wallet", value: a) }
                    if let ax = consentAx { CopyRow(label: "Consent Ax", value: ax) }
                    if let kh = consentKeyHash {
                        CopyRow(label: "keyHash", value: kh)
                        Text("Bind on-chain: ConsentKeyRegistry.bindConsentKey(keyHash) @ \(String(roax.consentKeyRegistry.prefix(10)))…")
                            .font(.system(size: 11)).foregroundColor(c.muted)
                    }
                    if let m = mnemonic {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Recovery phrase (24 words)").font(.system(size: 12, weight: .bold)).foregroundColor(c.danger)
                            Text("Write these down and store them offline. Anyone with them controls your wallet.")
                                .font(.system(size: 11)).foregroundColor(c.muted)
                            Text(m).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
                                .textSelection(.enabled)
                            HStack(spacing: 10) {
                                CopyButton(title: "Copy phrase", value: m, secret: true)
                                Button {
                                    mnemonic = nil
                                    walletMsg = "Recovery phrase hidden. Export it again anytime from “Export account keys”."
                                } label: {
                                    Text("I've saved it")
                                        .font(.system(size: 12, weight: .semibold)).foregroundColor(c.onAccent)
                                        .padding(.vertical, 8).padding(.horizontal, 12)
                                        .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
                                }.buttonStyle(.plain)
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

                // ---- Dog-tags: dog tags issued to this wallet (scan the vet's /p/<token> QR to issue one) ----
                SectionTitle(text: "Dog-tags")
                VStack(alignment: .leading, spacing: 6) {
                    let minted = store.pets.filter { !$0.dogTagId.isEmpty && $0.dogTagId.allSatisfy { $0.isNumber } }
                    if minted.isEmpty {
                        Text("No dog tag yet. Scan your vet's dog-tag QR (Scan) to have one issued and bound to this wallet — the dogTagId then appears here.")
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
                    kv("VerificationRegistry", String(roax.verificationRegistry.prefix(16)) + "…")
                    kv("ConsentKeyRegistry", String(roax.consentKeyRegistry.prefix(16)) + "…")
                    kv("IssuerRegistry", String(roax.issuerRegistry.prefix(16)) + "…")
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .sheet(isPresented: $showExport) {
            ExportAccountSheet(mnemonic: exportMnemonic, privateKeyHex: exportPrivKey)
                .environment(\.dogTagColors, c)
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
/// the whole value. Used for the wallet address, consent Ax, keyHash, and dog-tag ids.
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

/// The biometric-gated account-export sheet. Shows a hard security warning, the 24-word recovery
/// phrase in a numbered grid, and the raw secp256k1 private key, each with a copy action
/// (auto-expiring clipboard). The secrets are only ever held in SwiftUI state for this sheet; they
/// are never logged or transmitted.
private struct ExportAccountSheet: View {
    @Environment(\.dogTagColors) var c
    @Environment(\.dismiss) var dismiss
    let mnemonic: String?
    let privateKeyHex: String?

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
                    mnemonicGrid(m)
                    CopyButton(title: "Copy recovery phrase", value: m, secret: true)
                    Text("Restores your account in DogTag (or a wallet using the same derivation). The copy clears from the clipboard automatically.")
                        .font(.system(size: 11)).foregroundColor(c.muted)
                } else {
                    Text("This wallet's recovery phrase isn't available on this device. It was created before phrase export was supported, so the 24 words can't be reconstructed here.")
                        .font(.system(size: 13)).foregroundColor(c.onBackground)
                }

                if let pk = privateKeyHex, !pk.isEmpty {
                    Divider().overlay(c.outline).padding(.vertical, 2)
                    Text("Private key (secp256k1)").font(.system(size: 13, weight: .semibold)).foregroundColor(c.muted)
                    Text(pk)
                        .font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 10).fill(c.surface))
                    CopyButton(title: "Copy private key", value: pk, secret: true)
                    Text("Full control of your on-chain wallet. Import it into a compatible EVM wallet to restore this exact address (the recovery phrase alone would derive a different one).")
                        .font(.system(size: 11)).foregroundColor(c.muted)
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
        .textSelection(.enabled)
    }
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
