import SwiftUI

/// Result of running the verify-core over the bundled testvectors.json through the Rust FFI.
struct ParityResult {
    let pass: Bool
    let leafChecks: Int
    let merkleChecks: Int
    let detail: String
}

/// Drive the UniFFI Rust SDK surface over the shared vectors; mobile output must == server hex.
func runParity() -> ParityResult {
    guard let url = Bundle.main.url(forResource: "testvectors", withExtension: "json"),
          let data = try? Data(contentsOf: url),
          let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
        return ParityResult(pass: false, leafChecks: 0, merkleChecks: 0, detail: "could not load testvectors.json")
    }
    do {
        var leafChecks = 0
        if let leaves = root["leaves"] as? [[String: Any]] {
            for v in leaves {
                let keyPath = v["keyPath"] as? String ?? ""
                let saltHex = v["saltHex"] as? String ?? ""
                let tag = UInt8((v["tag"] as? Int) ?? 0)
                let value = (v["value"] as? String) ?? ""
                let expected = v["expected_hex"] as? String ?? ""
                let got = try hashLeafHex(keyPath: keyPath, saltHex: saltHex, tag: tag, value: value)
                if got != expected {
                    return ParityResult(pass: false, leafChecks: leafChecks, merkleChecks: 0,
                                        detail: "leaf mismatch '\(v["name"] ?? "?")'")
                }
                leafChecks += 1
            }
        }
        var merkleChecks = 0
        if let merkle = root["merkle"] as? [[String: Any]] {
            for m in merkle {
                let leafHexes = (m["leaf_hexes"] as? [String]) ?? []
                let expected = m["root_hex"] as? String ?? ""
                let got = try buildMerkleRootHex(leafHexes: leafHexes)
                if got != expected {
                    return ParityResult(pass: false, leafChecks: leafChecks, merkleChecks: merkleChecks,
                                        detail: "root mismatch '\(m["name"] ?? "?")'")
                }
                merkleChecks += 1
            }
        }
        return ParityResult(pass: true, leafChecks: leafChecks, merkleChecks: merkleChecks,
                            detail: "all leaves + roots match the server vectors")
    } catch {
        return ParityResult(pass: false, leafChecks: 0, merkleChecks: 0, detail: "FFI error: \(error)")
    }
}

struct VerifyScreen: View {
    @Environment(\.dogTagColors) var c
    private let parity = runParity()

    @State private var scanning = false
    @State private var request: VerificationRequest? = nil
    @State private var signed: SignedConsent? = nil
    @State private var status = ""

    var body: some View {
        if scanning {
            ZStack(alignment: .topLeading) {
                QRScannerView { raw in
                    scanning = false
                    if let req = VerificationRequest.parse(raw) {
                        request = req
                    } else {
                        status = "Not a verifier QR"
                    }
                }
                .ignoresSafeArea()
                Button("Cancel") { scanning = false }
                    .padding()
                    .foregroundColor(.white)
            }
        } else {
            content
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Verify").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

                // Verify-core parity panel.
                VStack(alignment: .leading, spacing: 6) {
                    Text("Trust-core parity").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
                    Text("Rust SDK via UniFFI (xcframework + Swift binding)").font(.system(size: 12)).foregroundColor(c.muted)
                    Text("mobile root == server root: \(parity.pass ? "PASS" : "FAIL")")
                        .font(.system(size: 16, weight: .bold))
                        .foregroundColor(parity.pass ? c.success : c.danger)
                    Text("leaves: \(parity.leafChecks)   merkle trees: \(parity.merkleChecks)")
                        .font(.system(size: 12)).foregroundColor(c.muted)
                    Text(parity.detail).font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))

                Button {
                    status = ""; signed = nil; request = nil; scanning = true
                } label: {
                    Text("Scan verifier QR")
                        .frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(c.onAccent)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }

                if let req = request {
                    ConsentReview(req: req) { sc in
                        signed = sc; status = "Signed locally — ready to submit."
                    }
                }

                if let sc = signed {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Signed consent (\(sc.mode.rawValue))").font(.system(size: 14, weight: .bold)).foregroundColor(c.onBackground)
                        Text("nullifier: \(String(sc.nullifier.prefix(18)))…").font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                        if let e = sc.eddsa {
                            Text("EdDSA S: \(String(e.sDec.prefix(20)))…").font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                        }
                        Button {
                            submit(sc)
                        } label: {
                            Text("Submit to /v1/verify/consent")
                                .padding(.vertical, 10).padding(.horizontal, 14)
                                .foregroundColor(c.onAccent)
                                .background(RoundedRectangle(cornerRadius: 10).fill(c.accent))
                        }
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 16).fill(c.surfaceVariant))
                }

                if !status.isEmpty { Text(status).font(.system(size: 12)).foregroundColor(c.muted) }
                Spacer(minLength: 40)
            }
            .padding(20)
        }
    }

    private func submit(_ sc: SignedConsent) {
        guard let urlStr = request?.callbackUrl, let url = URL(string: urlStr) else {
            status = "No callback URL in request; consent built but not submitted."
            return
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = sc.payloadJson.data(using: .utf8)
        URLSession.shared.dataTask(with: req) { _, resp, err in
            DispatchQueue.main.async {
                if let e = err { status = "POST failed: \(e.localizedDescription)" }
                else if let h = resp as? HTTPURLResponse { status = "POST \(urlStr) → \(h.statusCode)" }
            }
        }.resume()
    }
}

private struct ConsentReview: View {
    @Environment(\.dogTagColors) var c
    let req: VerificationRequest
    let onSign: (SignedConsent) -> Void
    @State private var err = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Verification request").font(.system(size: 15, weight: .bold)).foregroundColor(c.onBackground)
            field("Verifier", req.verifierName)
            field("Mode", req.mode == .zk ? "Zero-knowledge (EdDSA-BabyJubjub)" : "ECDSA (EIP-712)")
            field("DogTag", req.dogTagId)
            field("Purpose", String(req.purpose.prefix(18)) + "…")
            field("Relayer", req.relayer)
            Button {
                Biometric.authenticate(reason: "Sign a \(req.mode == .zk ? "ZK" : "standard") verification consent") { ok, e in
                    guard ok else { err = e ?? "auth failed"; return }
                    do {
                        let consentPriv = (req.mode == .zk) ? (try? Wallet.load())??.consent.prvHex : nil
                        onSign(try ConsentSigner.sign(req, consentPrivHex: consentPriv))
                    } catch { self.err = "sign failed: \(error)" }
                }
            } label: {
                Text("Approve & sign")
                    .frame(maxWidth: .infinity).padding(.vertical, 12)
                    .foregroundColor(.white)
                    .background(RoundedRectangle(cornerRadius: 12).fill(c.success))
            }
            if !err.isEmpty { Text(err).font(.system(size: 12)).foregroundColor(c.danger) }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }

    private func field(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label).font(.system(size: 12)).foregroundColor(c.muted).frame(width: 90, alignment: .leading)
            Text(value).font(.system(size: 12, design: .monospaced)).foregroundColor(c.onBackground)
        }
    }
}
