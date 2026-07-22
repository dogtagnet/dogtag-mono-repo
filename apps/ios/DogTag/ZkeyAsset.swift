import Foundation

/// The proving artifacts one protocol version needs, named by their bundle resources.
struct ArtifactDescriptor {
    let version: String
    let zkey: (name: String, ext: String)
    let graph: (name: String, ext: String)
}

/// Resolves the frozen owner-hidden consent proving key and witness graph from the signed app bundle.
/// The version key remains `dogtag-levelb/1` for deployed protocol compatibility.
enum ZkeyAsset {
    static let ownerHiddenV1 = ArtifactDescriptor(
        version: "dogtag-levelb/1",
        zkey: (name: "consent_final", ext: "zkey"),
        graph: (name: "consent", ext: "graph")
    )

    static let registry: [ArtifactDescriptor] = [ownerHiddenV1]

    static func current() -> ArtifactDescriptor { ownerHiddenV1 }

    /// Unknown versions fail closed rather than proving against a different verifying key.
    static func resolve(version: String? = nil) -> ArtifactDescriptor? {
        guard let version else { return current() }
        return registry.first { $0.version == version }
    }

    static func ensure(descriptor: ArtifactDescriptor = current()) -> String? {
        Bundle.main.url(forResource: descriptor.zkey.name, withExtension: descriptor.zkey.ext)?.path
    }

    static func ensureGraph(descriptor: ArtifactDescriptor = current()) -> String? {
        Bundle.main.url(forResource: descriptor.graph.name, withExtension: descriptor.graph.ext)?.path
    }
}
