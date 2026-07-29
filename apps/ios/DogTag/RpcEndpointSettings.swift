import Foundation

/// Monotonic identity for asynchronous endpoint checks.
///
/// URLSession cancellation is cooperative: a probe can still return after its task was cancelled.
/// Callers therefore accept a completion only while its token is current. `invalidate()` is also
/// used when the settings view disappears, so an old view instance cannot overwrite a choice made
/// after the user returns.
struct RpcEndpointRequestGeneration {
    private(set) var current: UInt64 = 0

    mutating func begin() -> UInt64 {
        current &+= 1
        return current
    }

    mutating func invalidate() {
        current &+= 1
    }

    func accepts(_ token: UInt64) -> Bool {
        token == current
    }
}

/// The holder's persisted JSON-RPC choice.
///
/// This preference is deliberately narrower than `AppConfig`: only the CHAIN transport is
/// user-selectable. The provider directory (`centralApi`) and QR-discovered role APIs remain outside
/// this store, so changing an RPC endpoint can never repoint a centralized DogTag service.
enum RpcEndpointSettings {
    private static let storageKey = "roax_rpc_url"

    /// The saved custom endpoint, or nil when the bundled endpoint is selected. Invalid legacy/corrupt
    /// values fail closed to nil rather than being handed to URLSession.
    static func customRpc(in defaults: UserDefaults = .standard) -> String? {
        guard let raw = defaults.string(forKey: storageKey),
              let normalized = normalizedURL(raw),
              normalized != AppConfig.roaxRpc else { return nil }
        return normalized
    }

    /// What every chain call site should request. `RoaxRpc` still checks `eth_chainId` immediately
    /// before each read: persistence proves only what the user chose, not what that peer serves now.
    static func rpcUrl(in defaults: UserDefaults = .standard) -> String {
        customRpc(in: defaults) ?? AppConfig.roaxRpc
    }

    /// Persist a custom URL only when the chain guard accepted that SAME normalized endpoint.
    /// Any other route clears an older custom choice, making the bundled endpoint active. Taking the
    /// typed route rather than a caller-supplied Bool prevents a future settings caller from persisting
    /// a URL merely because its syntax looked valid.
    @discardableResult
    static func saveChecked(
        _ raw: String,
        route: RoaxRpc.EndpointRoute,
        in defaults: UserDefaults = .standard
    ) -> Bool {
        guard let normalized = normalizedURL(raw),
              case .custom(let checked) = route,
              checked == normalized else {
            useBundled(in: defaults)
            return false
        }
        defaults.set(normalized, forKey: storageKey)
        return true
    }

    static func useBundled(in defaults: UserDefaults = .standard) {
        defaults.removeObject(forKey: storageKey)
    }

    /// Keep endpoint paths and query tokens intact; only trim accidental surrounding whitespace.
    /// Fragments are rejected because they are never sent in an HTTP request and would make the saved
    /// value claim a distinction that the peer cannot observe.
    static func normalizedURL(_ raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              scheme == "https" || scheme == "http",
              components.host?.isEmpty == false,
              components.fragment == nil,
              components.url != nil else { return nil }
        return trimmed
    }
}
