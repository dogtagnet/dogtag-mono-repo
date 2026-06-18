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

/// The Verify tab. The user app SCANS — it never displays a QR. The primary action ("Scan a QR") opens
/// the unified ScanScreen, which routes to import-a-record or present-a-record by the QR shape. The
/// trust-core parity panel below is a real diagnostic (mobile Merkle root == server root).
struct VerifyScreen: View {
    @Environment(\.dogTagColors) var c
    let onScan: () -> Void
    private let parity = runParity()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Verify").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)

                Text("Scan a vet or groomer's QR to import a verified record, or to present one of your stored records for an on-chain proof-of-verification. Your app only scans — it never shows a QR.")
                    .font(.system(size: 13)).foregroundColor(c.muted)

                Button(action: onScan) {
                    Text("Scan a QR").frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(c.onAccent).background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }

                // Verify-core parity panel (real diagnostic).
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

                Spacer(minLength: 40)
            }
            .padding(20)
        }
    }
}
