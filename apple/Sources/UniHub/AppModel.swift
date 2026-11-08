import AppKit
import Observation
import UniHubCore

/// UI state. All calls into the Rust core run off the main thread.
@MainActor
@Observable
final class AppModel {
    // Projects and editors
    var projects: [Project] = []
    var editors: [Editor] = []

    // Releases
    var stream: ReleaseStream = .lts
    var releases: [Release] = []
    var releaseTotal: UInt32 = 0
    var isLoadingReleases = false

    // Settings
    private(set) var settings: Settings

    // Unity CLI, account and tools (all through the official Unity CLI)
    var cli: CliInfo?
    var cliUpdate: CliUpdate?
    var auth: AuthStatus?
    var accounts: [Account] = []
    var license: LicenseStatus?
    var plugins: [Plugin] = []
    var isSigningIn = false
    var isLoadingAccount = true  // until the first refresh finishes
    var isLoadingTools = false
    var cliBusy: String?  // what the CLI section is doing, e.g. "Installing…"
    var busyPlugins: Set<String> = []

    // Project details panel
    var showDetails = false
    private(set) var detailsPath: String?
    private(set) var details: ProjectDetails?
    private(set) var detailsSize: ProjectSize?
    private(set) var isLoadingDetails = false
    private(set) var isLoadingSize = false

    var isAddingProject = false
    var projectPendingRemoval: Project?
    var errorMessage: String?

    private let unihub: UniHub?

    init() {
        do {
            let core = try UniHub()
            unihub = core
            settings = core.settings()
        } catch {
            unihub = nil
            settings = Settings.defaults
            errorMessage = describe(error)
        }
        applyAppearance()
    }

    var installedVersions: Set<String> { Set(editors.map(\.version)) }

    func isInstalled(_ version: String?) -> Bool {
        version.map(installedVersions.contains) ?? false
    }

    var dataDirectory: String? { unihub?.paths().dataDir }

    // MARK: Projects and editors

    func refreshLocal() async {
        guard let unihub else { return }
        let (projects, editors) = await Task.detached {
            (unihub.listProjects(), unihub.listEditors())
        }.value
        self.projects = projects
        self.editors = editors
    }

    func addProject(at url: URL) async {
        let path = url.path(percentEncoded: false)
        if await perform({ _ = try $0.addProject(path: path) }) {
            await refreshLocal()
        }
    }

    /// Removes now, or asks first when the setting says so.
    func requestRemoval(of project: Project) {
        if settings.askBeforeRemovingProject {
            projectPendingRemoval = project
        } else {
            Task { await remove(project) }
        }
    }

    func remove(_ project: Project) async {
        if await perform({ try $0.removeProject(path: project.path) }) {
            await refreshLocal()
        }
    }

    func toggleFavorite(_ project: Project) async {
        let favorite = !project.isFavorite
        if await perform({ try $0.setFavorite(path: project.path, favorite: favorite) }) {
            await refreshLocal()
        }
    }

    func importFromHub() async {
        if await perform({ _ = try $0.importFromHub() }) {
            await refreshLocal()
        }
    }

    func open(_ project: Project, with version: String? = nil) async {
        if await perform({ try $0.openProject(path: project.path, editorVersion: version) }) {
            if settings.hideOnProjectOpen { NSApp.hide(nil) }
            await refreshLocal()
        }
    }

    var detailsProject: Project? {
        detailsPath.flatMap { path in projects.first { $0.path == path } }
    }

    /// The row's arrow button: opens the panel for this project, or closes it if it
    /// already shows this project.
    func toggleDetails(for project: Project) {
        if showDetails && detailsPath == project.path {
            showDetails = false
        } else {
            showDetails(for: project)
        }
    }

    func showDetails(for project: Project) {
        showDetails = true
        guard detailsPath != project.path || details == nil else { return }
        detailsPath = project.path
        details = nil
        detailsSize = nil
        Task { await loadDetails(path: project.path) }
    }

    func reloadDetails() {
        guard let path = detailsPath else { return }
        details = nil
        detailsSize = nil
        Task { await loadDetails(path: path) }
    }

    private func loadDetails(path: String) async {
        guard let unihub else { return }
        isLoadingDetails = true
        let result = await Task.detached { Result { try unihub.projectDetails(path: path) } }.value
        guard detailsPath == path else { return }  // another project was picked meanwhile
        isLoadingDetails = false
        switch result {
        case .success(let d): details = d
        case .failure(let error): errorMessage = describe(error)
        }

        isLoadingSize = true
        let size = try? await Task.detached { try unihub.projectSize(path: path) }.value
        guard detailsPath == path else { return }
        detailsSize = size
        isLoadingSize = false
    }

