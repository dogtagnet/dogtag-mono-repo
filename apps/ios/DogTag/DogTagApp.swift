import SwiftUI

@main
struct DogTagApp: App {
    @StateObject private var theme = ThemeManager()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(theme)
                .preferredColorScheme(theme.preferredColorScheme)
        }
    }
}

/// Resolves the active DogTagColors from the ThemeManager + the current system color scheme, and
/// exposes them to the whole tree via the environment.
struct RootView: View {
    @EnvironmentObject var theme: ThemeManager
    @Environment(\.colorScheme) var systemScheme

    var body: some View {
        let colors = theme.colors(systemDark: systemScheme == .dark)
        MainTabView()
            .environment(\.dogTagColors, colors)
    }
}

private struct DogTagColorsKey: EnvironmentKey {
    static let defaultValue = DogTagColors.tokens(for: .pink, dark: false)
}

extension EnvironmentValues {
    var dogTagColors: DogTagColors {
        get { self[DogTagColorsKey.self] }
        set { self[DogTagColorsKey.self] = newValue }
    }
}

enum Tab: Hashable { case verify, travel, home, documents, profile }

struct MainTabView: View {
    @Environment(\.dogTagColors) var c
    @State private var tab: Tab = .home
    @State private var scanning = false

    var body: some View {
        ZStack(alignment: .bottom) {
            c.background.ignoresSafeArea()

            Group {
                switch tab {
                case .verify: VerifyScreen(onScan: { scanning = true })
                case .travel: TravelScreen(onScan: { scanning = true })
                case .home: HomeScreen(onScan: { scanning = true })
                case .documents: DocumentsScreen(onScan: { scanning = true })
                case .profile: ProfileScreen()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.bottom, 64)

            BottomBar(current: $tab)
        }
        .fullScreenCover(isPresented: $scanning) {
            ScanScreen(onDone: { scanning = false })
                .environment(\.dogTagColors, c)
        }
    }
}

private struct BottomBar: View {
    @Environment(\.dogTagColors) var c
    @Binding var current: Tab

    private struct Item { let tab: Tab; let label: String; let icon: String; let isHome: Bool }
    private var items: [Item] {
        [
            Item(tab: .verify, label: "Verify", icon: "checkmark.shield.fill", isHome: false),
            Item(tab: .travel, label: "Travel", icon: "airplane", isHome: false),
            Item(tab: .home, label: "Home", icon: "house.fill", isHome: true),
            Item(tab: .documents, label: "Documents", icon: "doc.text.fill", isHome: false),
            Item(tab: .profile, label: "Profile", icon: "person.fill", isHome: false),
        ]
    }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(items, id: \.tab) { item in
                let selected = item.tab == current
                Button {
                    current = item.tab
                } label: {
                    VStack(spacing: 3) {
                        if item.isHome {
                            ZStack {
                                Circle()
                                    .fill(selected ? c.accent : c.surfaceVariant)
                                    .frame(width: 40, height: 40)
                                Image(systemName: item.icon)
                                    .foregroundColor(selected ? c.onAccent : c.muted)
                                    .font(.system(size: 18))
                            }
                        } else {
                            Image(systemName: item.icon)
                                .font(.system(size: 18))
                                .foregroundColor(selected ? c.accent : c.muted)
                            Text(item.label)
                                .font(.system(size: 10, weight: selected ? .semibold : .regular))
                                .foregroundColor(selected ? c.accent : c.muted)
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.plain)
            }
        }
        .frame(height: 64)
        .background(c.surface.shadow(.drop(color: .black.opacity(0.12), radius: 6, y: -2)))
    }
}

/// Shared section header used across screens.
struct SectionTitle: View {
    @Environment(\.dogTagColors) var c
    let text: String
    var trailing: String? = nil
    var body: some View {
        HStack {
            Text(text).font(.system(size: 18, weight: .bold)).foregroundColor(c.onBackground)
            if let t = trailing {
                Spacer()
                Text(t).font(.system(size: 13)).foregroundColor(c.muted)
            }
        }
    }
}

// MARK: - copy affordances

/// Clipboard helpers behind the copy views. Namespaced (not free functions) so this file — which
/// carries the `@main` app entry point — contains no top-level code.
enum CopyFeedback {
    /// Copy `value` to the clipboard with a light success haptic. Callers own the transient "Copied" UI.
    static func copy(_ value: String) {
        UIPasteboard.general.string = value
        UINotificationFeedbackGenerator().notificationOccurred(.success)
    }

    /// Flip a "copied" flag on, then off after a short beat — the shared timing behind the copy views.
    static func flash(_ flag: Binding<Bool>) {
        withAnimation(.easeInOut(duration: 0.15)) { flag.wrappedValue = true }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.3) {
            withAnimation(.easeInOut(duration: 0.2)) { flag.wrappedValue = false }
        }
    }
}

/// A labeled, tap-to-copy value with a transient "Copied" confirmation. Mono-styled for the hash /
/// address / id values that pepper the app (Merkle roots, issuer addresses, dogTagIds, tx hashes).
///
/// State is self-contained so each row flashes "Copied" independently — never share one flag across
/// a screen. The full `value` is copied even when the display is middle-truncated.
struct CopyableMonoRow: View {
    @Environment(\.dogTagColors) var c
    let label: String
    let value: String
    /// Middle-truncate long values for display (the full value is still copied). Off for short ids.
    var truncate: Bool = true
    @State private var copied = false

    private var shown: String {
        guard truncate, value.count > 22 else { return value }
        return "\(value.prefix(12))…\(value.suffix(8))"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.system(size: 12, weight: .semibold)).foregroundColor(c.muted)
            Button {
                CopyFeedback.copy(value)
                CopyFeedback.flash($copied)
            } label: {
                HStack(spacing: 8) {
                    Text(value.isEmpty ? "—" : shown)
                        .font(.system(size: 13, design: .monospaced))
                        .foregroundColor(c.onBackground)
                        .lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 6)
                    if !value.isEmpty {
                        HStack(spacing: 4) {
                            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                                .font(.system(size: 12, weight: .semibold))
                            if copied {
                                Text("Copied").font(.system(size: 11, weight: .semibold))
                            }
                        }
                        .foregroundColor(copied ? c.success : c.muted)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(value.isEmpty)
            .accessibilityLabel("Copy \(label)")
        }
    }
}

/// A compact inline tap-to-copy label (e.g. a "DogTag #123" caption in a header). Shows `text`,
/// copies `copyValue` (defaults to `text`), and briefly swaps its trailing icon to a check + "Copied".
struct InlineCopyText: View {
    @Environment(\.dogTagColors) var c
    let text: String
    var copyValue: String? = nil
    var font: Font = .system(size: 12)
    var color: Color? = nil
    @State private var copied = false

    var body: some View {
        Button {
            CopyFeedback.copy(copyValue ?? text)
            CopyFeedback.flash($copied)
        } label: {
            HStack(spacing: 4) {
                Text(text).font(font).foregroundColor(color ?? c.muted)
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(copied ? c.success : c.muted.opacity(0.85))
                if copied {
                    Text("Copied").font(.system(size: 10, weight: .semibold)).foregroundColor(c.success)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Copy \(text)")
    }
}
