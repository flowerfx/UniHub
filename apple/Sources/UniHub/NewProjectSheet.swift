import SwiftUI
import UniHubCore

/// "New Project": pick an editor and a template, name the project and choose where it goes.
/// Creating runs in the background (downloading the template first if needed); the project
/// is listed at once and opens in Unity when the editor has made it.
struct NewProjectSheet: View {
    let model: AppModel
    @Environment(\.dismiss) private var dismiss

    /// Choices from last time, offered again.
    @AppStorage("newProjectEditor") private var savedEditor = ""
    @AppStorage("newProjectTemplate") private var savedTemplate = ""
    @AppStorage("newProjectLocation") private var savedLocation = ""

    @State private var editorID = ""
    @State private var catalog: TemplateCatalog?
    @State private var loadError: String?
    @State private var category: TemplateCategory = .all
    @State private var search = ""
    @State private var selectedID: String?
    @State private var name = ""
    /// The user typed a name: stop suggesting one when the location changes.
    @State private var nameEdited = false
    @State private var location = ""
    @State private var isChoosingLocation = false
    @State private var isStarting = false

    private var editor: Editor? { model.editors.first { $0.id == editorID } }

    private var templates: [Template] { catalog?.templates ?? [] }

    private var visibleTemplates: [Template] {
        let query = search.trimmingCharacters(in: .whitespaces)
        return templates.filter { template in
            category.includes(template)
                && (query.isEmpty
                    || template.displayName.localizedCaseInsensitiveContains(query)
                    || template.description.localizedCaseInsensitiveContains(query)
                    || template.id.localizedCaseInsensitiveContains(query))
        }
    }

    private var selected: Template? { templates.first { $0.id == selectedID } }

    /// The location as typed, with `~` expanded.
    private var folder: String { (location.trimmingCharacters(in: .whitespaces) as NSString).expandingTildeInPath }

    private var problem: String? { model.newProjectProblem(location: folder, name: name) }

