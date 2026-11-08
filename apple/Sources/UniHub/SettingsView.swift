import AppKit
import SwiftUI
import UniHubCore

/// Preferences window (⌘,). Sections follow the official Hub's Preferences.
enum SettingsPane: String, CaseIterable, Identifiable {
    case accounts = "Accounts"
    case licenses = "Licenses"
    case appearance = "Appearance"
    case projects = "Projects"
    case installs = "Installs"
    case cli = "Unity CLI"
    case advanced = "Advanced"

    var id: Self { self }

    var symbol: String {
        switch self {
        case .accounts: "person.crop.circle"
        case .licenses: "checkmark.seal"
        case .appearance: "paintbrush"
        case .projects: "folder"
        case .installs: "shippingbox"
        case .cli: "terminal"
        case .advanced: "gearshape.2"
        }
    }
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    /// Remembered between launches. `-settingsPane licenses` overrides it.
    @AppStorage("settingsPane") private var pane: SettingsPane = .accounts

    var body: some View {
        HStack(spacing: 0) {
            List(SettingsPane.allCases, selection: Binding<SettingsPane?>(get: { pane }, set: { pane = $0 ?? .accounts })) { item in
                Label(item.rawValue, systemImage: item.symbol)
            }
            .listStyle(.sidebar)
            .frame(width: 190)

            Divider()

            Group {
                switch pane {
                case .accounts: AccountsPane()
                case .licenses: LicensesPane()
                case .appearance: AppearancePane()
                case .projects: ProjectsPane()
                case .installs: InstallsPane()
                case .cli: CliPane()
                case .advanced: AdvancedPane()
                }
            }
            .formStyle(.grouped)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: 760, height: 520)
        .task { await model.refreshAccount() }
    }
}

// MARK: - Helpers

/// Two-way binding into one settings field; every change is saved immediately.
@MainActor
private func setting<T>(_ model: AppModel, _ keyPath: WritableKeyPath<UniHubCore.Settings, T>) -> Binding<T> {
    Binding(
        get: { model.settings[keyPath: keyPath] },
        set: { value in model.updateSettings { $0[keyPath: keyPath] = value } }
    )
}

@MainActor
private func chooseFolder(prompt: String) -> String? {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.canCreateDirectories = true
    panel.prompt = prompt
    return panel.runModal() == .OK ? panel.url?.path(percentEncoded: false) : nil
}

/// A folder setting: shows the path (or the default) with Choose… and Reset.
private struct FolderRow: View {
    let title: String
    let value: String?
    let defaultText: String
    let onChange: (String?) -> Void

    var body: some View {
        LabeledContent(title) {
            HStack {
                Text(value ?? defaultText)
                    .foregroundStyle(value == nil ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Button("Choose…") {
                    if let path = chooseFolder(prompt: "Choose") { onChange(path) }
                }
                if value != nil {
                    Button("Reset") { onChange(nil) }
                }
            }
        }
    }
}

/// Shown on panes that need the Unity CLI when it is missing.
private struct NeedsCli: View {
    let model: AppModel

    var body: some View {
        Section {
            ContentUnavailableView {
                Label("Unity CLI not installed", systemImage: "terminal")
            } description: {
                Text("UniHub signs in and manages licenses through Unity's official command-line tool. Install it from the Tools page in the main window.")
            }
        }
    }
}

// MARK: - Accounts

private struct AccountsPane: View {
    @Environment(AppModel.self) private var model
    @State private var confirmSignOut = false

