import SwiftUI
import UniHubCore

// Shared pieces for installing editors and adding/removing modules: the module picker,
// license terms, install progress and the platform chips on the Installs page.

extension Module: Identifiable {}

extension ModuleKind {
    var title: String {
        switch self {
        case .platform: "Platforms"
        case .languagePack: "Language packs"
        case .documentation: "Documentation"
        case .devTool: "Dev tools"
        case .other: "Other"
        }
    }

    /// Display order, platforms first.
    var order: Int {
        switch self {
        case .platform: 0
        case .languagePack: 1
        case .documentation: 2
        case .devTool: 3
        case .other: 4
        }
    }
}

extension Module {
    /// Short label for chips: "Android", "Mac IL2CPP", "Windows Server", "日本語".
    var shortName: String {
        if kind == .documentation { return "Docs" }
        var n = name
        for (from, to) in [(" Build Support", ""), (" Dedicated Server", " Server"), ("(", ""), (")", "")] {
            n = n.replacingOccurrences(of: from, with: to)
        }
        return n.trimmingCharacters(in: .whitespaces)
    }

    var symbol: String {
        switch kind {
        case .languagePack: return "character.bubble"
        case .documentation: return "book"
        case .devTool: return "hammer"
        case .other: return "puzzlepiece.extension"
        case .platform:
            let prefixes: [(String, String)] = [
                ("android", "smartphone"), ("ios", "iphone"), ("appletv", "appletv"),
                ("visionos", "visionpro"), ("webgl", "globe"), ("mac", "laptopcomputer"),
                ("windows", "pc"), ("linux", "server.rack"),
            ]
            return prefixes.first { id.hasPrefix($0.0) }?.1 ?? "cpu"
        }
    }
}

/// Visible modules as a two-level tree: top-level modules, each with its visible
/// children (e.g. Android → OpenJDK, Android SDK & NDK Tools). Hidden dependencies are
/// left out; the Unity CLI adds them.
struct ModuleTree {
    struct Node: Identifiable {
        let module: Module
        let children: [Module]
        var id: String { module.id }
    }

    let groups: [(kind: ModuleKind, nodes: [Node])]
    private let byId: [String: Module]

    init(_ modules: [Module]) {
        byId = Dictionary(modules.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let visible = modules.filter { !$0.hidden }
        let tops = visible.filter { $0.parentId == nil }
        let nodes = tops.map { top in Node(module: top, children: visible.filter { $0.parentId == top.id }) }
        groups = Dictionary(grouping: nodes, by: \.module.kind)
            .sorted { $0.key.order < $1.key.order }
            .map { (kind: $0.key, nodes: $0.value) }
    }

    func module(_ id: String) -> Module? { byId[id] }

    /// Ids of a module's visible children.
    func children(of id: String) -> [String] {
        groups.flatMap(\.nodes).first { $0.id == id }?.children.map(\.id) ?? []
    }

    /// Download size of the given modules plus their hidden dependencies.
    func downloadSize(of ids: Set<String>) -> UInt64 {
        var total: UInt64 = 0
        var seen = Set<String>()
        var stack = Array(ids)
        while let id = stack.popLast() {
            guard seen.insert(id).inserted, let m = byId[id] else { continue }
            total += m.download.downloadSize
            // Hidden children come along with their parent.
            stack += byId.values.filter { $0.parentId == id && $0.hidden }.map(\.id)
        }
        return total
    }

    /// Size on disk of the given modules plus their hidden dependencies.
    func installedSize(of ids: Set<String>) -> UInt64 {
        var total: UInt64 = 0
        var seen = Set<String>()
        var stack = Array(ids)
        while let id = stack.popLast() {
            guard seen.insert(id).inserted, let m = byId[id] else { continue }
            total += m.download.installedSize
            stack += byId.values.filter { $0.parentId == id && $0.hidden }.map(\.id)
        }
        return total
    }

    /// License terms of the given modules and of the hidden dependencies they pull in.
    func terms(of ids: Set<String>) -> [(module: Module, terms: LicenseTerms)] {
        var out: [(Module, LicenseTerms)] = []
        var seen = Set<String>()
        var stack = Array(ids).sorted()
        while let id = stack.popLast() {
            guard seen.insert(id).inserted, let m = byId[id] else { continue }
            out += m.terms.map { (m, $0) }
            stack += byId.values.filter { $0.parentId == id && $0.hidden }.map(\.id)
        }
        return out.sorted { $0.0.name < $1.0.name }
    }
}

/// Modules grouped by kind, children indented under their parent (like the Hub's
/// "Add modules" dialog). Modules not installed have a checkbox; installed ones show
/// "Installed" and, when `onRemove` is set and their files live inside the editor
/// folder, a Remove button.
///
/// Ticking a parent ticks its children that are not installed; ticking a child ticks its
/// parent unless the parent is installed; unticking a child unticks only that child.
struct ModulePicker: View {
    let tree: ModuleTree
    @Binding var selection: Set<String>
    var installed: Set<String> = []
    var onRemove: ((Module) -> Void)? = nil
    var disabled = false

