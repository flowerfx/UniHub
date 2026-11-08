import SwiftUI
import UniHubCore

/// "Modules" sheet of an installed editor: add platforms, language packs and
/// documentation, or remove installed ones. Installs run in the background through the
/// Unity CLI; progress shows on the Installs page.
struct EditorModulesSheet: View {
    let model: AppModel
    let editor: Editor
    @Environment(\.dismiss) private var dismiss

    @State private var modules: [Module]?
    @State private var loadError: String?
    @State private var selection: Set<String> = []
    @State private var acceptedTerms: Set<String> = []
    @State private var pendingRemoval: Module?
    @State private var isWorking = false

    var body: some View {
        let tree = ModuleTree(modules ?? [])
        let installed = Set((modules ?? []).filter(\.installed).map(\.id))
        let terms = tree.terms(of: selection)
        VStack(spacing: 0) {
            SheetHeader(
                symbol: "puzzlepiece.extension",
                title: "Modules of Unity \(editor.version)",
                subtitle: editor.installDir
            )
            Divider()
            if model.cli == nil {
                CliMissingNote(model: model)
            }
            if let job = model.runningJob(version: editor.version) {
                JobRow(model: model, job: job).padding(.horizontal, 16)
                Divider()
            }
            Group {
                if let modules, !modules.isEmpty {
                    ModulePicker(
                        tree: tree,
                        selection: $selection,
                        installed: installed,
                        onRemove: model.cli == nil || model.isBusy(editor) ? nil : { pendingRemoval = $0 },
                        disabled: isWorking || model.isBusy(editor)
                    )
                } else if let loadError {
                    ContentUnavailableView("Could not read the modules", systemImage: "exclamationmark.triangle",
                                           description: Text(loadError))
                } else if modules != nil {
                    ContentUnavailableView("No modules", systemImage: "puzzlepiece.extension",
                                           description: Text("Unity's release list has no modules for this version."))
                } else {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(maxHeight: .infinity)
            Divider()
            VStack(spacing: 10) {
                if !terms.isEmpty {
                    TermsBox(terms: terms, accepted: $acceptedTerms)
                }
                HStack {
                    SpaceSummary(
                        download: tree.downloadSize(of: selection),
                        onDisk: tree.installedSize(of: selection),
                        folder: editor.installDir,
                        count: selection.count
                    )
                    Spacer()
                    Button("Close") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                    Button("Install") { install(acceptTerms: !terms.isEmpty) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(selection.isEmpty || isWorking || model.cli == nil || model.isBusy(editor)
                                  || !terms.allSatisfy { acceptedTerms.contains($0.module.id) }
                                  || !SpaceSummary.fits(tree.installedSize(of: selection), at: editor.installDir))
                }
            }
            .padding(16)
        }
        .frame(width: 640, height: 500)
        .task { await load() }
        .confirmationDialog(
            "Remove \(pendingRemoval?.name ?? "") from Unity \(editor.version)?",
            isPresented: Binding(get: { pendingRemoval != nil }, set: { if !$0 { pendingRemoval = nil } }),
            presenting: pendingRemoval
        ) { module in
            Button("Remove", role: .destructive) { remove(module, installed: installed, tree: tree) }
        } message: { module in
            let children = tree.children(of: module.id).filter(installed.contains)
            Text(children.isEmpty
                 ? "Its files are deleted from the editor folder. You can add it again later."
                 : "Its files and its parts (\(children.compactMap { tree.module($0)?.name }.joined(separator: ", "))) are deleted from the editor folder.")
        }
    }

    private func load() async {
        switch await model.modules(of: editor) {
        case .success(let list): modules = list
        case .failure(let error): loadError = describe(error)
        }
    }

    private func install(acceptTerms: Bool) {
        isWorking = true
        Task {
            let started = await model.changeModules(of: editor, add: Array(selection), remove: [], acceptTerms: acceptTerms)
            isWorking = false
            if started { dismiss() }
        }
    }

    private func remove(_ module: Module, installed: Set<String>, tree: ModuleTree) {
        let ids = [module.id] + tree.children(of: module.id).filter(installed.contains)
        isWorking = true
        Task {
            _ = await model.changeModules(of: editor, add: [], remove: ids, acceptTerms: false)
            selection.subtract(ids)
            await load()
            isWorking = false
        }
    }
}

/// "Install" sheet of a release: pick modules, accept their terms, start the install.
struct InstallEditorSheet: View {
    let model: AppModel
    let release: Release
    @Environment(\.dismiss) private var dismiss

    @State private var selection: Set<String> = []
    @State private var acceptedTerms: Set<String> = []
    @State private var isStarting = false

    /// Top-level module ids picked last time, to pre-select them next time.
    private static let lastModulesKey = "lastInstallModules"

    var body: some View {
        let tree = ModuleTree(release.modules)
        let terms = tree.terms(of: selection)
        let editorSize = release.editor?.installedSize ?? 0
        let editorDownload = release.editor?.downloadSize ?? 0
        VStack(spacing: 0) {
            SheetHeader(
                symbol: "arrow.down.circle",
                title: "Install Unity \(release.version)",
                subtitle: subtitle
            )
            Divider()
            if model.cli == nil {
                CliMissingNote(model: model)
            }
            if release.architecture == "x86_64" {
                Label("No Apple Silicon build exists for this version. The Intel build runs with Rosetta 2.",
                      systemImage: "info.circle")
                    .font(.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                Divider()
            }
            if release.modules.isEmpty {
                ContentUnavailableView("No modules", systemImage: "puzzlepiece.extension",
                                       description: Text("Only the editor will be installed."))
                    .frame(maxHeight: .infinity)
            } else {
                ModulePicker(tree: tree, selection: $selection, disabled: isStarting)
                    .frame(maxHeight: .infinity)
            }
            Divider()
            VStack(spacing: 10) {
                if !terms.isEmpty {
                    TermsBox(terms: terms, accepted: $acceptedTerms)
                }
                HStack {
                    SpaceSummary(
                        download: editorDownload + tree.downloadSize(of: selection),
                        onDisk: editorSize + tree.installedSize(of: selection),
                        folder: model.editorInstallDir,
                        count: nil
                    )
                    Spacer()
                    Button("Cancel") { dismiss() }
                        .keyboardShortcut(.cancelAction)
                    Button("Install") { start(acceptTerms: !terms.isEmpty) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(isStarting || model.cli == nil
                                  || !terms.allSatisfy { acceptedTerms.contains($0.module.id) }
                                  || !SpaceSummary.fits(editorSize + tree.installedSize(of: selection),
                                                        at: model.editorInstallDir))
                }
            }
            .padding(16)
        }
        .frame(width: 640, height: 540)
        .onAppear { selection = initialSelection(tree) }
    }

    private var subtitle: String {
        var parts = [release.stream.capitalized, Format.day(iso: release.releaseDate)]
        parts.append(release.architecture == "x86_64" ? "Intel" : "Apple Silicon")
        parts.append("into \(model.editorInstallDir)")
        return parts.joined(separator: " · ")
    }

    /// Modules picked last time, else the platforms of the newest installed editor, plus
    /// the release's pre-selected modules; children come along with their parent.
    private func initialSelection(_ tree: ModuleTree) -> Set<String> {
        let available = Set(release.modules.filter { !$0.hidden }.map(\.id))
        var picked = Set(UserDefaults.standard.stringArray(forKey: Self.lastModulesKey) ?? [])
        if picked.isEmpty, let newest = model.editors.first {
            picked = Set(newest.modules.filter { $0.installed && $0.parentId == nil && $0.kind != .devTool }.map(\.id))
        }
        picked.formUnion(release.modules.filter { $0.preSelected && $0.parentId == nil }.map(\.id))
        picked.formIntersection(available)
        for id in picked {
            picked.formUnion(tree.children(of: id))
        }
        return picked
    }

    private func start(acceptTerms: Bool) {
        isStarting = true
        let tops = selection.filter { tree(for: $0)?.parentId == nil }
        UserDefaults.standard.set(Array(tops).sorted(), forKey: Self.lastModulesKey)
        Task {
            let started = await model.installEditor(release, modules: Array(selection), acceptTerms: acceptTerms)
            isStarting = false
            if started { dismiss() }
        }
    }

    private func tree(for id: String) -> Module? {
        release.modules.first { $0.id == id }
    }
}

// MARK: - Pieces

private struct SheetHeader: View {
    let symbol: String
    let title: String
    let subtitle: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 26))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.title3.weight(.semibold))
                Text(subtitle)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
    }
}

/// "Download 1.2 GB · 3.4 GB on disk · 120 GB available".
private struct SpaceSummary: View {
    let download: UInt64
    let onDisk: UInt64
    let folder: String
    let count: Int?

    var body: some View {
        let free = availableSpace(at: folder)
        VStack(alignment: .leading, spacing: 2) {
            Text(count.map { $0 == 0 ? "Nothing selected" : "Download \(Format.bytes(download))" }
                 ?? "Download \(Format.bytes(download))")
                .fontWeight(.semibold)
                .monospacedDigit()
            HStack(spacing: 4) {
                Text("\(Format.bytes(onDisk)) on disk")
                if let free {
                    Text("· \(Format.bytes(free)) available")
                        .foregroundStyle(free < onDisk ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .monospacedDigit()
        }
    }

    /// False only when the free space is known and too small.
    static func fits(_ bytes: UInt64, at folder: String) -> Bool {
        guard let free = availableSpace(at: folder) else { return true }
        return bytes <= free
    }
}

private struct CliMissingNote: View {
    let model: AppModel

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            Text("UniHub installs editors and modules with the official Unity CLI. Install it from the Tools page first.")
                .font(.callout)
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.orange.opacity(0.12))
    }
}
