import SwiftUI

#if DEBUG
/// Debug-only on-device test of the frozen owner-hidden consent circuit and proving artifacts.
struct ZkSelfTestCard: View {
    @Environment(\.dogTagColors) var c

    @State private var running = false
    @State private var status = ""
    @State private var result: ZkSelfTestResult? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionTitle(text: "Developer · ZK self-test")
            VStack(alignment: .leading, spacing: 8) {
                Text("Runs the real owner-hidden consent prover over the frozen cross-language vector and checks all seven public signals.")
                    .font(.system(size: 12)).foregroundColor(c.muted)

                Button(action: run) {
                    Text(running ? "Running…" : "Run ZK self-test")
                        .frame(maxWidth: .infinity).padding(.vertical, 12)
                        .foregroundColor(c.onAccent)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.accent))
                }
                .disabled(running)
                .accessibilityIdentifier("zk_selftest_run")

                let headline: String = {
                    if running { return "ZK-SELFTEST: RUNNING" }
                    guard let result else { return "ZK-SELFTEST: IDLE" }
                    return result.pass ? "ZK-SELFTEST: PASS" : "ZK-SELFTEST: FAIL"
                }()
                Text(headline)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundColor((result == nil || running) ? c.muted : (result!.pass ? c.success : c.danger))
                    .accessibilityIdentifier("zk_selftest_result")

                let detail = result?.detail ?? status
                if !detail.isEmpty {
                    Text(detail)
                        .font(.system(size: 11, design: .monospaced)).foregroundColor(c.muted)
                        .accessibilityIdentifier("zk_selftest_detail")
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
        }
    }

    private func run() {
        running = true
        result = nil
        status = "Starting…"
        DispatchQueue.global(qos: .userInitiated).async {
            let testResult = runZkSelfTest { update in
                DispatchQueue.main.async { status = update }
            }
            DispatchQueue.main.async {
                result = testResult
                running = false
            }
        }
    }
}

struct ZkSelfTestResult {
    let pass: Bool
    let detail: String
}

/// Exact fixture from `consent_prove_parity.rs`; do not replace it with a locally invented vector.
private func runZkSelfTest(_ onStatus: @escaping (String) -> Void) -> ZkSelfTestResult {
    let seedHex = "0x636f6e73656e74207061726974792077616c6c65742073656564202d2054455354204d4154455249414c204f4e4c592c206e6576657220686f6c642076616c7565"
    let dogTagId = "424242"
    let ownerAddress = "0x00000000000000000000000000000000deadbeef"
    let purpose = "0x" + String(repeating: "0", count: 63) + "7"
    let relayer = "0x" + String(repeating: "11", count: 20)
    let recordType = "0x" + String(repeating: "0", count: 62) + "13"
    let nonce = "0x" + String(repeating: "0", count: 62) + "63"
    let deadline = "1893456000"
    let expected = [
        "19282080935305080861096842252900215298393603684181619512414474199363734335896",
        "7",
        "97433442488726861213578988847752201310395502865",
        "10082827006016799336744122064490401655461844512429394449644692943092461376845",
        "16272705119130550328684133240225299777651023887772779485808543219167759832756",
        "19",
        "1893456000",
    ]
    let attributes: [[String: Any]] = [
        [
            "keyPath": "credentialSubject.name",
            "salt": "0x07070707070707070707070707070707",
            "tag": 2,
            "value": "Rex",
        ],
        [
            "keyPath": "credentialSubject.breedLabel",
            "salt": "0x09090909090909090909090909090909",
            "tag": 2,
            "value": "Shiba Inu",
        ],
        [
            "keyPath": "owner.identity.fullName",
            "salt": "0x15151515151515151515151515151515",
            "tag": 2,
            "value": "Alice Owner",
        ],
        [
            "keyPath": "owner.identity.country",
            "salt": "0x16161616161616161616161616161616",
            "tag": 2,
            "value": "GB",
        ],
        [
            "keyPath": "owner.identity.docNumber",
            "salt": "0x17171717171717171717171717171717",
            "tag": 2,
            "value": "PASSPORT-123",
        ],
    ]

    do {
        guard let attributesData = try? JSONSerialization.data(withJSONObject: attributes),
              let attributesJson = String(data: attributesData, encoding: .utf8) else {
            return ZkSelfTestResult(pass: false, detail: "could not encode consent attributes")
        }
        onStatus("Resolving consent artifacts…")
        guard let zkeyPath = ZkeyAsset.ensure() else {
            return ZkSelfTestResult(pass: false, detail: "consent_final.zkey not bundled")
        }
        guard let graphPath = ZkeyAsset.ensureGraph() else {
            return ZkSelfTestResult(pass: false, detail: "consent.graph not bundled")
        }

        onStatus("Generating owner-hidden consent proof…")
        let proof = try proveConsent(
            seedHex: seedHex,
            dogTagIdHandle: dogTagId,
            ownerAddressHex: ownerAddress,
            attributesJson: attributesJson,
            purposeHex: purpose,
            relayerHex: relayer,
            recordTypeHex: recordType,
            consentNonceHex: nonce,
            deadlineDec: deadline,
            zkeyPath: zkeyPath,
            graphPath: graphPath)

        guard proof.pubSignals.count == PublicSignalIndex.count else {
            return ZkSelfTestResult(
                pass: false,
                detail: "expected \(PublicSignalIndex.count) public signals, got \(proof.pubSignals.count)")
        }
        guard proof.pubSignals == expected else {
            let index = proof.pubSignals.indices.first {
                proof.pubSignals[$0] != expected[$0]
            } ?? -1
            return ZkSelfTestResult(pass: false, detail: "public-signal mismatch at index \(index)")
        }
        guard proof.pubSignals[PublicSignalIndex.ownerHidden.nullifier] != "0",
              proof.pubSignals[PublicSignalIndex.ownerHidden.root] != "0" else {
            return ZkSelfTestResult(pass: false, detail: "nullifier or profile root is zero")
        }
        guard !proof.a.isEmpty, !proof.b.isEmpty, !proof.c.isEmpty else {
            return ZkSelfTestResult(pass: false, detail: "proof coordinates are empty")
        }
        return ZkSelfTestResult(
            pass: true,
            detail: "7/7 consent signals match · nullifier and profile root non-zero · prover=on-device")
    } catch {
        return ZkSelfTestResult(pass: false, detail: "exception: \(error)")
    }
}
#endif
