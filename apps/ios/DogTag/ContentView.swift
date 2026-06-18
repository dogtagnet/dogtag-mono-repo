import SwiftUI

/// Result of running the verify-core over the bundled testvectors.json through the Rust FFI.
struct ParityResult {
    let pass: Bool
    let leafChecks: Int
    let merkleChecks: Int
    let detail: String
}

/// Drive the UniFFI-generated Rust SDK surface (hashLeafHex / buildMerkleRootHex) over the SAME
/// shared vectors the server/TS assert, and compare mobile output to server-side expected hex.
func runParity() -> ParityResult {
    guard let url = Bundle.main.url(forResource: "testvectors", withExtension: "json"),
          let data = try? Data(contentsOf: url),
          let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
        return ParityResult(pass: false, leafChecks: 0, merkleChecks: 0, detail: "could not load testvectors.json")
    }

    do {
        // 1) Leaf parity: every leaf hashed on-device must equal the server expected_hex.
        var leafChecks = 0
        if let leaves = root["leaves"] as? [[String: Any]] {
            for v in leaves {
                let keyPath = v["keyPath"] as? String ?? ""
                let saltHex = v["saltHex"] as? String ?? ""
                let tag = UInt8((v["tag"] as? Int) ?? 0)
                let value = (v["value"] as? String) ?? ""  // null -> ""
                let expected = v["expected_hex"] as? String ?? ""
                let got = try hashLeafHex(keyPath: keyPath, saltHex: saltHex, tag: tag, value: value)
                if got != expected {
                    return ParityResult(pass: false, leafChecks: leafChecks, merkleChecks: 0,
                                        detail: "leaf mismatch '\(v["name"] ?? "?")': \(got) != \(expected)")
                }
                leafChecks += 1
            }
        }

        // 2) Merkle-root parity: every tree built on-device must equal the server root_hex.
        var merkleChecks = 0
        if let merkle = root["merkle"] as? [[String: Any]] {
            for m in merkle {
                let leafHexes = (m["leaf_hexes"] as? [String]) ?? []
                let expected = m["root_hex"] as? String ?? ""
                let got = try buildMerkleRootHex(leafHexes: leafHexes)
                if got != expected {
                    return ParityResult(pass: false, leafChecks: leafChecks, merkleChecks: merkleChecks,
                                        detail: "root mismatch '\(m["name"] ?? "?")': \(got) != \(expected)")
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

struct ContentView: View {
    private let result = runParity()
    @State private var scanMsg = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("DogTag — Phase 6 verify core")
                .font(.title).bold()
            Text("Rust SDK via UniFFI (xcframework + Swift binding)")
                .font(.footnote)

            Text("mobile root == server root: \(result.pass ? "PASS" : "FAIL")")
                .font(.title2).bold()
                .foregroundColor(result.pass ? .green : .red)

            Text("leaves checked: \(result.leafChecks)    merkle trees checked: \(result.merkleChecks)")
                .font(.subheadline)
            Text(result.detail)
                .font(.system(.caption, design: .monospaced))

            Button("Scan QR") {
                scanMsg = "Scan QR — not yet implemented (Phase 6 later pass)"
            }
            .buttonStyle(.borderedProminent)
            if !scanMsg.isEmpty { Text(scanMsg).font(.caption) }

            Spacer()
        }
        .padding(24)
    }
}
