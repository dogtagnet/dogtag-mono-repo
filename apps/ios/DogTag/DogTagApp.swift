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
