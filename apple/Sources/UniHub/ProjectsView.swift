import UniHubCore
import SwiftUI

struct ProjectsView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var selection: Set<Project.ID> = []
    @State private var sortOrder = [KeyPathComparator(\Project.modifiedKey, order: .reverse)]

    private var rows: [Project] {
        let query = search.trimmingCharacters(in: .whitespaces)
        let filtered = query.isEmpty
            ? model.projects
            : model.projects.filter {
                $0.title.localizedCaseInsensitiveContains(query)
                    || $0.path.localizedCaseInsensitiveContains(query)
                    || ($0.editorVersion ?? "").contains(query)
            }
        return filtered.sorted(using: sortOrder)
    }

    var body: some View {
        @Bindable var model = model
        Table(rows, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("", value: \.favoriteKey) { project in
                Button {
                    Task { await model.toggleFavorite(project) }
                } label: {
                    Image(systemName: project.isFavorite ? "star.fill" : "star")
                        .foregroundStyle(project.isFavorite ? .yellow : .secondary)
                }
                .buttonStyle(.plain)
                .help(project.isFavorite ? "Remove from favorites" : "Add to favorites")
            }
            .width(24)

            TableColumn("Name", value: \.title) { project in
                VStack(alignment: .leading, spacing: 2) {
                    Text(project.title).fontWeight(.medium)
                    Text(project.path)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .opacity(project.exists ? 1 : 0.45)
            }
            .width(min: 220, ideal: 300)

            TableColumn("Editor", value: \.versionKey) { project in
                EditorVersionCell(model: model, project: project)
            }
            .width(min: 110, ideal: 130)

            TableColumn("Modified", value: \.modifiedKey) { project in
                Text(Format.relative(ms: project.modifiedMs)).foregroundStyle(.secondary)
            }
            .width(min: 100, ideal: 120)
        }
        .contextMenu(forSelectionType: Project.ID.self) { ids in
            if let project = model.projects.first(where: { ids.contains($0.id) }), ids.count == 1 {
                ProjectMenu(model: model, project: project)
            }
        } primaryAction: { ids in
            for project in model.projects where ids.contains(project.id) {
                Task { await model.open(project) }
            }
        }
        .searchable(text: $search, prompt: "Search projects")
        .confirmationDialog(
            "Remove \(model.projectPendingRemoval?.title ?? "") from the list?",
            isPresented: Binding(
                get: { model.projectPendingRemoval != nil },
                set: { if !$0 { model.projectPendingRemoval = nil } }
            ),
            presenting: model.projectPendingRemoval
        ) { project in
            Button("Remove", role: .destructive) { Task { await model.remove(project) } }
        } message: { _ in
            Text("The project folder stays on disk.")
        }
        .navigationTitle("Projects")
        .navigationSubtitle("\(model.projects.count) projects")
        .toolbar {
            ToolbarItemGroup {
                Button("Import from Unity Hub", systemImage: "square.and.arrow.down") {
                    Task { await model.importFromHub() }
                }
                .help("Add projects listed in the official Hub")
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task { await model.refreshLocal() }
                }
                Button("Add", systemImage: "plus") { model.isAddingProject = true }
                    .help("Add an existing project folder")
            }
        }
        .fileImporter(isPresented: $model.isAddingProject, allowedContentTypes: [.folder]) { result in
            switch result {
            case .success(let url): Task { await model.addProject(at: url) }
            case .failure(let error): model.errorMessage = describe(error)
            }
        }
        .overlay {
            if model.projects.isEmpty {
                ContentUnavailableView(
                    "No projects",
                    systemImage: "folder.badge.plus",
                    description: Text("Add a project folder, or import your list from Unity Hub.")
                )
            }
        }
    }
}

private struct EditorVersionCell: View {
    // Passed explicitly: table cells and context menus do not inherit the environment.
    let model: AppModel
    let project: Project

    var body: some View {
        if let version = project.editorVersion {
            HStack(spacing: 4) {
                Text(version).monospacedDigit()
                if !model.isInstalled(version) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help("Unity \(version) is not installed")
                }
            }
        } else {
            Text("—").foregroundStyle(.secondary)
        }
    }
}

private struct ProjectMenu: View {
    // Passed explicitly: table cells and context menus do not inherit the environment.
    let model: AppModel
    let project: Project

    var body: some View {
        Button("Open") { Task { await model.open(project) } }
            .disabled(!project.exists || !model.isInstalled(project.editorVersion))
        Menu("Open With") {
            ForEach(model.editors) { editor in
                Button(editor.version) { Task { await model.open(project, with: editor.version) } }
            }
        }
        .disabled(!project.exists || model.editors.isEmpty)
        Divider()
        Button("Show in Finder") { model.revealInFinder(project.path) }
            .disabled(!project.exists)
        Button(project.isFavorite ? "Remove from Favorites" : "Add to Favorites") {
            Task { await model.toggleFavorite(project) }
        }
        Divider()
        Button("Remove from List…", role: .destructive) { model.requestRemoval(of: project) }
    }
}
