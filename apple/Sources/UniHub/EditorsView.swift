import UniHubCore
import SwiftUI

/// Installs page: installed editors with their platforms, installs in progress, and
/// actions to manage modules or uninstall.
struct EditorsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        List {
            if !model.installJobs.isEmpty {
                Section("Installing") {
                    ForEach(model.installJobs, id: \.id) { job in
                        JobRow(model: model, job: job)
                    }
                }
            }
            Section {
                ForEach(model.editors) { editor in
                    EditorRow(model: model, editor: editor)
                }
            } header: {
                if !model.installJobs.isEmpty { Text("Installed") }
            }
        }
        .navigationTitle("Installs")
        .navigationSubtitle("\(model.editors.count) editors")
        .toolbar {
            ToolbarItem {
                Button("Install Editor", systemImage: "plus") {
                    UserDefaults.standard.set(Page.releases.rawValue, forKey: "page")
                }
                .help("Pick a version on the Releases page")
            }
        }
        .overlay {
            if !model.hasLoadedLocal {
                ProgressView()
            } else if model.editors.isEmpty && model.installJobs.isEmpty {
                ContentUnavailableView {
                    Label("No editors found", systemImage: "shippingbox")
                } description: {
                    Text("UniHub looks in \(model.editorInstallDir). Install one from the Releases page.")
                } actions: {
                    Button("Show Releases") { UserDefaults.standard.set(Page.releases.rawValue, forKey: "page") }
                }
            }
        }
        .sheet(item: $model.modulesEditor) { editor in
            EditorModulesSheet(model: model, editor: editor)
        }
        .confirmationDialog(
            "Uninstall Unity \(model.editorPendingUninstall?.version ?? "")?",
            isPresented: Binding(
                get: { model.editorPendingUninstall != nil },
                set: { if !$0 { model.editorPendingUninstall = nil } }
            ),
            presenting: model.editorPendingUninstall
        ) { editor in
            Button("Uninstall", role: .destructive) { Task { await model.uninstall(editor) } }
        } message: { editor in
            let count = model.projects.filter { $0.editorVersion == editor.version }.count
            Text("Deletes \(editor.installDir) and all its modules."
                 + (count > 0 ? " \(count) of your projects use this version." : ""))
        }
    }
}

private struct EditorRow: View {
    let model: AppModel
    let editor: Editor

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "cube.fill")
                .font(.title2)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text("Unity \(editor.version)").font(.headline).monospacedDigit()
                    Text(editor.architecture == "x86_64" ? "Intel" : editor.architecture == "arm64" ? "Apple Silicon" : editor.architecture)
                        .font(.caption)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 1)
                        .background(.quaternary, in: Capsule())
                    if model.isBusy(editor) {
                        ProgressView().controlSize(.mini)
                    }
                }
                Text(editor.installDir)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                ModuleChips(editor: editor)
            }
            Spacer()
            Text(usage).foregroundStyle(.secondary)
            Button("Modules…") { model.modulesEditor = editor }
                .help("Add or remove platforms, language packs and documentation")
            Menu {
                menuItems
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
        }
        .padding(.vertical, 4)
        .contextMenu { menuItems }
    }

    @ViewBuilder
    private var menuItems: some View {
        Button("Modules…") { model.modulesEditor = editor }
        Button("Show in Finder") { model.revealInFinder(editor.appPath) }
        Divider()
        Button("Uninstall…", role: .destructive) { model.editorPendingUninstall = editor }
            .disabled(model.cli == nil || model.isBusy(editor))
    }

    private var usage: String {
        let count = model.projects.filter { $0.editorVersion == editor.version }.count
        return count == 1 ? "1 project" : "\(count) projects"
    }
}
