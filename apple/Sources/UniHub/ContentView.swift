import SwiftUI
import UniHubCore

enum Page: String, CaseIterable, Identifiable {
    case projects = "Projects"
    case installs = "Installs"
    case releases = "Releases"
    case tools = "Tools"

    var id: Self { self }

    var symbol: String {
        switch self {
        case .projects: "folder"
        case .installs: "shippingbox"
        case .releases: "arrow.down.circle"
        case .tools: "wrench.and.screwdriver"
        }
    }
}

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings
    /// Remembered between launches. `-page tools` on the command line overrides it.
    @AppStorage("page") private var page: Page = .projects

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            List(Page.allCases, selection: Binding<Page?>(get: { page }, set: { page = $0 ?? .projects })) { item in
                Label(item.rawValue, systemImage: item.symbol)
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 200)
            .safeAreaInset(edge: .bottom) {
                AccountBadge(model: model)
                    .padding(10)
            }
        } detail: {
            switch page {
            case .projects: ProjectsView()
            case .installs: EditorsView()
            case .releases: ReleasesView()
            case .tools: ToolsView()
            }
        }
        .task {
            // Development aid: `-openSettings YES` opens Settings at launch.
            if UserDefaults.standard.bool(forKey: "openSettings") { openSettings() }
            await model.refreshAccount()
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(
                get: { model.errorMessage != nil },
                set: { if !$0 { model.errorMessage = nil } }
            ),
            actions: { Button("OK", role: .cancel) {} },
            message: { Text(model.errorMessage ?? "") }
        )
    }
}

/// Signed-in user at the bottom of the sidebar, like the Hub's profile button.
private struct AccountBadge: View {
    let model: AppModel
    @AppStorage("settingsPane") private var settingsPane: SettingsPane = .accounts

    var body: some View {
        if let auth = model.auth, auth.signedIn {
            SettingsLink {
                HStack(spacing: 8) {
                    Avatar(name: auth.displayName, size: 28)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(auth.displayName).lineLimit(1)
                        if let email = auth.email {
                            Text(email).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .simultaneousGesture(TapGesture().onEnded { settingsPane = .accounts })
            .help("Account settings")
            .contextMenu {
                Button("Sign Out") { Task { await model.signOut() } }
            }
        } else if model.isSigningIn {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Finish in your browser…").font(.callout)
                Spacer(minLength: 0)
                Button("Cancel") { model.cancelSignIn() }.controlSize(.small)
            }
        } else if model.isLoadingAccount && model.auth == nil {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Checking account…").font(.callout).foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
        } else if model.cli != nil {
            Button {
                Task { await model.signIn() }
            } label: {
                Label("Sign In", systemImage: "person.crop.circle")
                    .frame(maxWidth: .infinity)
            }
            .controlSize(.large)
        } else {
            Text("Install the Unity CLI from Tools to sign in.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct Avatar: View {
    let name: String?
    var size: CGFloat = 32

    var body: some View {
        Text(initials(name))
            .font(.system(size: size * 0.4, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(Circle().fill(.tint))
    }
}
