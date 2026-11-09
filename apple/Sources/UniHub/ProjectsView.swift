import UniHubCore
import SwiftUI

struct ProjectsView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var selection: Set<Project.ID> = []
    @State private var sortOrder = [KeyPathComparator(\Project.modifiedKey, order: .reverse)]
    /// Project whose details were open, restored on the next launch.
    /// `-detailsProject <path>` on the command line overrides it.
    @AppStorage("detailsProject") private var savedDetailsPath = ""
    @State private var didOpenLaunchCleanup = false

    private var removalTitle: String {
        let pending = model.projectsPendingRemoval
        return pending.count == 1
            ? "Remove \(pending[0].title) from the list?"
            : "Remove \(pending.count) projects from the list?"
    }

    private var rows: [Project] {
        let query = search.trimmingCharacters(in: .whitespaces)
        let filtered = query.isEmpty
            ? model.projects
            : model.projects.filter {
                $0.title.localizedCaseInsensitiveContains(query)
                    || $0.path.localizedCaseInsensitiveContains(query)
                    || ($0.editorVersion ?? "").contains(query)
                    || $0.branchKey.localizedCaseInsensitiveContains(query)
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

            TableColumn("Source Control", value: \.branchKey) { project in
                SourceControlCell(project: project)
            }
            .width(min: 130, ideal: 190)

            TableColumn("Modified", value: \.modifiedKey) { project in
                Text(Format.relative(ms: project.modifiedMs)).foregroundStyle(.secondary)
            }
            .width(min: 90, ideal: 110)

            // The arrow that opens the project's details, like the Hub's expand button.
            TableColumn("") { project in
                DetailsButton(model: model, project: project)
            }
            .width(28)
        }
        // Projects being created, with their progress, above the list.
        .safeAreaInset(edge: .top, spacing: 0) {
            if !model.createJobs.isEmpty {
                VStack(spacing: 0) {
                    ForEach(model.createJobs, id: \.id) { job in
                        JobRow(model: model, job: job)
                            .padding(.horizontal, 12)
                    }
                    Divider()
                }
                .background(.bar)
            }
        }
        .inspector(isPresented: $model.showDetails) {
            ProjectDetailsView(model: model)
                .inspectorColumnWidth(min: 300, ideal: 360, max: 520)
        }
        .onChange(of: model.projects.count) { _, _ in
            // Reopen the saved panel once the project list has loaded.
            if !model.showDetails, !savedDetailsPath.isEmpty,
               let project = model.projects.first(where: { $0.path == savedDetailsPath }) {
                selection = [project.id]
                model.showDetails(for: project)
            }
            // Development aid: `-cleanupProject <path>` opens Clean Up once at launch.
            if !didOpenLaunchCleanup, let path = UserDefaults.standard.string(forKey: "cleanupProject"),
               let project = model.projects.first(where: { $0.path == path }) {
                didOpenLaunchCleanup = true
                model.requestCleanup(of: project)
            }
        }
        .onChange(of: model.showDetails) { _, open in
            savedDetailsPath = open ? (model.detailsPath ?? "") : ""
        }
        .onChange(of: model.detailsPath) { _, path in
            if model.showDetails { savedDetailsPath = path ?? "" }
        }
        .onChange(of: selection) { _, ids in
            // While the panel is open it follows the selected row.
            if model.showDetails, ids.count == 1, let project = model.projects.first(where: { ids.contains($0.id) }) {
                model.showDetails(for: project)
            }
        }
        .contextMenu(forSelectionType: Project.ID.self) { ids in
            if let project = model.projects.first(where: { ids.contains($0.id) }), ids.count == 1 {
                ProjectMenu(model: model, project: project)
            } else if ids.count > 1 {
                Button("Remove \(ids.count) Projects from List…", role: .destructive) {
                    model.requestRemoval(of: model.projects.filter { ids.contains($0.id) })
                }
            }
        } primaryAction: { ids in
            for project in model.projects where ids.contains(project.id) {
                Task { await model.open(project) }
            }
        }
        // Delete key / Edit → Delete: remove the selected projects from the list.
        .onDeleteCommand {
            model.requestRemoval(of: model.projects.filter { selection.contains($0.id) })
        }
        .searchable(text: $search, prompt: "Search projects")
        .confirmationDialog(
            removalTitle,
            isPresented: Binding(
                get: { !model.projectsPendingRemoval.isEmpty },
                set: { if !$0 { model.projectsPendingRemoval = [] } }
            ),
            presenting: model.projectsPendingRemoval
        ) { projects in
            Button("Remove", role: .destructive) { Task { await model.remove(projects) } }
        } message: { projects in
            Text(projects.count == 1 ? "The project folder stays on disk." : "The project folders stay on disk.")
        }
        .navigationTitle("Projects")
        .navigationSubtitle("\(model.projects.count) projects")
        .toolbar {
            ToolbarItemGroup {
                Button("New Project", systemImage: "plus.rectangle.on.folder") { model.isCreatingProject = true }
                    .labelStyle(.titleAndIcon)
                    .help("Create a project from a template (⌘N)")
                Button("Import from Unity Hub", systemImage: "square.and.arrow.down") {
                    Task { await model.importFromHub() }
                }
                .help("Add projects listed in the official Hub")
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task { await model.refreshLocal() }
                    if model.showDetails { model.reloadDetails() }
                }
                Button("Add", systemImage: "plus") { model.isAddingProject = true }
                    .help("Add an existing project folder")
                Button("Details", systemImage: "sidebar.right") {
                    if model.showDetails {
                        model.showDetails = false
                    } else if let project = model.projects.first(where: { selection.contains($0.id) }) {
                        model.showDetails(for: project)
                    } else {
                        model.showDetails = true
                    }
                }
                .help("Show or hide project details")
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
                ContentUnavailableView {
                    Label("No projects", systemImage: "folder.badge.plus")
                } description: {
                    Text("Create a project from a template, add a project folder, or import your list from Unity Hub.")
                } actions: {
                    Button("New Project…") { model.isCreatingProject = true }
                    Button("Add Project…") { model.isAddingProject = true }
                }
            }
        }
    }
}

