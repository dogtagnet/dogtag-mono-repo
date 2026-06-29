import SwiftUI

#if DEBUG
/// Debug-only ON-DEVICE ZK self-test — the iOS mirror of Android `ui/screens/ZkSelfTest.kt`, and the
/// mobile end-to-end check the audit could previously only read (no device in the lab). It drives the
/// SAME native code path the privacy-preserving groomer export uses, end to end, with no camera /
/// biometric / network: a fixed *imported record* (a deterministic WrappedDoc) is signed, proved, and
/// the proof is checked against the server-recomputed public signals — all on the device's own arm64
/// native code (UniFFI → Rust SDK + circom-prover graph witness calculator + bundled zkey).
///
/// The fixed vector (`zk_selftest.json`) is produced by, and byte-for-byte mirrors,
/// `crates/dogtag-standard-rs/tests/prove_parity.rs::fixed_prove_inputs` (regenerate via its
/// `dump_selftest_fixture` test — it writes the identical Android + iOS fixtures), so the device
/// proof MUST reproduce the same 7 public signals the server SDK computes — and the on-chain
/// `Groth16Verifier` was generated from the same vkey.
///
/// Steps (each a real native call):
///   1. `signConsentEddsa`     — EdDSA-BabyJubjub consent signature (consent signing). The circuit
///                               re-verifies this signature as a constraint inside step 2's proof.
///   2. `proveVerification`    — generate the Groth16 proof ON-DEVICE (graph witnesscalc + zkey).
///   3. public-signal check    — proof's `pubSignals` == the server-recomputed expected vector, and
///                               the 32-bit-regression guard (nullifier & keyHash non-zero).
///   4. `keyHashHex` +
///      `bindConsentKeyDigestHex` — derive the consent keyHash and the EIP-712 consent-key bind
///                               digest (consent-key bind).
///
/// The result line renders the stable text `ZK-SELFTEST: PASS` / `ZK-SELFTEST: FAIL` that the Maestro
/// flow (`apps/ios/maestro/zk_e2e.yaml`) asserts on. Wrapped in `#if DEBUG` so it never ships.
struct ZkSelfTestCard: View {
    @Environment(\.dogTagColors) var c

    @State private var running = false
    @State private var status = ""
    @State private var result: ZkSelfTestResult? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionTitle(text: "Developer · ZK self-test")
            VStack(alignment: .leading, spacing: 8) {
                Text("Runs the REAL on-device Groth16 prover (UniFFI → Rust circom-prover, graph witness calculator + bundled proving key) over a fixed imported-record vector, then checks the proof's public signals match the server-recomputed values. Debug builds only.")
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
                    guard let r = result else { return "ZK-SELFTEST: IDLE" }
                    return r.pass ? "ZK-SELFTEST: PASS" : "ZK-SELFTEST: FAIL"
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
            let r = runZkSelfTest { s in DispatchQueue.main.async { status = s } }
            DispatchQueue.main.async {
                result = r
                running = false
            }
        }
    }
}

struct ZkSelfTestResult {
    let pass: Bool
    let detail: String
}

