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
    /// False until the first project and editor scan finishes (avoids flashing "empty").
    private(set) var hasLoadedLocal = false

    // Releases
    /// Releases page tab; Recommended (the Hub's "Official releases") first.
    var releaseTab: ReleaseTab = .recommended
    var releases: [Release] = []
    var releaseTotal: UInt32 = 0
    var isLoadingReleases = false
    /// Version search on the Releases page ("2022.3"); searches every stream when set.
    var releaseQuery = ""
    /// Why the last release list failed to load. Shown in the list, not as an alert.
    var releaseError: String?
    /// Bumped on every fresh load, so a slower earlier load cannot add stale rows.
    private var releaseGeneration = 0

    // Installing editors and modules
    private(set) var jobs: [Job] = []
    private var jobPolling: Task<Void, Never>?
    /// Release whose "Install" sheet is open.
    var installingRelease: Release?
    /// Editor whose "Modules" sheet is open.
    var modulesEditor: Editor?
    var editorPendingUninstall: Editor?

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

    /// The open "Clean Up" sheet, if any.
    var cleanup: Cleanup?

    // UniHub updates
    /// A newer UniHub release; its sheet shows while this is set.
    var availableUpdate: AppUpdate?
    private(set) var isCheckingForUpdate = false
    private(set) var isInstallingUpdate = false
    /// Result of a manual check that found nothing ("UniHub 0.1.0 is up to date").
    var updateCheckMessage: String?
    /// Why the last install failed; shown in the update sheet.
    var updateError: String?

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

    /// The five projects opened or changed most recently (the menu bar list).
    var recentProjects: [Project] {
        let recency = { (p: Project) in max(p.lastOpenedMs ?? 0, p.modifiedMs ?? 0) }
        return Array(projects.filter(\.exists).sorted { recency($0) > recency($1) }.prefix(5))
    }

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
        hasLoadedLocal = true
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

    // MARK: Clean up

    /// Opens the Clean Up sheet and measures what it would remove.
    func requestCleanup(of project: Project) {
        cleanup = Cleanup(project: project)
        Task { await loadCleanupPlan() }
    }

    func loadCleanupPlan() async {
        guard let unihub, let path = cleanup?.project.path else { return }
        cleanup?.isMeasuring = true
        cleanup?.error = nil
        let result = await Task.detached { Result { try unihub.cleanPlan(path: path) } }.value
        guard cleanup?.project.path == path else { return }  // closed or replaced meanwhile
        cleanup?.isMeasuring = false
        switch result {
        case .success(let plan):
            cleanup?.plan = plan
            // Pre-select what Unity or the IDE recreates; never builds or ignored assets.
            cleanup?.selected = Set(plan.items.filter { $0.kind != .other }.map(\.path))
        case .failure(let error):
            cleanup?.error = describe(error)
        }
    }

    func performCleanup() async {
        guard let unihub, let current = cleanup, current.plan != nil, !current.selected.isEmpty else { return }
        let path = current.project.path
        let items = Array(current.selected)
        cleanup?.isDeleting = true
        cleanup?.error = nil
        let result = await Task.detached { Result { try unihub.cleanProject(path: path, items: items) } }.value
        guard cleanup?.project.path == path else { return }
        cleanup?.isDeleting = false
        switch result {
        case .success(let done): cleanup?.result = done
        case .failure(let error): cleanup?.error = describe(error)
        }
        if detailsPath == path { reloadDetails() }  // the size on disk changed
        await refreshLocal()
    }

    func copyToPasteboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    func revealInFinder(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(filePath: path)])
    }

    // MARK: Releases

    /// Loads the selected stream, or searches every stream when `releaseQuery` is set.
    /// Failures show in the list (`releaseError`), never as an alert.
    func loadReleases(reset: Bool) async {
        guard let unihub else { return }
        if reset {
            releaseGeneration += 1
            releases = []
            releaseTotal = 0
            releaseError = nil
        } else if isLoadingReleases {
            return
        }
        isLoadingReleases = true
        let generation = releaseGeneration

        let stream = releaseTab.stream
        let query = releaseQuery.trimmingCharacters(in: .whitespaces)
        let offset = UInt32(releases.count)
        let result = await Task.detached {
            Result {
                if !query.isEmpty {
                    return try unihub.searchReleases(query: query, limit: 25, offset: offset)
                }
                guard let stream else {
                    let list = try unihub.recommendedReleases()
                    return ReleasePage(total: UInt32(list.count), offset: 0, releases: list)
                }
                return try unihub.fetchReleases(stream: stream, limit: 25, offset: offset, versionPrefix: nil)
            }
        }.value
        // The user switched tabs or typed meanwhile: a newer load owns the list.
        guard generation == releaseGeneration else { return }
        isLoadingReleases = false
        switch result {
        case .success(let page):
            releases += page.releases
            releaseTotal = page.total
        case .failure(let error):
            releaseError = describe(error)
        }
    }

    // MARK: UniHub updates

    var appVersion: String { unihub?.appVersion() ?? "" }

    /// Checks GitHub for a newer UniHub. A manual check also reports "up to date" and errors;
    /// the automatic one stays quiet unless it finds an update.
    func checkForUpdate(manual: Bool) async {
        guard let unihub, !isCheckingForUpdate else { return }
        isCheckingForUpdate = true
        defer { isCheckingForUpdate = false }
        let result = await Task.detached { Result { try unihub.checkForUpdate() } }.value
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: Self.lastUpdateCheckKey)
        switch result {
        case .success(let update?):
            availableUpdate = update
        case .success(nil):
            if manual { updateCheckMessage = "UniHub \(appVersion) is the latest version." }
        case .failure(let error):
            if manual { updateCheckMessage = "Could not check for updates.\n\(describe(error))" }
        }
    }

    /// The automatic check at launch: at most once a day, unless turned off in Settings.
    func checkForUpdateIfDue() async {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: Self.autoUpdateKey) as? Bool ?? true else { return }
        let last = defaults.double(forKey: Self.lastUpdateCheckKey)
        guard Date().timeIntervalSince1970 - last > 24 * 60 * 60 else { return }
        await checkForUpdate(manual: false)
    }

    static let autoUpdateKey = "checkForUpdatesAutomatically"
    private static let lastUpdateCheckKey = "lastUpdateCheck"

    /// Downloads and installs the update, then relaunches UniHub.
    func installUpdate(_ update: AppUpdate) async {
        guard let unihub, !isInstallingUpdate else { return }
        isInstallingUpdate = true
        updateError = nil
        let appPath = Bundle.main.bundlePath
        let result = await Task.detached { Result { try unihub.installUpdate(update: update, appPath: appPath) } }.value
        switch result {
        case .success:
            // Close the sheet first: AppKit does not quit while a sheet is open.
            availableUpdate = nil
            relaunch(appPath)
        case .failure(let error):
            // Shown in the sheet, which stays open (an alert would be lost while it closes).
            isInstallingUpdate = false
            updateError = describe(error)
        }
    }

    /// Quits and opens the updated app once this process has really exited.
    private func relaunch(_ appPath: String) {
        let pid = String(ProcessInfo.processInfo.processIdentifier)
        let task = Process()
        task.executableURL = URL(filePath: "/bin/sh")
        // Waits up to 30 s for this process to exit; `open` on a still-running app would only
        // bring the old one to the front.
        task.arguments = [
            "-c",
            "i=0; while kill -0 \"$1\" 2>/dev/null && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done; /usr/bin/open \"$0\"",
            appPath, pid,
        ]
        try? task.run()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
            NSApp.terminate(nil)
            // If something still keeps the app open, quit anyway: the update is installed.
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { exit(0) }
        }
    }

    // MARK: Installing editors and modules

    /// Folder new editors go into.
    var editorInstallDir: String { unihub?.editorInstallDir() ?? "/Applications/Unity/Hub/Editor" }

    /// The running install of this version, if any.
    func runningJob(version: String) -> Job? {
        jobs.last { $0.version == version && $0.state == .running }
    }

    func isBusy(_ editor: Editor) -> Bool {
        runningJob(version: editor.version) != nil
    }

    /// Starts installing an editor in the background. Returns false (and shows why) if
    /// it could not start.
    func installEditor(_ release: Release, modules: [String], acceptTerms: Bool) async -> Bool {
        let version = release.version
        let architecture = release.architecture
        let started = await perform {
            _ = try $0.installEditor(version: version, architecture: architecture, modules: modules, acceptTerms: acceptTerms)
        }
        if started { pollJobs() }
        return started
    }

    /// Every module of an installed editor (installed ones marked).
    func modules(of editor: Editor) async -> Result<[Module], Error> {
        guard let unihub else { return .success(editor.modules) }
        let version = editor.version
        let architecture = editor.architecture
        return await Task.detached {
            Result { try unihub.editorModules(version: version, architecture: architecture) }
        }.value
    }

    /// Removes modules right away, then adds the others in the background.
    func changeModules(of editor: Editor, add: [String], remove: [String], acceptTerms: Bool) async -> Bool {
        let version = editor.version
        let architecture = editor.architecture
        if !remove.isEmpty {
            let removed = await perform {
                try $0.removeModules(version: version, architecture: architecture, modules: remove)
            }
            guard removed else { return false }
        }
        if !add.isEmpty {
            let started = await perform {
                _ = try $0.installModules(version: version, architecture: architecture, modules: add, acceptTerms: acceptTerms)
            }
            if started { pollJobs() }
            await refreshLocal()
            return started
        }
        await refreshLocal()
        return true
    }

    func uninstall(_ editor: Editor) async {
        let version = editor.version
        let architecture = editor.architecture
        if await perform({ try $0.uninstallEditor(version: version, architecture: architecture) }) {
            await refreshLocal()
        }
    }

    func cancelJob(_ job: Job) {
        unihub?.cancelJob(id: job.id)
        pollJobs()
    }

    func dismissJob(_ job: Job) {
        unihub?.dismissJob(id: job.id)
        jobs.removeAll { $0.id == job.id }
    }

    /// Polls the core's installs while one is running; refreshes the editor list when
    /// an install ends.
    func pollJobs() {
        guard jobPolling == nil, let unihub else { return }
        jobPolling = Task { [weak self] in
            while !Task.isCancelled {
                let latest = await Task.detached { unihub.jobs() }.value
                guard let self else { return }
                let ended = latest.contains { job in
                    job.state != .running && self.jobs.contains { $0.id == job.id && $0.state == .running }
                }
                self.jobs = latest
                if ended { await self.refreshLocal() }
                if !latest.contains(where: { $0.state == .running }) { break }
                try? await Task.sleep(for: .milliseconds(500))
            }
            self?.jobPolling = nil
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

/// Tabs of the Releases page.
enum ReleaseTab: String, CaseIterable, Identifiable {
    case recommended, lts, supported, beta, alpha, tech

    var id: Self { self }

    var title: String {
        switch self {
        case .recommended: "Recommended"
        case .lts: "LTS"
        case .supported: "Supported"
        case .beta: "Beta"
        case .alpha: "Alpha"
        case .tech: "Tech"
        }
    }

    /// `nil` for Recommended, which spans streams.
    var stream: ReleaseStream? {
        switch self {
        case .recommended: nil
        case .lts: .lts
        case .supported: .supported
        case .beta: .beta
        case .alpha: .alpha
        case .tech: .tech
        }
    }
}

/// State of the Clean Up sheet for one project.
struct Cleanup: Identifiable {
    let project: Project
    var plan: CleanPlan?
    /// Paths (from `plan`) to delete.
    var selected: Set<String> = []
    var isMeasuring = false
    var isDeleting = false
    var result: CleanResult?
    var error: String?

    var id: String { project.path }

    var selectedBytes: UInt64 {
        plan?.items.filter { selected.contains($0.path) }.reduce(0) { $0 + $1.bytes } ?? 0
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