private struct EditorVersionCell: View {
    // Passed explicitly: table cells and context menus do not inherit the environment.
    let model: AppModel
    let project: Project

    var body: some View {
        if model.creatingJob(path: project.path) != nil {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Creating…").foregroundStyle(.secondary)
            }
        } else if let version = project.editorVersion {
            HStack(spacing: 4) {
                Text(version).monospacedDigit()
                if !model.isInstalled(version) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help("Unity \(version) is not installed")
                }
            }
        } else if project.accessDenied {
            Label("No access", systemImage: "lock.fill")
                .foregroundStyle(.orange)
                .help("macOS does not let UniHub read this folder. Allow it in System Settings → Privacy & Security → Files and Folders.")
        } else {
            Text("—").foregroundStyle(.secondary)
        }
    }
}

private struct SourceControlCell: View {
    let project: Project

    var body: some View {
        if let sc = project.sourceControl {
            HStack(spacing: 6) {
                Image(systemName: sc.provider.symbol)
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 1) {
                    Text(sc.branch ?? "—").lineLimit(1).truncationMode(.middle)
                    Text(sc.provider.displayName).font(.caption).foregroundStyle(.secondary)
                }
            }
            .help(sc.repositoryName.map { "\(sc.provider.displayName): \($0)" } ?? sc.provider.displayName)
        } else {
            Text("—").foregroundStyle(.tertiary)
        }
    }
}

private struct DetailsButton: View {
    let model: AppModel
    let project: Project

    var body: some View {
        let isOpen = model.showDetails && model.detailsPath == project.path
        Button {
            model.toggleDetails(for: project)
        } label: {
            Image(systemName: isOpen ? "chevron.left.circle.fill" : "chevron.right.circle")
                .font(.title3)
                .foregroundStyle(isOpen ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
        }
        .buttonStyle(.plain)
        .help(isOpen ? "Hide project details" : "Show project details")
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
        Button("Show Details") { model.showDetails(for: project) }
        Button("Show in Finder") { model.revealInFinder(project.path) }
            .disabled(!project.exists)
        if let sc = project.sourceControl, let web = sc.webUrl {
            Divider()
            Button("View on \(sc.provider.displayName)") { model.openURL(web) }
            Button("Copy Repository Link") { model.copyToPasteboard(web) }
        }
        Button(project.isFavorite ? "Remove from Favorites" : "Add to Favorites") {
            Task { await model.toggleFavorite(project) }
        }
        Divider()
        Button("Clean Up…") { model.requestCleanup(of: project) }
            .disabled(!project.exists)
            .help("Free disk space: remove Library, Temp, Logs, generated .sln/.csproj files…")
        Button("Remove from List…", role: .destructive) { model.requestRemoval(of: project) }
    }
}
