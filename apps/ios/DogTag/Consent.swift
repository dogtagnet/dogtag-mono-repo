import Foundation

/// Canonical field encoding for the public labels bound into an owner-hidden consent proof.
enum ConsentField {
    private static let zero32 =
        "0x0000000000000000000000000000000000000000000000000000000000000000"

    /// A bytes32 input passes through; a human-readable purpose or record type is keccak256-hashed.
    static func keccakLabel(_ label: String) -> String {
        if label.isEmpty { return zero32 }
        if label.hasPrefix("0x"), label.count == 66 { return label }
        let hash = Keccak256.digest(Data(label.utf8))
        return "0x" + hash.map { String(format: "%02x", $0) }.joined()
    }
}