    var body: some View {
        Form {
            if model.cli == nil && !model.isLoadingAccount {
                NeedsCli(model: model)
            } else if let auth = model.auth, auth.signedIn {
                Section {
                    HStack(spacing: 14) {
                        Avatar(name: auth.displayName, size: 48)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(auth.displayName).font(.title3.weight(.semibold))
                            if let email = auth.email { Text(email).foregroundStyle(.secondary) }
                            if let state = auth.sessionState {
                                Text("Session: \(state)").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        Button("Sign Out…") { confirmSignOut = true }
                    }
                    .padding(.vertical, 4)
                }
                if !model.accounts.isEmpty {
                    Section("Accounts on this Mac") {
                        ForEach(model.accounts) { account in
                            HStack {
                                Avatar(name: account.name ?? account.email, size: 24)
                                VStack(alignment: .leading) {
                                    Text(account.name ?? account.email ?? account.id)
                                    if let org = account.primaryOrg {
                                        Text(org).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                if account.isActive {
                                    Text("Active").font(.caption).foregroundStyle(.green)
                                } else {
                                    Button("Switch") { Task { await model.switchAccount(account) } }
                                }
                            }
                        }
                    }
                }
                Section {
                    Button("Add Another Account…") { Task { await model.signIn() } }
                        .disabled(model.isSigningIn)
                }
            } else {
                Section {
                    VStack(spacing: 12) {
                        Image(systemName: "person.crop.circle")
                            .font(.system(size: 44))
                            .foregroundStyle(.secondary)
                        Text("Sign in with your Unity ID").font(.title3.weight(.semibold))
                        Text("Signing in happens in your browser, through the Unity CLI. UniHub never sees your password or tokens.")
                            .multilineTextAlignment(.center)
                            .foregroundStyle(.secondary)
                        if model.isSigningIn {
                            HStack {
                                ProgressView().controlSize(.small)
                                Text("Waiting for the browser…")
                                Button("Cancel") { model.cancelSignIn() }
                            }
                        } else {
                            Button("Sign In") { Task { await model.signIn() } }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.large)
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                }
            }
        }
        .overlay {
            if model.isLoadingAccount && model.auth == nil { ProgressView() }
        }
        .confirmationDialog("Sign out of Unity?", isPresented: $confirmSignOut) {
            Button("Sign Out", role: .destructive) { Task { await model.signOut() } }
        } message: {
            Text("This signs out every account the Unity CLI stores on this Mac. The Unity Hub uses the same sign-in.")
        }
    }
}

// MARK: - Licenses

private struct LicensesPane: View {
    @Environment(AppModel.self) private var model
    @State private var confirmPersonal = false
    @State private var confirmReturn = false

    var body: some View {
        Form {
            if model.cli == nil && !model.isLoadingAccount {
                NeedsCli(model: model)
            } else if model.auth?.signedIn != true {
                Section {
                    Text("Sign in on the Accounts page to see and manage your licenses.")
                        .foregroundStyle(.secondary)
                }
            } else {
                Section("Active licenses") {
                    if let licenses = model.license?.licenses, !licenses.isEmpty {
                        ForEach(licenses, id: \.product) { license in
                            LabeledContent {
                                VStack(alignment: .trailing) {
                                    Text(license.kind ?? "")
                                    if let expires = license.expires {
                                        Text("Expires \(expires)").font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            } label: {
                                Label(license.product, systemImage: "checkmark.seal.fill")
                                if let org = license.organization {
                                    Text(org)
                                }
                            }
                        }
                    } else {
                        Text("No active license.").foregroundStyle(.secondary)
                    }
                }
                Section {
                    Button("Activate Unity Personal…") { confirmPersonal = true }
                    Button("Return Licenses…", role: .destructive) { confirmReturn = true }
                        .disabled(model.license?.licenses.isEmpty ?? true)
                } footer: {
                    Text("Licenses are activated through the Unity CLI and the Unity Licensing Client.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .confirmationDialog("Activate Unity Personal?", isPresented: $confirmPersonal) {
            Button("Agree and Activate") { Task { await model.activatePersonalLicense() } }
            Button("Read the Terms") { model.openURL("https://unity.com/legal") }
        } message: {
            Text("Unity Personal is free if you meet Unity's eligibility rules. By activating it you accept the Unity Personal license terms.")
        }
        .confirmationDialog("Return all licenses on this Mac?", isPresented: $confirmReturn) {
            Button("Return Licenses", role: .destructive) { Task { await model.returnLicenses() } }
        } message: {
            Text("The editor will not open until you activate a license again.")
        }
    }
}

// MARK: - Appearance

private struct AppearancePane: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            Section {
                Picker("Theme", selection: setting(model, \.appearance)) {
                    Text("System").tag(Appearance.system)
                    Text("Light").tag(Appearance.light)
                    Text("Dark").tag(Appearance.dark)
                }
                .pickerStyle(.segmented)
                Toggle("Hide UniHub when a project opens", isOn: setting(model, \.hideOnProjectOpen))
            }
            Section {
                Toggle("Show UniHub in the menu bar", isOn: $showMenuBarIcon)
                Text("Recent projects, your Unity account and help, from the menu bar. While it is shown, closing the window keeps UniHub running there.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @AppStorage(MenuBarMenu.showKey) private var showMenuBarIcon = true
}

// MARK: - Projects

private struct ProjectsPane: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            Section {
                FolderRow(
                    title: "Default project location",
                    value: model.settings.defaultProjectsDir,
                    defaultText: "~/Documents"
                ) { path in model.updateSettings { $0.defaultProjectsDir = path } }
                Toggle("Ask before removing a project from the list",
                       isOn: setting(model, \.askBeforeRemovingProject))
            }
            Section {
                Toggle("Add new Unity Hub projects when UniHub starts",
                       isOn: setting(model, \.syncHubProjectsOnLaunch))
                Button("Import from Unity Hub Now") { Task { await model.importFromHub() } }
            } footer: {
                Text("UniHub only reads Unity Hub's project list. It never changes it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Installs

private struct InstallsPane: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            Section {
                FolderRow(
                    title: "Editors install location",
                    value: model.settings.editorInstallDir,
                    defaultText: "/Applications/Unity/Hub/Editor"
                ) { path in model.updateSettings { $0.editorInstallDir = path } }
                FolderRow(
                    title: "Downloads location",
                    value: model.settings.downloadsDir,
                    defaultText: "UniHub data folder"
                ) { path in model.updateSettings { $0.downloadsDir = path } }
                Stepper(value: setting(model, \.maxConcurrentDownloads), in: 1...8) {
                    LabeledContent("Simultaneous downloads", value: "\(model.settings.maxConcurrentDownloads)")
                }
            } footer: {
                Text("UniHub's own installer is still in development. Until then, editors are installed with Unity Hub, and the download settings take effect once it ships.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Other editor folders") {
                ForEach(model.settings.extraEditorDirs, id: \.self) { dir in
                    HStack {
                        Text(dir).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Button("Remove", systemImage: "minus.circle") {
                            model.updateSettings { $0.extraEditorDirs.removeAll { $0 == dir } }
                        }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                    }
                }
                Button("Add Folder…") {
                    if let path = chooseFolder(prompt: "Add") {
                        model.updateSettings { $0.extraEditorDirs.append(path) }
                    }
                }
            }
        }
    }
}

// MARK: - Unity CLI

private struct CliPane: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            Section("Unity CLI") {
                if let cli = model.cli {
                    LabeledContent("Version", value: cli.version)
                    LabeledContent("Channel", value: cli.channel ?? "—")
                    LabeledContent("Location") {
                        Text(cli.path).lineLimit(1).truncationMode(.middle)
                    }
                } else {
                    Text("Not installed. Install it from the Tools page.").foregroundStyle(.secondary)
                }
            }
            Section {
                Picker("Release channel", selection: setting(model, \.cliChannel)) {
                    Text("Stable").tag(CliChannel.stable)
                    Text("Beta").tag(CliChannel.beta)
                }
                LabeledContent("Custom location") {
                    HStack {
                        Text(model.settings.unityCliPath ?? "Find automatically")
                            .foregroundStyle(model.settings.unityCliPath == nil ? .secondary : .primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Button("Choose…") {
                            let panel = NSOpenPanel()
                            panel.canChooseFiles = true
                            panel.canChooseDirectories = false
                            if panel.runModal() == .OK, let url = panel.url {
                                model.updateSettings { $0.unityCliPath = url.path(percentEncoded: false) }
                                Task { await model.refreshAccount() }
                            }
                        }
                        if model.settings.unityCliPath != nil {
                            Button("Reset") {
                                model.updateSettings { $0.unityCliPath = nil }
                                Task { await model.refreshAccount() }
                            }
                        }
                    }
                }
            } footer: {
                Text("UniHub looks in ~/.unity/bin, /opt/homebrew/bin and /usr/local/bin.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Advanced

private struct AdvancedPane: View {
    @Environment(AppModel.self) private var model
    @State private var confirmReset = false
    @AppStorage(AppModel.autoUpdateKey) private var autoUpdate = true

    var body: some View {
        Form {
            Section {
                LabeledContent("Version") {
                    HStack {
                        Text(Bundle.main.shortVersion)
                        Button("Check for Updates") { Task { await model.checkForUpdate(manual: true) } }
                            .disabled(model.isCheckingForUpdate || model.isInstallingUpdate)
                        if model.isCheckingForUpdate { ProgressView().controlSize(.small) }
                    }
                }
                Toggle("Check for updates automatically", isOn: $autoUpdate)
                Text("Once a day, UniHub looks for a newer release on GitHub (github.com/flowerfx/UniHub/releases) and offers to install it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let dir = model.dataDirectory {
                    LabeledContent("Data folder") {
                        HStack {
                            Text(dir).lineLimit(1).truncationMode(.middle)
                            Button("Show in Finder") { model.revealInFinder(dir) }
                        }
                    }
                }
            }
            Section {
                Button("Reset All Settings…", role: .destructive) { confirmReset = true }
            }
        }
        .confirmationDialog("Reset all settings?", isPresented: $confirmReset) {
            Button("Reset", role: .destructive) { model.resetSettings() }
        } message: {
            Text("Your project list and Unity sign-in are kept.")
        }
    }
}

private extension Bundle {
    var shortVersion: String {
        (infoDictionary?["CFBundleShortVersionString"] as? String) ?? "development build"
    }
}