    /// Why Create is disabled, other than the name or location.
    private var blocker: String? {
        guard editor != nil else { return "Install an editor first." }
        guard let selected else { return "Choose a template." }
        if !selected.isDownloaded && model.cli == nil {
            return "Downloading this template needs the Unity CLI. Install it from the Tools page, or pick a template on this Mac."
        }
        return nil
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if model.editors.isEmpty {
                ContentUnavailableView {
                    Label("No editors installed", systemImage: "shippingbox")
                } description: {
                    Text("A project is created with a Unity editor. Install one from the Releases page first.")
                } actions: {
                    Button("Go to Releases") {
                        UserDefaults.standard.set(Page.releases.rawValue, forKey: "page")
                        dismiss()
                    }
                }
                .frame(maxHeight: .infinity)
            } else {
                if let note = catalog?.note {
                    NoteBanner(text: note)
                }
                HStack(spacing: 0) {
                    categoryList
                        .frame(width: 170)
                    Divider()
                    templateList
                        .frame(minWidth: 300, maxWidth: .infinity)
                    Divider()
                    TemplateDetails(template: selected)
                        .frame(width: 300)
                }
                .frame(maxHeight: .infinity)
            }
            Divider()
            footer
        }
        .frame(width: 980, height: 660)
        .task { setUp() }
        // The sheet can open before the editor list has loaded (e.g. at launch).
        .onChange(of: model.editors.map(\.id)) { _, _ in pickEditor() }
        .task(id: editorID) { await loadTemplates() }
        .onChange(of: location) { _, _ in
            if !nameEdited { name = model.suggestProjectName(in: folder) }
        }
        .fileImporter(isPresented: $isChoosingLocation, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result { location = url.path(percentEncoded: false) }
        }
    }

    // MARK: Parts

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: "plus.rectangle.on.folder")
                .font(.system(size: 26))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 3) {
                Text("New Project").font(.title3.weight(.semibold))
                Text("Start from a template. Unity creates the project and opens it.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            if !model.editors.isEmpty {
                Picker("Editor version", selection: $editorID) {
                    ForEach(model.editors) { editor in
                        Text(editor.architecture == "x86_64" ? "\(editor.version) (Intel)" : editor.version)
                            .tag(editor.id)
                    }
                }
                .frame(width: 280)
            }
        }
        .padding(16)
    }

    private var categoryList: some View {
        List(selection: Binding<TemplateCategory?>(get: { category }, set: { category = $0 ?? .all })) {
            ForEach(TemplateCategory.allCases.filter { $0 == .all || templates.contains(where: $0.includes) }) { item in
                HStack {
                    Label(item.title, systemImage: item.symbol)
                    Spacer()
                    Text("\(templates.filter(item.includes).count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                .tag(item)
            }
        }
        .listStyle(.sidebar)
    }

    @ViewBuilder
    private var templateList: some View {
        VStack(spacing: 0) {
            TextField("Search templates", text: $search)
                .textFieldStyle(.roundedBorder)
                .padding(10)
            Divider()
            if catalog == nil && loadError == nil {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Loading templates…").font(.callout).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let loadError {
                ContentUnavailableView("Could not list the templates", systemImage: "exclamationmark.triangle",
                                       description: Text(loadError))
            } else if visibleTemplates.isEmpty {
                ContentUnavailableView(
                    templates.isEmpty ? "No templates" : "No matching templates",
                    systemImage: "square.dashed",
                    description: Text(templates.isEmpty
                                      ? "This editor has no project templates on this Mac."
                                      : "Try another search or category.")
                )
            } else {
                List(visibleTemplates, id: \.id, selection: $selectedID) { template in
                    TemplateRow(template: template)
                        .tag(template.id)
                }
                .listStyle(.inset)
            }
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                GridRow {
                    Text("Project name").gridColumnAlignment(.trailing)
                    TextField("Project name", text: Binding(get: { name }, set: { name = $0; nameEdited = true }))
                        .textFieldStyle(.roundedBorder)
                        .labelsHidden()
                }
                GridRow {
                    Text("Location")
                    HStack {
                        TextField("Location", text: $location)
                            .textFieldStyle(.roundedBorder)
                            .labelsHidden()
                        Button("Choose…") { isChoosingLocation = true }
                    }
                }
            }
            HStack(alignment: .center, spacing: 8) {
                if let message = problem ?? blocker {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .lineLimit(2)
                } else if let selected {
                    let path = (folder as NSString).appendingPathComponent(name.trimmingCharacters(in: .whitespaces))
                    Text(selected.isDownloaded
                         ? "Creates \(path)"
                         : "Downloads the template (\(Format.bytes(selected.downloadBytes))), then creates \(path)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create Project") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(problem != nil || blocker != nil || isStarting)
            }
        }
        .padding(16)
    }

    // MARK: Actions

    private func setUp() {
        pickEditor()
        let remembered = savedLocation.isEmpty ? nil : savedLocation
        location = remembered.flatMap { FileManager.default.fileExists(atPath: $0) ? $0 : nil }
            ?? model.defaultProjectLocation
        name = model.suggestProjectName(in: folder)
    }

    /// The editor used last time, else the newest one, unless one is picked already.
    private func pickEditor() {
        guard editor == nil else { return }
        editorID = model.editors.first { $0.id == savedEditor }?.id ?? model.editors.first?.id ?? ""
    }

    private func loadTemplates() async {
        guard let editor else { return }
        catalog = nil
        loadError = nil
        let result = await model.templates(for: editor)
        guard editor.id == editorID else { return }  // another editor was picked meanwhile
        switch result {
        case .success(let list):
            catalog = list
            if !list.templates.contains(where: { $0.id == selectedID }) {
                selectedID = list.templates.first { $0.id == savedTemplate }?.id
                    ?? list.templates.first { $0.kind == .core && $0.isDownloaded }?.id
                    ?? list.templates.first?.id
            }
            if !category.isEmpty(in: list.templates) { return }
            category = .all
        case .failure(let error):
            loadError = describe(error)
        }
    }

    private func create() {
        guard let editor, let selected else { return }
        isStarting = true
        let projectName = name.trimmingCharacters(in: .whitespaces)
        Task {
            let started = await model.createProject(location: folder, name: projectName, editor: editor, template: selected)
            isStarting = false
            if started {
                savedEditor = editor.id
                savedTemplate = selected.id
                savedLocation = folder
                dismiss()
            }
        }
    }
}

// MARK: - Categories

enum TemplateCategory: String, CaseIterable, Identifiable {
    case all, new, core, sample, learning, custom

    var id: Self { self }

    var title: String {
        switch self {
        case .all: "All templates"
        case .new: "New"
        case .core: "Core"
        case .sample: "Sample"
        case .learning: "Learning"
        case .custom: "Custom"
        }
    }

    var symbol: String {
        switch self {
        case .all: "square.grid.2x2"
        case .new: "star"
        case .core: "cube"
        case .sample: "sparkles"
        case .learning: "graduationcap"
        case .custom: "person.crop.square"
        }
    }

    func includes(_ template: Template) -> Bool {
        switch self {
        case .all: true
        case .new: template.isNew
        case .core: template.kind == .core
        case .sample: template.kind == .sample
        case .learning: template.kind == .learning
        case .custom: template.kind == .custom
        }
    }

    /// No template of the list is in this category (so it is not shown).
    func isEmpty(in templates: [Template]) -> Bool {
        self != .all && !templates.contains(where: includes)
    }
}

// MARK: - Rows and details

private struct TemplateRow: View {
    let template: Template

    var body: some View {
        HStack(spacing: 10) {
            TemplateIcon(template: template, size: 36)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(template.displayName).fontWeight(.medium).lineLimit(1)
                    if template.isNew {
                        Text("New")
                            .font(.caption2.weight(.semibold))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(.tint.opacity(0.2)))
                    }
                }
                Text([template.pipelineName, template.kindName].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            if !template.isDownloaded {
                Label(Format.bytes(template.downloadBytes), systemImage: "arrow.down.circle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .help("Downloaded when you create the project")
            }
        }
        .padding(.vertical, 3)
    }
}

private struct TemplateIcon: View {
    let template: Template
    let size: CGFloat

    var body: some View {
        Group {
            if let url = template.iconUrl.flatMap(URL.init(string:)) {
                AsyncImage(url: url) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit()
                    } else {
                        placeholder
                    }
                }
            } else {
                placeholder
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.2))
    }

    private var placeholder: some View {
        RoundedRectangle(cornerRadius: size * 0.2)
            .fill(.tint.opacity(0.15))
            .overlay {
                Image(systemName: template.symbol)
                    .font(.system(size: size * 0.45))
                    .foregroundStyle(.tint)
            }
    }
}