    func copyToPasteboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    func revealInFinder(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(filePath: path)])
    }

    // MARK: Releases

    func loadReleases(reset: Bool) async {
        guard let unihub, !isLoadingReleases else { return }
        if reset {
            releases = []
            releaseTotal = 0
        }
        isLoadingReleases = true
        defer { isLoadingReleases = false }

        let stream = self.stream
        let offset = UInt32(releases.count)
        do {
            let page = try await Task.detached {
                try unihub.fetchReleases(stream: stream, limit: 25, offset: offset, versionPrefix: nil)
            }.value
            guard stream == self.stream else { return }  // user switched tabs meanwhile
            releases += page.releases
            releaseTotal = page.total
        } catch {
            errorMessage = describe(error)
        }
    }

    func openURL(_ string: String?) {
        guard let string, let url = URL(string: string) else { return }
        NSWorkspace.shared.open(url)
    }

    // MARK: Settings

    func updateSettings(_ change: (inout Settings) -> Void) {
        guard let unihub else { return }
        var next = settings
        change(&next)
        do {
            settings = try unihub.updateSettings(settings: next)
            applyAppearance()
        } catch {
            errorMessage = describe(error)
        }
        Task { await refreshLocal() }  // editor folders may have changed
    }

    func resetSettings() {
        guard let unihub else { return }
        do {
            settings = try unihub.resetSettings()
            applyAppearance()
        } catch {
            errorMessage = describe(error)
        }
    }

    func applyAppearance() {
        switch settings.appearance {
        case .system: NSApp?.appearance = nil
        case .light: NSApp?.appearance = NSAppearance(named: .aqua)
        case .dark: NSApp?.appearance = NSAppearance(named: .darkAqua)
        }
    }

    // MARK: Unity CLI

    func refreshCli(checkUpdate: Bool = false) async {
        guard let unihub else { return }
        let cli = await Task.detached { unihub.unityCli() }.value
        self.cli = cli
        if cli == nil {
            cliUpdate = nil
        } else if checkUpdate {
            cliUpdate = try? await Task.detached { try unihub.checkUnityCliUpdate() }.value
        }
    }

    func installCli() async {
        await runCli("Installing…") { _ = try $0.installUnityCli() }
        await refreshAccount()
        await refreshTools(refresh: false)
    }

    func updateCli() async {
        await runCli("Updating…") { _ = try $0.updateUnityCli() }
        cliUpdate = nil
        await refreshCli(checkUpdate: true)
    }

    func uninstallCli() async {
        await runCli("Uninstalling…") { try $0.uninstallUnityCli() }
        auth = nil
        accounts = []
        license = nil
        plugins = []
    }

    private func runCli(_ label: String, _ work: @escaping @Sendable (UniHub) throws -> Void) async {
        cliBusy = label
        _ = await perform(work)
        cliBusy = nil
        await refreshCli()
    }

    // MARK: Account and licenses

    func refreshAccount() async {
        guard let unihub else { return }
        isLoadingAccount = true
        defer { isLoadingAccount = false }
        await refreshCli()
        guard cli != nil else {
            auth = nil
            accounts = []
            license = nil
            return
        }
        // Each CLI call takes about a second, so run them side by side.
        async let authResult = Task.detached { () -> Result<AuthStatus, Error> in
            Result { try unihub.authStatus() }
        }.value
        async let accountsResult = Task.detached { (try? unihub.accounts()) ?? [] }.value
        async let licenseResult = Task.detached { try? unihub.licenseStatus() }.value

        switch await authResult {
        case .success(let status):
            auth = status
            accounts = status.signedIn ? await accountsResult : []
            license = status.signedIn ? await licenseResult : nil
        case .failure(let error):
            auth = nil
            accounts = []
            license = nil
            errorMessage = describe(error)
        }
    }

    func signIn() async {
        guard let unihub, !isSigningIn else { return }
        isSigningIn = true
        defer { isSigningIn = false }
        do {
            auth = try await Task.detached { try unihub.signIn() }.value
        } catch UniHubError.Cancelled {
            // The user cancelled; nothing to report.
        } catch {
            errorMessage = describe(error)
        }
        await refreshAccount()
    }

    func cancelSignIn() {
        unihub?.cancelSignIn()
    }

    func signOut(_ account: Account? = nil) async {
        let id = account?.id
        _ = await perform { try $0.signOut(account: id) }
        await refreshAccount()
    }

    func switchAccount(_ account: Account) async {
        _ = await perform { try $0.switchAccount(account: account.id) }
        await refreshAccount()
    }

    func activatePersonalLicense() async {
        _ = await perform { try $0.activatePersonalLicense(acceptTerms: true) }
        await refreshAccount()
    }

    func returnLicenses() async {
        _ = await perform { try $0.returnLicenses() }
        await refreshAccount()
    }

    // MARK: Tools

    func refreshTools(refresh: Bool) async {
        guard let unihub else { return }
        await refreshCli(checkUpdate: refresh)
        guard cli != nil else {
            plugins = []
            return
        }
        isLoadingTools = true
        defer { isLoadingTools = false }
        do {
            plugins = try await Task.detached { try unihub.plugins(refresh: refresh) }.value
        } catch {
            errorMessage = describe(error)
        }
    }

    func install(_ plugin: Plugin) async {
        await changePlugin(plugin) { try $0.installPlugin(id: plugin.id, acceptTerms: true) }
    }

    func upgrade(_ plugin: Plugin) async {
        await changePlugin(plugin) { try $0.upgradePlugin(id: plugin.id, acceptTerms: true) }
    }

    func remove(_ plugin: Plugin) async {
        await changePlugin(plugin) { try $0.removePlugin(id: plugin.id) }
    }

    private func changePlugin(_ plugin: Plugin, _ work: @escaping @Sendable (UniHub) throws -> Void) async {
        busyPlugins.insert(plugin.id)
        _ = await perform(work)
        busyPlugins.remove(plugin.id)
        await refreshTools(refresh: false)
    }

    // MARK: Helpers

    /// Runs a core call in the background. Shows the error and returns false if it throws.
    private func perform(_ work: @escaping @Sendable (UniHub) throws -> Void) async -> Bool {
        guard let unihub else { return false }
        do {
            try await Task.detached { try work(unihub) }.value
            return true
        } catch {
            errorMessage = describe(error)
            return false
        }
    }
}

extension Settings {
    /// Used only if the core fails to start; mirrors the Rust defaults.
    static let defaults = Settings(
        appearance: .system,
        defaultProjectsDir: nil,
        editorInstallDir: nil,
        downloadsDir: nil,
        maxConcurrentDownloads: 2,
        extraEditorDirs: [],
        syncHubProjectsOnLaunch: true,
        unityCliPath: nil,
        cliChannel: .stable,
        hideOnProjectOpen: false,
        askBeforeRemovingProject: true
    )
}
