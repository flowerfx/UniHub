import SwiftUI
import UniHubCore

/// Inspector panel opened by a project's arrow button. It covers what the Hub shows
/// when a project is expanded: editor, render pipeline, platform, size, source control
/// and Unity Cloud.
struct ProjectDetailsView: View {
    // Passed explicitly, like the table cells, so it never depends on the environment.
    let model: AppModel

    var body: some View {
        Group {
            if let project = model.detailsProject {
                Form {
                    HeaderSection(model: model, project: project)
                    if project.accessDenied {
                        AccessSection(model: model, project: project)
                    }
                    ProjectSection(model: model, project: project, details: model.details)
                    SizeSection(model: model, project: project)
                    SourceControlSection(model: model, project: project, details: model.details)
                    if project.exists && !project.accessDenied {
                        CloudSection(model: model, project: project)
                    }
                    if let details = model.details, !details.fromUnityCli {
                        Section {
                            Label("Install the Unity CLI from Tools to also see the build target and scripting backend.",
                                  systemImage: "info.circle")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .formStyle(.grouped)
            } else {
                ContentUnavailableView(
                    "No project selected",
                    systemImage: "sidebar.right",
                    description: Text("Click the arrow on a project to see its details.")
                )
            }
        }
    }
}

// MARK: - Sections

private struct HeaderSection: View {
    let model: AppModel
    let project: Project

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "folder.fill")
                        .font(.system(size: 30))
                        .foregroundStyle(.tint)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(project.title).font(.title3.weight(.semibold))
                        Text(project.path)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .lineLimit(3)
                    }
                }
                HStack {
                    Button("Open") { Task { await model.open(project) } }
                        .buttonStyle(.borderedProminent)
                        .disabled(!project.exists || !model.isInstalled(project.editorVersion))
                    Menu("Open With") {
                        ForEach(model.editors) { editor in
                            Button(editor.version) { Task { await model.open(project, with: editor.version) } }
                        }
                    }
                    .fixedSize()
                    .disabled(!project.exists || model.editors.isEmpty)
                    Button("Show in Finder") { model.revealInFinder(project.path) }
                        .disabled(!project.exists)
                }
            }
            .padding(.vertical, 4)
        }
    }
}

/// macOS refused access to the project folder, so nothing below could be read.
private struct AccessSection: View {
    let model: AppModel
    let project: Project

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Label("UniHub cannot read this folder", systemImage: "lock.fill")
                    .font(.headline)
                    .foregroundStyle(.orange)
                Text("macOS blocked access to it. For a project on an external drive, turn on Removable Volumes (or Network Volumes) for UniHub in Files and Folders; for Documents, Desktop or Downloads, turn on that folder. Then press ⌘R to refresh.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button("Open Privacy Settings") {
                        model.openURL("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders")
                    }
                    Button("Refresh") {
                        Task { await model.refreshLocal() }
                        model.reloadDetails()
                    }
                }
            }
            .padding(.vertical, 4)
        }
    }
}

private struct ProjectSection: View {
    let model: AppModel
    let project: Project
    let details: ProjectDetails?

    var body: some View {
        Section("Project") {
            LabeledContent("Editor version") {
                HStack(spacing: 4) {
                    Text(project.editorVersion ?? "Unknown").monospacedDigit()
                    if let v = project.editorVersion, !model.isInstalled(v) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .help("Unity \(v) is not installed")
                    }
                }
            }
            if let details {
                Row("Render pipeline", pipelineName(details.renderPipeline))
                Row("Current platform", details.buildTarget)
                Row("Scripting backend", details.scriptingBackends.isEmpty ? nil : details.scriptingBackends.joined(separator: ", "))
                Row("Packages", details.packageCount > 0 ? "\(details.packageCount)" : nil)
                Row("Product name", details.productName)
                Row("Company", details.companyName)
                Row("Version", details.version)
            } else if model.isLoadingDetails {
                HStack { ProgressView().controlSize(.small); Text("Reading project…").foregroundStyle(.secondary) }
            }
            Row("Last modified", Format.relative(ms: project.modifiedMs))
            Row("Last opened in UniHub", project.lastOpenedMs.map { Format.relative(ms: $0) })
        }
    }

    private func pipelineName(_ value: String?) -> String? {
        switch value {
        case "Built-in": "Built-in"
        case "URP": "Universal (URP)"
        case "HDRP": "High Definition (HDRP)"
        case "SRP": "Scriptable (SRP)"
        default: value
        }
    }
}

private struct SizeSection: View {
    let model: AppModel
    let project: Project

    var body: some View {
        Section {
            if let size = model.detailsSize {
                LabeledContent("Total") {
                    Text(Format.bytes(size.total)).fontWeight(.semibold)
                }
                ForEach(size.folders.prefix(6), id: \.name) { folder in
                    VStack(alignment: .leading, spacing: 3) {
                        LabeledContent(folder.name, value: Format.bytes(folder.bytes))
                        ProgressView(value: Double(folder.bytes), total: Double(max(size.total, 1)))
                            .progressViewStyle(.linear)
                            .controlSize(.mini)
                    }
                }
            } else if model.isLoadingSize || model.isLoadingDetails {
                HStack { ProgressView().controlSize(.small); Text("Measuring…").foregroundStyle(.secondary) }
            } else {
                Text("Unavailable").foregroundStyle(.secondary)
            }
        } header: {
            HStack {
                Text("Size on disk")
                Spacer()
                Button("Clean Up…") { model.requestCleanup(of: project) }
                    .controlSize(.small)
                    .disabled(!project.exists)
                    .help("Free disk space: remove Library, Temp, Logs, generated .sln/.csproj files…")
            }
        }
    }
}

