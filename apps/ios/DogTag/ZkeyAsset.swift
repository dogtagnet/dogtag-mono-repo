import Foundation

/// The proving artifacts ONE protocol version needs, named by their bundle resources.
///
/// Mirrors the Rust `dogtag_prover::artifact::ArtifactDescriptor` (and Android `ArtifactDescriptor`)
/// — the same version-keyed model, expressed per platform. Only the fields this resolver needs to
/// resolve a path are here: the server-side descriptor additionally carries the r1cs/wasm the ark
/// backend uses (this app's witness backend is the graph interpreter) and the integrity pins.
///
/// Hashes are deliberately absent: a bundled resource ships inside the signed `.app`, so its
/// integrity is the code signature's job, not a hash check here. A version whose artifacts are
/// FETCHED must pin and verify them before load — see the resolver docs.
struct ArtifactDescriptor {
    /// The version key — what the protocol calls this artifact set.
    let version: String
    /// The Groth16 proving key resource (~65 MB), as `(name, extension)`.
    let zkey: (name: String, ext: String)
    /// The precompiled witness graph resource (~3 MB) the on-device prover interprets.
    let graph: (name: String, ext: String)
}

/// Resolves the on-device prover assets for a protocol version — the Groth16 proving key and the
/// witness graph — to absolute filesystem paths so the prover (`proveVerification`) can read them by
/// path.
///
/// ## Version-keyed, not fetched
///
/// Which files a version needs comes from its ``ArtifactDescriptor`` (``registry``) rather than from
/// hard-coded filenames, so this app can hold more than one artifact set. What it does NOT do is
/// fetch: every registered version resolves to resources bundled in the app, exactly as before.
/// Making a version resolvable from the network — download, verify its published SHA-256 BEFORE
/// load, cache it content-addressed — is the later discovery work, and ``ensure(descriptor:)`` is the
/// seam it plugs into.
///
/// ## Resolving
///
/// Mirrors Android `data/ZkeyAsset.kt`. On Android the assets ship compressed inside the APK and must
/// be copied into `filesDir` before a path exists; on iOS bundled resources already live on the
/// filesystem (inside `DogTag.app`) with a stable absolute path, so "ensure" simply resolves that
/// path from the bundle — no copy is required. The contract is identical: return the absolute path,
/// or nil if the asset is missing from the bundle.
enum ZkeyAsset {
    /// The current Level-A artifact set — what a caller naming no version gets.
    static let levelAV1 = ArtifactDescriptor(
        version: "dogtag-levela/1",
        zkey: (name: "verification_final", ext: "zkey"),
        graph: (name: "verification", ext: "graph")
    )

    /// Every version this build can prove. A consent version joins it with the consent path (M7 P1).
    static let registry: [ArtifactDescriptor] = [levelAV1]

    /// The version a caller gets when it names none.
    static func current() -> ArtifactDescriptor { levelAV1 }

    /// Resolve a version key to its artifact set.
    ///
    /// `nil` ⇒ ``current()`` (back-compat: callers that name no version get the Level-A set). An
    /// unknown version returns nil rather than falling back — proving with another version's key
    /// yields a proof its verifier rejects, so failing closed beats answering wrongly.
    static func resolve(version: String? = nil) -> ArtifactDescriptor? {
        guard let version else { return current() }
        return registry.first { $0.version == version }
    }

    /// Absolute path to `descriptor`'s bundled zkey asset, or nil if missing. (Equivalent to Android
    /// `ensure`.)
    static func ensure(descriptor: ArtifactDescriptor = current()) -> String? {
        Bundle.main.url(forResource: descriptor.zkey.name, withExtension: descriptor.zkey.ext)?.path
    }

    /// Absolute path to `descriptor`'s bundled witness-graph asset, or nil if missing. Same contract
    /// as ``ensure(descriptor:)``; mirrors Android `ZkeyAsset.ensureGraph()`.
    static func ensureGraph(descriptor: ArtifactDescriptor = current()) -> String? {
        Bundle.main.url(forResource: descriptor.graph.name, withExtension: descriptor.graph.ext)?.path
    }
}
