import Foundation

/// Materialises the bundled on-device prover assets — the Groth16 proving key
/// (`verification_final.zkey`, ~65 MB) and the witness graph (`verification.graph`, ~3 MB) — as
/// absolute filesystem paths so the prover (`proveVerification`) can read them by path.
///
/// Mirrors Android `data/ZkeyAsset.kt`. On Android the assets ship compressed inside the APK and
/// must be copied into `filesDir` before a path exists; on iOS bundled resources already live on the
/// filesystem (inside `DogTag.app`) with a stable absolute path, so "ensure" simply resolves that
/// path from the bundle — no copy is required. The contract is identical: return the absolute path,
/// or nil if the asset is missing from the bundle.
enum ZkeyAsset {
    private static let zkeyName = "verification_final"
    private static let zkeyExt = "zkey"
    private static let graphName = "verification"
    private static let graphExt = "graph"

    /// Absolute path to the bundled zkey asset, or nil if missing. (Equivalent to Android `ensure`.)
    static func ensure() -> String? {
        Bundle.main.url(forResource: zkeyName, withExtension: zkeyExt)?.path
    }

    /// Absolute path to the bundled witness-graph asset, or nil if missing. Same contract as
    /// [ensure]; mirrors Android `ZkeyAsset.ensureGraph()`.
    static func ensureGraph() -> String? {
        Bundle.main.url(forResource: graphName, withExtension: graphExt)?.path
    }
}