private struct TemplateDetails: View {
    let template: Template?

    var body: some View {
        if let template {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    preview(template)
                    Text(template.displayName).font(.title3.weight(.semibold))
                    if !template.description.isEmpty {
                        Text(template.description)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Divider()
                    Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 6) {
                        if let pipeline = template.pipelineName {
                            InfoRow(label: "Render pipeline", value: pipeline)
                        }
                        if let kind = template.kindName {
                            InfoRow(label: "Category", value: kind)
                        }
                        InfoRow(label: "Version", value: template.version)
                        InfoRow(label: "Template", value: template.isDownloaded
                                ? "On this Mac"
                                : "Download \(Format.bytes(template.downloadBytes))")
                        if !template.buildPlatforms.isEmpty {
                            InfoRow(label: "Platforms", value: template.buildPlatforms.map(platformName).joined(separator: ", "))
                        }
                    }
                    .font(.callout)
                    Text(template.id)
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                        .textSelection(.enabled)
                }
                .padding(16)
            }
        } else {
            ContentUnavailableView("No template selected", systemImage: "square.dashed")
        }
    }

    @ViewBuilder
    private func preview(_ template: Template) -> some View {
        let shape = RoundedRectangle(cornerRadius: 8)
        Group {
            if let url = template.previewImageUrl.flatMap(URL.init(string:)) {
                AsyncImage(url: url) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFill()
                    } else if phase.error != nil {
                        placeholder(template)
                    } else {
                        ZStack { placeholder(template); ProgressView().controlSize(.small) }
                    }
                }
            } else {
                placeholder(template)
            }
        }
        .frame(width: 268, height: 151)
        .clipShape(shape)
        .overlay(shape.strokeBorder(.quaternary))
    }

    private func placeholder(_ template: Template) -> some View {
        Rectangle()
            .fill(.tint.opacity(0.12))
            .overlay {
                Image(systemName: template.symbol)
                    .font(.system(size: 48))
                    .foregroundStyle(.tint)
            }
    }

    private func platformName(_ id: String) -> String {
        switch id.uppercased() {
        case "IOS": "iOS"
        case "TVOS": "tvOS"
        case "VISIONOS": "visionOS"
        case "MACOS": "macOS"
        case "WEBGL": "Web"
        case "ANDROID": "Android"
        case "WINDOWS": "Windows"
        case "LINUX": "Linux"
        default: id.capitalized
        }
    }
}

private struct InfoRow: View {
    let label: String
    let value: String

    var body: some View {
        GridRow {
            Text(label).foregroundStyle(.secondary).gridColumnAlignment(.leading)
            Text(value).fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct NoteBanner: View {
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "info.circle.fill").foregroundStyle(.blue)
            Text(text).font(.callout)
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.blue.opacity(0.1))
    }
}

extension Template {
    var isDownloaded: Bool { archivePath != nil }

    var pipelineName: String? {
        switch renderPipeline?.uppercased() {
        case "URP": "Universal (URP)"
        case "HDRP": "High Definition (HDRP)"
        case "BUILT_IN", "BUILTIN": "Built-in"
        case let other?: other.capitalized
        case nil: nil
        }
    }

    var kindName: String? {
        switch kind {
        case .core: "Core"
        case .sample: "Sample"
        case .learning: "Learning"
        case .custom: "Custom"
        }
    }

    /// Stand-in picture when Unity's icon is not available.
    var symbol: String {
        let text = (id + " " + displayName).lowercased()
        if text.contains("vr") || text.contains("mixed-reality") || text.contains("mixed reality") { return "visionpro" }
        if text.contains("ar-") || text.contains(" ar ") { return "arkit" }
        if text.contains("2d") { return "square.on.square" }
        if text.contains("multiplayer") { return "person.2" }
        if kind == .learning { return "graduationcap" }
        return "cube"
    }
}
