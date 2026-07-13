import Foundation

/// DogTag Selective-Disclosure Protocol — Merkle inclusion-proof verifier (DSDP plan §2.3).
///
/// The Swift leg of the reference verifier. It owns the verification ALGORITHM (recompute the leaf,
/// walk the root-ward `Sibling | Promote` steps, compare to the anchored root); the Poseidon
/// parameter set stays pinned in the Rust core, reached through the UniFFI primitives
/// `hashLeafHex` (Poseidon5 under DS_LEAF) and `hashNodeHex` (commutative Poseidon3 under DS_NODE).

/// One root-ward step of a `Sibling | Promote` inclusion proof (plan §2.3, wire shape §4.4).
enum InclusionStep: Equatable {
    /// The node was paired with `sibling` at this level (folded via the commutative node hash).
    case sibling(String) // 0x.. 32-byte node hex
    /// The node was the lone odd node at this level and passed through unchanged (carries no hash).
    case promote
}

/// Parse the JSON step array (`[{"sibling":"0x…"}, {"promote":true}, …]`) into `[InclusionStep]`.
func inclusionSteps(fromJson raw: [[String: Any]]) -> [InclusionStep] {
    raw.map { step in
        if (step["promote"] as? Bool) == true { return .promote }
        return .sibling((step["sibling"] as? String) ?? "")
    }
}

/// The FFI encoding of the steps for `verifyInclusionProofHex`: `"promote"` or the 0x.. sibling hex.
func inclusionStepStrings(_ steps: [InclusionStep]) -> [String] {
    steps.map { step in
        switch step {
        case .promote: return "promote"
        case .sibling(let s): return s
        }
    }
}

/// The NORMATIVE DSDP §2.3 disclosed-leaf check, in Swift.
///
/// RECOMPUTES the leaf hash from `(keyPath, salt, tag, value)` via the pinned Rust Poseidon
/// (`hashLeafHex`) — it never trusts a caller-supplied leaf hash — then folds the root-ward proof:
/// `.sibling` → `hashNodeHex` (commutative `Poseidon3([DS_NODE, min, max])`), `.promote` →
/// pass-through. Returns whether the fold equals `rootHex`.
///
/// The arity/domain split is what makes this sound: a disclosed leaf can only ever be a Poseidon5
/// image under `DS_LEAF`, while every internal node is a Poseidon3 image under `DS_NODE`; passing an
/// internal node off as a disclosed leaf would need `(keyPath, salt, tag, value)` whose Poseidon5
/// equals that node — preimage-infeasible. `.promote` steps do not change the arithmetic result;
/// they carry tree-shape/depth information, not authentication.
func verifyInclusion(keyPath: String, saltHex: String, tag: UInt8, value: String,
                     steps: [InclusionStep], rootHex: String) throws -> Bool {
    var h = try hashLeafHex(keyPath: keyPath, saltHex: saltHex, tag: tag, value: value)
    for step in steps {
        switch step {
        case .sibling(let s):
            h = try hashNodeHex(aHex: h, bHex: s)
        case .promote:
            break // lone odd node — pass-through, no hashing
        }
    }
    return h.lowercased() == rootHex.lowercased()
}

/// Result of the iOS inclusion-proof conformance run over the shared `testvectors.json`.
struct InclusionParityResult {
    let pass: Bool
    let accepted: Int
    let rejected: Int
    let detail: String
}

/// Drive the Swift `Sibling | Promote` verifier over every shared inclusion vector, cross-checking
/// it against the canonical Rust FFI (`verifyInclusionProofHex`). Both the Swift fold and the Rust
/// reference MUST agree with each vector's `valid` flag — the iOS leg of the Rust↔TS↔Swift suite,
/// covering all audit leaf counts, multi-level promotion, and the negative (reject) cases.
func runInclusionConformance(vectorsRoot: [String: Any]) -> InclusionParityResult {
    guard let vectors = vectorsRoot["inclusion"] as? [[String: Any]] else {
        return InclusionParityResult(pass: false, accepted: 0, rejected: 0, detail: "no inclusion vectors")
    }
    var accepted = 0
    var rejected = 0
    var maxPromotes = 0
    do {
        for v in vectors {
            let keyPath = v["keyPath"] as? String ?? ""
            let saltHex = v["saltHex"] as? String ?? ""
            let tag = UInt8((v["tag"] as? Int) ?? 0)
            let value = (v["value"] as? String) ?? "" // null (tag 0) packs as empty, matching hashLeafHex
            let rootHex = v["root"] as? String ?? ""
            let want = (v["valid"] as? Bool) ?? false
            let name = (v["name"] as? String) ?? "?"

            let steps = inclusionSteps(fromJson: (v["steps"] as? [[String: Any]]) ?? [])
            maxPromotes = max(maxPromotes, steps.filter { $0 == .promote }.count)

            let gotSwift = try verifyInclusion(keyPath: keyPath, saltHex: saltHex, tag: tag,
                                               value: value, steps: steps, rootHex: rootHex)
            let gotRust = try verifyInclusionProofHex(keyPath: keyPath, saltHex: saltHex, tag: tag,
                                                      value: value, proofSteps: inclusionStepStrings(steps),
                                                      rootHex: rootHex)
            if gotSwift != want || gotRust != want {
                return InclusionParityResult(pass: false, accepted: accepted, rejected: rejected,
                    detail: "mismatch '\(name)': swift=\(gotSwift) rust=\(gotRust) want=\(want)")
            }
            if want { accepted += 1 } else { rejected += 1 }
        }
    } catch {
        return InclusionParityResult(pass: false, accepted: accepted, rejected: rejected,
                                     detail: "FFI error: \(error)")
    }
    if maxPromotes < 2 {
        return InclusionParityResult(pass: false, accepted: accepted, rejected: rejected,
                                     detail: "vectors did not exercise multi-level promotion")
    }
    return InclusionParityResult(pass: true, accepted: accepted, rejected: rejected,
        detail: "\(accepted) accepted, \(rejected) rejected — Swift == Rust == vectors")
}

/// Convenience wrapper that loads the bundled `testvectors.json` (used by the Verify tab panel).
func runInclusionConformance() -> InclusionParityResult {
    guard let url = Bundle.main.url(forResource: "testvectors", withExtension: "json"),
          let data = try? Data(contentsOf: url),
          let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
        return InclusionParityResult(pass: false, accepted: 0, rejected: 0,
                                     detail: "could not load testvectors.json")
    }
    return runInclusionConformance(vectorsRoot: root)
}
