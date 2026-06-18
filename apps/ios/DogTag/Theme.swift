import SwiftUI

/// The seven brand accents the user can pick from (reference: pet-owner app theme dot row).
/// Each theme resolves to a full set of semantic color tokens for BOTH light and dark.
/// Mirrors apps/android/.../ui/Theme.kt ThemeId.
enum ThemeId: Int, CaseIterable, Identifiable {
    case black, white, blue, red, pink, green, yellow
    var id: Int { rawValue }

    var label: String {
        switch self {
        case .black: return "Black"
        case .white: return "White"
        case .blue: return "Blue"
        case .red: return "Red"
        case .pink: return "Pink"
        case .green: return "Green"
        case .yellow: return "Yellow"
        }
    }

    /// The accent color shown in the theme dot.
    var accent: Color {
        switch self {
        case .black: return Color(hex: 0x222222)
        case .white: return Color(hex: 0x6E7178)
        case .blue: return Color(hex: 0x2F6BFF)
        case .red: return Color(hex: 0xEF4E4E)
        case .pink: return Color(hex: 0xEC6FB0)
        case .green: return Color(hex: 0x2FB36B)
        case .yellow: return Color(hex: 0xF2B23A)
        }
    }
}

/// Tri-state dark preference: follow system, force light, force dark.
enum DarkPref: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }
}

/// Semantic design tokens — views reference these, never raw hex. The theme layer maps the chosen
/// (ThemeId, isDark) pair onto a concrete token set so a single picker + a dark switch drive the
/// whole app (7 themes × light/dark = 14 palettes). Mirrors DogTagColors in Theme.kt.
struct DogTagColors {
    let accent: Color
    let onAccent: Color
    let background: Color
    let surface: Color
    let surfaceVariant: Color
    let onBackground: Color
    let onSurface: Color
    let muted: Color
    let outline: Color
    let success: Color
    let danger: Color
    let healthTint: Color
    let serviceTint: Color
    let travelTint: Color
    let isDark: Bool

    static func tokens(for id: ThemeId, dark: Bool) -> DogTagColors {
        let accent = id.accent
        if dark {
            return DogTagColors(
                accent: accent,
                onAccent: .white,
                background: Color(hex: 0x0E1014),
                surface: Color(hex: 0x181B22),
                surfaceVariant: Color(hex: 0x222631),
                onBackground: Color(hex: 0xF2F4F8),
                onSurface: Color(hex: 0xE6E9EF),
                muted: Color(hex: 0x9AA0AC),
                outline: Color(hex: 0x2C313C),
                success: Color(hex: 0x45C97B),
                danger: Color(hex: 0xFF6B6B),
                healthTint: Color(hex: 0x3A1F22),
                serviceTint: Color(hex: 0x15301F),
                travelTint: Color(hex: 0x152234),
                isDark: true
            )
        } else {
            return DogTagColors(
                accent: accent,
                onAccent: .white,
                background: Color(hex: 0xF6F7FB),
                surface: Color(hex: 0xFFFFFF),
                surfaceVariant: Color(hex: 0xEEF0F6),
                onBackground: Color(hex: 0x13151A),
                onSurface: Color(hex: 0x1B1E25),
                muted: Color(hex: 0x6B7180),
                outline: Color(hex: 0xE2E5EE),
                success: Color(hex: 0x1B7F3B),
                danger: Color(hex: 0xD23B3B),
                healthTint: Color(hex: 0xFDECEC),
                serviceTint: Color(hex: 0xE7F6EE),
                travelTint: Color(hex: 0xE8F1FD),
                isDark: false
            )
        }
    }
}

/// Observable theme state. Persists themeId + darkPref in UserDefaults (mirrors SettingsStore.kt).
final class ThemeManager: ObservableObject {
    @Published var themeId: ThemeId {
        didSet { UserDefaults.standard.set(themeId.rawValue, forKey: "theme_id") }
    }
    @Published var darkPref: DarkPref {
        didSet { UserDefaults.standard.set(darkPref.rawValue, forKey: "dark_pref") }
    }

    init() {
        let raw = UserDefaults.standard.object(forKey: "theme_id") as? Int ?? ThemeId.pink.rawValue
        themeId = ThemeId(rawValue: raw) ?? .pink
        let dp = UserDefaults.standard.string(forKey: "dark_pref") ?? DarkPref.system.rawValue
        darkPref = DarkPref(rawValue: dp) ?? .system
    }

    /// Resolve tokens for the current system color scheme.
    func colors(systemDark: Bool) -> DogTagColors {
        let dark: Bool
        switch darkPref {
        case .system: dark = systemDark
        case .light: dark = false
        case .dark: dark = true
        }
        return DogTagColors.tokens(for: themeId, dark: dark)
    }

    /// The SwiftUI preferred color scheme override (nil = follow system).
    var preferredColorScheme: ColorScheme? {
        switch darkPref {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

extension Color {
    init(hex: UInt32) {
        let r = Double((hex >> 16) & 0xFF) / 255.0
        let g = Double((hex >> 8) & 0xFF) / 255.0
        let b = Double(hex & 0xFF) / 255.0
        self.init(.sRGB, red: r, green: g, blue: b, opacity: 1.0)
    }
}