    var body: some View {
        List {
            ForEach(tree.groups, id: \.kind) { group in
                Section(group.kind.title) {
                    ForEach(group.nodes) { node in
                        row(node.module, parent: nil)
                        ForEach(node.children) { child in
                            row(child, parent: node.module)
                        }
                    }
                }
            }
        }
        .listStyle(.inset)
        .disabled(disabled)
    }

    @ViewBuilder
    private func row(_ module: Module, parent: Module?) -> some View {
        let isInstalled = installed.contains(module.id)
        HStack(spacing: 8) {
            if isInstalled {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .frame(width: 16)
            } else {
                Toggle("", isOn: binding(module, parent: parent))
                    .toggleStyle(.checkbox)
                    .labelsHidden()
                    .frame(width: 16)
            }
            Image(systemName: module.symbol)
                .foregroundStyle(.secondary)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(module.name)
                    if isInstalled {
                        Text("Installed")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.green)
                    }
                    if !module.terms.isEmpty && !isInstalled {
                        Image(systemName: "doc.text")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .help("Has license terms to accept")
                    }
                }
                if !module.description.isEmpty, module.description != module.name {
                    Text(module.description)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if isInstalled {
                if let onRemove, module.isRemovable {
                    Button("Remove…") { onRemove(module) }
                        .controlSize(.small)
                }
            } else if module.download.downloadSize > 0 {
                Text(Format.bytes(module.download.downloadSize))
                    .font(.callout)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.leading, parent == nil ? 0 : 26)
        .contentShape(Rectangle())
    }

    private func binding(_ module: Module, parent: Module?) -> Binding<Bool> {
        Binding(
            get: { selection.contains(module.id) },
            set: { on in
                if on {
                    selection.insert(module.id)
                    if let parent, !installed.contains(parent.id) {
                        selection.insert(parent.id)
                    }
                    if parent == nil {
                        selection.formUnion(tree.children(of: module.id).filter { !installed.contains($0) })
                    }
                } else {
                    selection.remove(module.id)
                    if parent == nil {
                        selection.subtract(tree.children(of: module.id))
                    }
                }
            }
        )
    }
}

extension Module {
    /// Files live inside the editor folder, so the Unity CLI can remove them. Modules
    /// installed elsewhere (Visual Studio Code in /Applications) cannot be removed here.
    var isRemovable: Bool {
        destination?.hasPrefix("{UNITY_PATH}") ?? (kind != .devTool)
    }
}

/// License terms of the modules about to be installed: one checkbox per module, all
/// required before installing (acceptance is not remembered).
struct TermsBox: View {
    let terms: [(module: Module, terms: LicenseTerms)]
    @Binding var accepted: Set<String>

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("License terms").font(.callout.weight(.semibold))
            ForEach(Array(terms.enumerated()), id: \.offset) { _, item in
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 4) {
                        Text("\(item.module.name):").foregroundStyle(.secondary)
                        if let url = URL(string: item.terms.url), !item.terms.url.isEmpty {
                            Link(item.terms.label.isEmpty ? item.terms.url : item.terms.label, destination: url)
                        } else {
                            Text(item.terms.label)
                        }
                    }
                    .font(.caption)
                    .lineLimit(1)
                    Toggle(
                        "I have read and agree to the \(item.terms.label.isEmpty ? "license terms" : item.terms.label)",
                        isOn: Binding(
                            get: { accepted.contains(item.module.id) },
                            set: { if $0 { accepted.insert(item.module.id) } else { accepted.remove(item.module.id) } }
                        )
                    )
                    .toggleStyle(.checkbox)
                    .font(.caption)
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
    }
}