private struct SourceControlSection: View {
    let model: AppModel
    let project: Project
    let details: ProjectDetails?

    var body: some View {
        Section("Source control") {
            if let sc = details?.sourceControl ?? project.sourceControl {
                LabeledContent("Provider") {
                    Label(sc.provider.displayName, systemImage: sc.provider.symbol)
                }
                Row("Repository", sc.repositoryName)
                Row(sc.provider == .unityVersionControl ? "Branch" : "Current branch",
                    sc.branch.map { sc.detached ? "\($0) (detached)" : $0 })
                Row("Remote", sc.remoteUrl.map { url in sc.remoteName.map { "\($0): \(url)" } ?? url })
                Row("Folder in repository", sc.pathInRepository.isEmpty ? nil : sc.pathInRepository)

                if let git = details?.git {
                    if let subject = git.lastCommitSubject {
                        LabeledContent("Last commit") {
                            VStack(alignment: .trailing, spacing: 2) {
                                Text(subject).lineLimit(2).multilineTextAlignment(.trailing)
                                Text([git.lastCommitHash, git.lastCommitAuthor, Format.relative(iso: git.lastCommitDate)]
                                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    LabeledContent("Uncommitted changes") {
                        Text(git.changedFiles == 0 ? "None" : "\(git.changedFiles) files")
                            .foregroundStyle(git.changedFiles == 0 ? AnyShapeStyle(.secondary) : AnyShapeStyle(.orange))
                    }
                    if let upstream = git.upstream {
                        LabeledContent("Sync with \(upstream)") {
                            Text(syncText(ahead: git.ahead ?? 0, behind: git.behind ?? 0))
                        }
                    }
                } else if model.isLoadingDetails {
                    HStack { ProgressView().controlSize(.small); Text("Reading history…").foregroundStyle(.secondary) }
                }

                if let web = sc.webUrl {
                    HStack {
                        Button("View on \(sc.provider.displayName)") { model.openURL(web) }
                        Menu("More") {
                            if let branch = sc.branchUrl {
                                Button("Open Branch") { model.openURL(branch) }
                            }
                            if let reviews = sc.reviewsUrl {
                                Button(sc.provider.reviewsLabel) { model.openURL(reviews) }
                            }
                            if let commits = sc.commitsUrl {
                                Button("Commits") { model.openURL(commits) }
                            }
                            Divider()
                            Button("Copy Link") { model.copyToPasteboard(web) }
                            if let remote = sc.remoteUrl {
                                Button("Copy Clone URL") { model.copyToPasteboard(remote) }
                            }
                        }
                        .fixedSize()
                    }
                } else if let remote = sc.remoteUrl {
                    Button("Copy Remote URL") { model.copyToPasteboard(remote) }
                }
            } else {
                Text("This project is not in a Git, Unity Version Control or Perforce workspace.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func syncText(ahead: UInt32, behind: UInt32) -> String {
        switch (ahead, behind) {
        case (0, 0): "Up to date"
        case (let a, 0): "\(a) to push"
        case (0, let b): "\(b) to pull"
        case let (a, b): "\(a) to push · \(b) to pull"
        }
    }
}

/// Connection to a Unity Cloud project, read from the project's settings, with Connect
/// and Disconnect like the Hub's project details.
private struct CloudSection: View {
    let model: AppModel
    let project: Project

    var body: some View {
        let cloud = project.cloud
        let busy = model.disconnectingCloud.contains(project.path)
        Section {
            LabeledContent("Status") {
                if busy {
                    HStack(spacing: 6) { ProgressView().controlSize(.small); Text("Disconnecting…") }
                } else if cloud.isConnected {
                    Label("Connected", systemImage: "checkmark.icloud.fill").foregroundStyle(.green)
                } else {
                    Label("Not connected", systemImage: "icloud.slash").foregroundStyle(.secondary)
                }
            }
            if cloud.isConnected {
                LabeledContent("Organization") {
                    let stored = cloud.organizationId ?? ""
                    Text(model.cloudOrganizationName(for: stored) ?? stored)
                        .textSelection(.enabled)
                        .help("Stored as \(stored)")
                }
                Row("Cloud project", cloud.projectName)
                if let id = cloud.cloudProjectId {
                    LabeledContent("Project ID") {
                        HStack(spacing: 4) {
                            Text(id).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1)
                            Button("Copy", systemImage: "doc.on.doc") { model.copyToPasteboard(id) }
                                .labelStyle(.iconOnly)
                                .buttonStyle(.borderless)
                        }
                    }
                }
            } else {
                Text("Connect the project to a Unity Cloud project to use Build Automation, Analytics, Version Control and other services.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } header: {
            HStack {
                Text("Unity Cloud")
                Spacer()
                if cloud.isConnected {
                    Button("Disconnect…") { model.requestCloudDisconnect(of: project) }
                        .controlSize(.small)
                        .disabled(busy)
                        .help("Stop using Unity Cloud services in this project")
                } else {
                    Button("Connect…") { model.requestCloudConnect(for: project) }
                        .controlSize(.small)
                        .help("Connect this project to a Unity Cloud project")
                }
            }
        }
        .task(id: cloud.organizationId) {
            if cloud.isConnected { await model.loadCloudOrganizationNamesIfNeeded() }
        }
    }
}

/// A labelled value that hides itself when there is nothing to show.
private struct Row: View {
    let label: String
    let value: String?

    init(_ label: String, _ value: String?) {
        self.label = label
        self.value = value
    }

    var body: some View {
        if let value, !value.isEmpty {
            LabeledContent(label) {
                Text(value).multilineTextAlignment(.trailing).textSelection(.enabled)
            }
        }
    }
}
