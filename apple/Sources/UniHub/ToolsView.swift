import SwiftUI
import UniHubCore

/// The Unity CLI and the tools it can install, like the Hub's CLI integration.
struct ToolsView: View {
    @Environment(AppModel.self) private var model
    @State private var confirmCliInstall = false
    @State private var confirmCliUninstall = false
    @State private var pluginToInstall: Plugin?
    @State private var pluginToRemove: Plugin?

    var body: some View {
        List {
            Section("Unity CLI") {
                CliRow(
                    model: model,
                    onInstall: { confirmCliInstall = true },
                    onUninstall: { confirmCliUninstall = true }
                )
            }
            if model.cli != nil {
                Section {
                    ForEach(model.plugins) { plugin in
                        PluginRow(
                            model: model,
                            plugin: plugin,
                            onInstall: { pluginToInstall = plugin },
                            onRemove: { pluginToRemove = plugin }
                        )
                    }
                    if model.plugins.isEmpty && !model.isLoadingTools {
                        Text("No tools are available.").foregroundStyle(.secondary)
                    }
                } header: {
                    Text("Tools")
                } footer: {
                    Text("Tools are downloaded and managed by the Unity CLI.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle("Tools")
        .toolbar {
            Button("Check for Updates", systemImage: "arrow.clockwise") {
                Task { await model.refreshTools(refresh: true) }
            }
            .disabled(model.isLoadingTools)
        }
        .overlay {
            if model.isLoadingTools && model.plugins.isEmpty { ProgressView() }
        }
        .task { await model.refreshTools(refresh: false) }
        .confirmationDialog("Install the Unity CLI?", isPresented: $confirmCliInstall) {
            Button("Install") { Task { await model.installCli() } }
        } message: {
            Text("""
                UniHub runs Unity's official installer from public-cdn.cloud.unity3d.com. \
                It checks the download's SHA-256, installs it to ~/.unity/bin, and adds that \
                folder to PATH in your shell configuration (for example ~/.zshrc).
                """)
        }
        .confirmationDialog("Uninstall the Unity CLI?", isPresented: $confirmCliUninstall) {
            Button("Uninstall", role: .destructive) { Task { await model.uninstallCli() } }
        } message: {
            Text("Sign-in, licenses and tools in UniHub need the CLI. Your Unity account stays signed in.")
        }
        .confirmationDialog(
            "Install \(pluginToInstall?.displayName ?? "")?",
            isPresented: Binding(get: { pluginToInstall != nil }, set: { if !$0 { pluginToInstall = nil } }),
            presenting: pluginToInstall
        ) { plugin in
            Button("Agree and Install") { Task { await model.install(plugin) } }
        } message: { plugin in
            Text("Installing \(plugin.displayName) means you accept its license terms. Run `unity plugin install \(plugin.id) --dry-run` in Terminal to read them first.")
        }
        .confirmationDialog(
            "Remove \(pluginToRemove?.displayName ?? "")?",
            isPresented: Binding(get: { pluginToRemove != nil }, set: { if !$0 { pluginToRemove = nil } }),
            presenting: pluginToRemove
        ) { plugin in
            Button("Remove", role: .destructive) { Task { await model.remove(plugin) } }
        }
    }
}

private struct CliRow: View {
    let model: AppModel
    let onInstall: () -> Void
    let onUninstall: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "terminal.fill")
                .font(.title)
                .foregroundStyle(.tint)
                .frame(width: 36)
            VStack(alignment: .leading, spacing: 3) {
                Text("Unity CLI").font(.headline)
                if let cli = model.cli {
                    Text("Version \(cli.version)\(cli.channel.map { " · \($0)" } ?? "")")
                        .monospacedDigit()
                    Text(cli.path).font(.caption).foregroundStyle(.secondary)
                    if let update = model.cliUpdate, update.updateAvailable, let latest = update.latestVersion {
                        Text("Version \(latest) is available.").font(.caption).foregroundStyle(.orange)
                    }
                } else {
                    Text("Unity's command-line tool. UniHub uses it for sign-in, licenses and tools.")
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if let busy = model.cliBusy {
                ProgressView().controlSize(.small)
                Text(busy).foregroundStyle(.secondary)
            } else if model.cli == nil {
                Button("Install", action: onInstall).buttonStyle(.borderedProminent)
            } else {
                if model.cliUpdate?.updateAvailable == true {
                    Button("Update") { Task { await model.updateCli() } }
                        .buttonStyle(.borderedProminent)
                }
                Menu {
                    Button("Check for Updates") { Task { await model.refreshCli(checkUpdate: true) } }
                    if let path = model.cli?.path {
                        Button("Show in Finder") { model.revealInFinder(path) }
                    }
                    Divider()
                    Button("Uninstall…", role: .destructive, action: onUninstall)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
            }
        }
        .padding(.vertical, 6)
    }
}

private struct PluginRow: View {
    let model: AppModel
    let plugin: Plugin
    let onInstall: () -> Void
    let onRemove: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: plugin.kind == "runtime" ? "gearshape.2" : "hammer")
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 36)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(plugin.displayName).font(.headline)
                    Text(plugin.id).font(.caption).foregroundStyle(.secondary)
                }
                Text(plugin.description)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                Text(versionText).font(.caption).monospacedDigit()
                    .foregroundStyle(plugin.newerVersion != nil ? .orange : .secondary)
            }
            Spacer()
            if model.busyPlugins.contains(plugin.id) {
                ProgressView().controlSize(.small)
            } else if let blocker = plugin.blockedByCliVersion {
                Text("Needs Unity CLI \(blocker)").font(.caption).foregroundStyle(.secondary)
            } else if !plugin.installed {
                Button("Install", action: onInstall)
            } else {
                if plugin.newerVersion != nil {
                    Button("Update") { Task { await model.upgrade(plugin) } }
                }
                if plugin.kind == "runtime" {
                    Text("Used by the CLI").font(.caption).foregroundStyle(.secondary)
                } else if plugin.managed {
                    Button("Remove", action: onRemove)
                } else {
                    // Found elsewhere (npm, Homebrew…): the Unity CLI cannot remove it.
                    Text("Installed outside the Unity CLI")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .help(plugin.path.map { "At \($0). Remove it the way it was installed." }
                              ?? "Remove it the way it was installed.")
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var versionText: String {
        if let installed = plugin.installedVersion {
            if let newer = plugin.newerVersion { return "Installed \(installed) · \(newer) available" }
            return "Installed \(installed)"
        }
        return plugin.availableVersion.map { "Not installed · \($0)" } ?? "Not installed"
    }
}