/// Free space on the volume holding `path` (or its nearest existing parent).
func availableSpace(at path: String) -> UInt64? {
    var url = URL(filePath: path)
    while !FileManager.default.fileExists(atPath: url.path(percentEncoded: false)), url.pathComponents.count > 1 {
        url.deleteLastPathComponent()
    }
    let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
    return values?.volumeAvailableCapacityForImportantUsage.map { UInt64(max($0, 0)) }
}

/// One install in progress (or just finished) with Cancel / Dismiss.
struct JobRow: View {
    let model: AppModel
    let job: Job

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
                .font(.title3)
                .foregroundStyle(color)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.callout.weight(.semibold))
                if job.state == .running {
                    if let percent = job.percent {
                        ProgressView(value: percent, total: 100)
                    } else {
                        ProgressView().progressViewStyle(.linear)
                    }
                }
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(job.state == .failed ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                    .lineLimit(2)
                    .textSelection(.enabled)
            }
            Spacer()
            if job.state == .running {
                Button("Cancel") { model.cancelJob(job) }
                    .controlSize(.small)
            } else {
                Button("Dismiss", systemImage: "xmark") { model.dismissJob(job) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
            }
        }
        .padding(.vertical, 4)
    }

    private var title: String {
        switch job.kind {
        case .installEditor:
            let arch = job.architecture == "x86_64" ? " (Intel)" : ""
            return "Installing Unity \(job.version)\(arch)"
        case .installModules:
            return "Adding modules to Unity \(job.version)"
        case .createProject:
            let name = job.projectPath.map { ($0 as NSString).lastPathComponent } ?? "project"
            return "Creating \(name) with Unity \(job.version)"
        }
    }

    private var detail: String {
        switch job.state {
        case .running:
            let step = job.percent.map { " — \(Int($0))%" } ?? ""
            return job.message + step
        case .succeeded:
            switch job.kind {
            case .installEditor: return "Installed."
            case .installModules: return "Modules added."
            case .createProject: return job.message
            }
        case .failed:
            return job.error ?? (job.kind == .createProject ? "The project could not be created." : "The install failed.")
        case .cancelled:
            return "Cancelled."
        }
    }

    private var icon: String {
        switch job.state {
        case .running: "arrow.down.circle"
        case .succeeded: "checkmark.circle.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .cancelled: "xmark.circle"
        }
    }

    private var color: Color {
        switch job.state {
        case .running: .accentColor
        case .succeeded: .green
        case .failed: .orange
        case .cancelled: .secondary
        }
    }
}

/// An editor's build platforms and language packs, as small chips. The Mac platform is
/// built into every editor (Mono); the rest are installed modules.
struct ModuleChips: View {
    let editor: Editor

    var body: some View {
        let shown = editor.modules
            .filter { $0.installed && $0.parentId == nil && !$0.hidden && $0.kind != .devTool }
            .sorted { ($0.kind.order, $0.name) < ($1.kind.order, $1.name) }
        HStack(spacing: 4) {
            if hasBuiltInMac {
                chip("Mac", symbol: "laptopcomputer", help: "Mac build support is built into the editor")
            }
            ForEach(shown) { module in
                chip(module.shortName, symbol: module.symbol, help: module.name)
            }
        }
    }

    private var hasBuiltInMac: Bool {
        let fm = FileManager.default
        return fm.fileExists(atPath: editor.appPath + "/Contents/PlaybackEngines/MacStandaloneSupport")
            || fm.fileExists(atPath: editor.installDir + "/PlaybackEngines/MacStandaloneSupport")
    }

    private func chip(_ text: String, symbol: String, help: String) -> some View {
        Label(text, systemImage: symbol)
            .font(.caption)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(.quaternary.opacity(0.7), in: Capsule())
            .help(help)
    }
}