/// Execute the on-device ZK self-test off the main thread. Returns PASS only if every native step
/// succeeds AND the proof's public signals equal the server-recomputed expected vector.
private func runZkSelfTest(_ onStatus: @escaping (String) -> Void) -> ZkSelfTestResult {
    do {
        guard let url = Bundle.main.url(forResource: "zk_selftest", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let fixture = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return ZkSelfTestResult(pass: false, detail: "could not load zk_selftest.json")
        }
        guard let wrappedDocJson = fixture["wrappedDocJson"] as? String,
              let consentJson = fixture["consentJson"] as? String,
              let prvHex = fixture["consentPrvHex"] as? String,
              let axHex = fixture["consentAxHex"] as? String,
              let ayHex = fixture["consentAyHex"] as? String,
              let expected = fixture["expectedPubSignals"] as? [String],
              let consent = (try? JSONSerialization.jsonObject(with: Data(consentJson.utf8))) as? [String: String] else {
            return ZkSelfTestResult(pass: false, detail: "malformed zk_selftest.json")
        }
        func field(_ k: String) -> String { consent[k] ?? "" }

        // 1. EdDSA-BabyJubjub consent signature (real native signing). The circuit verifies this
        //    signature as a constraint inside the proof below, so a proof whose public signals match
        //    the expected vector is itself proof that the signature was valid — no separate verify.
        onStatus("Signing EdDSA consent…")
        let sig = try signConsentEddsa(
            prvHex: prvHex,
            dogTagIdHex: field("dogTagId"), recordTypeHex: field("recordType"),
            purposeHex: field("purpose"), credentialRootHex: field("credentialRoot"),
            challengeHex: field("challenge"), relayerHex: field("relayer"),
            subjectHex: field("subject"), nonceHex: field("nonce"), deadlineHex: field("deadline"))

        // 2. Generate the Groth16 proof ON-DEVICE (graph witness calculator + bundled zkey).
        onStatus("Materialising zkey + graph…")
        guard let zkeyPath = ZkeyAsset.ensure() else {
            return ZkSelfTestResult(pass: false, detail: "verification_final.zkey not bundled")
        }
        guard let graphPath = ZkeyAsset.ensureGraph() else {
            return ZkSelfTestResult(pass: false, detail: "verification.graph not bundled")
        }
        onStatus("Generating Groth16 proof on-device…")
        let eddsaInput = EddsaSigInput(r8xDec: sig.r8xDec, r8yDec: sig.r8yDec, sDec: sig.sDec,
                                       axHex: axHex, ayHex: ayHex)
        let proof = try proveVerification(wrappedDocJson: wrappedDocJson, consentJson: consentJson,
                                          eddsaSig: eddsaInput, zkeyPath: zkeyPath, graphPath: graphPath)

        // 3. The proof's public signals MUST equal the server-recomputed expected vector.
        if proof.pubSignals.count != 7 {
            return ZkSelfTestResult(pass: false, detail: "expected 7 public signals, got \(proof.pubSignals.count)")
        }
        if proof.pubSignals != expected {
            let firstBad = (0..<proof.pubSignals.count).first { proof.pubSignals[$0] != expected[$0] } ?? -1
            return ZkSelfTestResult(pass: false, detail: "public-signal mismatch at index \(firstBad)")
        }
        // 32-bit witness regression guard (wasm2c zeroed the last-computed output wires).
        if proof.pubSignals[4] == "0" { return ZkSelfTestResult(pass: false, detail: "nullifier (pub[4]) is zero") }
        if proof.pubSignals[5] == "0" { return ZkSelfTestResult(pass: false, detail: "keyHash (pub[5]) is zero") }

        // 4. Consent-key bind: derive the keyHash and the EIP-712 bind digest (real native calls).
        onStatus("Deriving consent-key bind digest…")
        let keyHash = try keyHashHex(axHex: axHex, ayHex: ayHex)
        let roax = RoaxConfig.load()
        let bindDigest = try bindConsentKeyDigestHex(
            consentKeyRegistryAddr: roax.consentKeyRegistry, keyHashHex: keyHash,
            walletAddr: field("subject"), nonce: 0, chainId: UInt64(roax.chainId))
        if !bindDigest.hasPrefix("0x") || bindDigest.count != 66 {
            return ZkSelfTestResult(pass: false, detail: "bad consent-key bind digest: \(bindDigest)")
        }

        return ZkSelfTestResult(
            pass: true,
            detail: "7/7 public signals match · nullifier+keyHash non-zero · bind digest \(bindDigest.prefix(12))… · prover=on-device(arm64)")
    } catch {
        return ZkSelfTestResult(pass: false, detail: "exception: \(error)")
    }
}
#endif
