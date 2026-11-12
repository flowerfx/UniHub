import SwiftUI
import UniHubCore

/// "Connect to Unity Cloud" sheet: pick one of the account's organizations, then one of
/// its cloud projects. Connecting writes the cloud project into the project's settings,
/// as Unity Hub does; nothing changes on Unity Cloud.
struct CloudConnectSheet: View {
    // Passed explicitly: sheets do not reliably inherit the environment.
    let model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            if let connect = model.cloudConnect {
                header(connect)
                Divider()
                content(connect)
                Divider()
                footer(connect)
            }
        }
        // Fits the main window's minimum height (520).
        .frame(width: 560, height: 470)
    }

    // MARK: Parts

    private func header(_ connect: CloudConnect) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "cloud")
                .font(.system(size: 26))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 4) {
                Text("Connect “\(connect.project.title)” to Unity Cloud").font(.title3.weight(.semibold))
                Text("Choose the Unity Cloud project to use for services such as Build Automation, Analytics and Version Control. UniHub saves it in the project’s settings, like Unity Hub; nothing changes on Unity Cloud.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
    }

    @ViewBuilder
    private func content(_ connect: CloudConnect) -> some View {
        if model.cli == nil {
            ContentUnavailableView(
                "Unity CLI needed",
                systemImage: "wrench.and.screwdriver",
                description: Text("UniHub lists your organizations and cloud projects with the Unity CLI. Install it from the Tools page, then sign in.")
            )
        } else if model.auth?.signedIn == false {
            ContentUnavailableView {
                Label("Sign in to Unity", systemImage: "person.crop.circle")
            } description: {
                Text("Sign in to see the cloud projects of your organizations.")
            } actions: {
                if model.isSigningIn {
                    ProgressView().controlSize(.small)
                } else {
                    Button("Sign In") {
                        Task {
                            await model.signIn()
                            await model.loadCloudOrganizations()
                        }
                    }
                }
            }
        } else if connect.isLoadingOrganizations && connect.organizations.isEmpty {
            loading("Loading organizations…")
        } else if connect.organizations.isEmpty, let error = connect.error {
            ContentUnavailableView {
                Label("Could not load your organizations", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                Button("Try Again") { Task { await model.loadCloudOrganizations() } }
            }
        } else {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Picker("Organization", selection: organizationBinding) {
                        ForEach(connect.organizations) { org in
                            Text(org.name).tag(Optional(org.id))
                        }
                    }
                    .fixedSize()
                    Spacer()
                    TextField("Search", text: searchBinding)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 180)
                }
                projectList(connect)
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func projectList(_ connect: CloudConnect) -> some View {
        if connect.isLoadingProjects {
            loading("Loading cloud projects…")
        } else if connect.projects.isEmpty, connect.error == nil {
            ContentUnavailableView {
                Label("No cloud projects", systemImage: "cloud")
            } description: {
                Text("This organization has no Unity Cloud projects yet. Create one on the Unity Dashboard, or pick another organization.")
            } actions: {
                Button("Open Unity Dashboard") { model.openURL(Self.dashboard) }
                Button("Reload") { Task { await model.loadCloudProjects() } }
            }
        } else {
            List(connect.visibleProjects, selection: projectBinding) { project in
                VStack(alignment: .leading, spacing: 2) {
                    Text(project.name).fontWeight(.medium)
                    Text(project.id).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                .padding(.vertical, 2)
                .tag(project.id)
            }
            .listStyle(.bordered(alternatesRowBackgrounds: true))
            .overlay {
                if connect.visibleProjects.isEmpty, !connect.projects.isEmpty {
                    Text("No cloud project matches “\(connect.search)”.").foregroundStyle(.secondary)
                }
            }
        }
    }

    private func footer(_ connect: CloudConnect) -> some View {
        HStack(spacing: 10) {
            if let error = connect.error, !connect.organizations.isEmpty {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .lineLimit(3)
            } else {
                Button("Unity Dashboard") { model.openURL(Self.dashboard) }
                    .buttonStyle(.link)
                    .help("Create or manage cloud projects in your browser")
            }
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .disabled(connect.isConnecting)
            if connect.isConnecting {
                ProgressView().controlSize(.small)
            }
            Button("Connect") { Task { await model.connectCloud() } }
                .keyboardShortcut(.defaultAction)
                .disabled(connect.selectedProject == nil || connect.isConnecting)
        }
        .padding(16)
    }

    private func loading(_ text: String) -> some View {
        VStack(spacing: 10) {
            ProgressView()
            Text(text).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: Bindings

    private var organizationBinding: Binding<String?> {
        Binding(
            get: { model.cloudConnect?.organizationId },
            set: { id in if let id { Task { await model.selectCloudOrganization(id) } } }
        )
    }

    private var searchBinding: Binding<String> {
        Binding(get: { model.cloudConnect?.search ?? "" }, set: { model.cloudConnect?.search = $0 })
    }

    private var projectBinding: Binding<String?> {
        Binding(get: { model.cloudConnect?.selectedProjectId }, set: { model.cloudConnect?.selectedProjectId = $0 })
    }

    static let dashboard = "https://cloud.unity.com"
}
