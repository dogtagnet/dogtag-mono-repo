import SwiftUI

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

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Profile").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

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
                    }

                    if let a = ethAddr { kv("Wallet", a) }
                    if let ax = consentAx { kv("Consent Ax", String(ax.prefix(22)) + "…") }
                    if let kh = consentKeyHash {
                        kv("keyHash", String(kh.prefix(22)) + "…")
                        Text("Bind on-chain: ConsentKeyRegistry.bindConsentKey(keyHash) @ \(String(roax.consentKeyRegistry.prefix(10)))…")
                            .font(.system(size: 11)).foregroundColor(c.muted)
                    }
                    if let m = mnemonic {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Recovery phrase (24 words)").font(.system(size: 12, weight: .bold)).foregroundColor(c.danger)
                            Text(m).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
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
                            kv(pet.name.isEmpty ? "Pet" : pet.name, "dogTagId \(pet.dogTagId)")
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
    }

    private func walletButton(_ title: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).padding(.vertical, 10).padding(.horizontal, 14)
                .foregroundColor(c.onAccent)
                .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
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
