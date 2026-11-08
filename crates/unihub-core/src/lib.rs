//! UniHub core: project list, editor discovery, release catalogue, launching,
//! settings, and Unity account / tool management through the official Unity CLI.
//!
//! Exposed to Swift through UniFFI as the `UniHub` object. The same API is used by the
//! `unihub` command-line tool.

uniffi::setup_scaffolding!();

pub mod clean;
pub mod details;
pub mod editors;
mod error;
pub mod jobs;
pub mod modules;
mod paths;
pub mod projects;
pub mod releases;
pub mod settings;
pub mod unity_cli;
pub mod updates;
pub mod vcs;
pub mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub use clean::{CleanItem, CleanKind, CleanMethod, CleanPlan, CleanResult};
pub use details::{FolderSize, ProjectDetails, ProjectSize};
pub use editors::Editor;
pub use error::{Result, UniHubError};
pub use paths::UniHubPaths;
pub use projects::Project;
pub use jobs::{Job, JobKind, JobState};
pub use releases::{Download, LicenseTerms, Module, ModuleKind, Release, ReleasePage, ReleaseStream};
pub use settings::{Appearance, CliChannel, Settings};
pub use updates::AppUpdate;
pub use unity_cli::{Account, AuthStatus, CliInfo, CliUpdate, License, LicenseStatus, Plugin};
pub use vcs::{GitStatus, SourceControl, VcsProvider};

use projects::ProjectStore;

/// Official, documented installer for the Unity CLI (docs.unity3d.com/hub/unity-cli).
pub const CLI_INSTALL_SCRIPT: &str = "https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.sh";

#[derive(uniffi::Object)]
pub struct UniHub {
    paths: UniHubPaths,
    store: Mutex<ProjectStore>,
    settings: Mutex<Settings>,
    http: reqwest::blocking::Client,
    sign_in: unity_cli::SignIn,
    /// Last CLI found by `unity_cli()`. Finding it runs the binary (~1 s), so the
    /// account and tool calls reuse this.
    cli: Mutex<Option<CliInfo>>,
    /// Editor and module installs running in the background.
    jobs: jobs::JobRunner,
}

#[uniffi::export]
impl UniHub {
    /// Uses the standard macOS locations.
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>> {
        Self::with_paths(UniHubPaths::detect())
    }

    #[uniffi::constructor]
    pub fn with_paths(paths: UniHubPaths) -> Result<Arc<Self>> {
        let settings = Settings::load(&paths.data_dir())?;
        let hub_dir = paths.hub_data_dir();
        let mut store = ProjectStore::open(&paths.data_dir(), hub_dir.as_deref())?;
        if settings.sync_hub_projects_on_launch {
            if let Some(hub) = &hub_dir {
                // A broken Hub file should not stop UniHub from starting.
                let _ = store.import_from_hub(hub);
            }
        }
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("UniHub/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Arc::new(Self {
            paths,
            store: Mutex::new(store),
            settings: Mutex::new(settings),
            http,
            sign_in: unity_cli::SignIn::default(),
            cli: Mutex::new(None),
            jobs: jobs::JobRunner::default(),
        }))
    }

    pub fn paths(&self) -> UniHubPaths {
        self.paths.clone()
    }

    // ---- UniHub updates -----------------------------------------------------------

    /// UniHub's version (the core and the app are built and released together).
    pub fn app_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// The latest UniHub release on GitHub when it is newer than this build, else `None`.
    pub fn check_for_update(&self) -> Result<Option<AppUpdate>> {
        updates::check(&self.http, env!("CARGO_PKG_VERSION"))
    }

    /// Downloads `update`, checks its SHA-256 and the app inside, and puts it in place of
    /// the app at `app_path` (the running `UniHub.app`). The caller relaunches.
    pub fn install_update(&self, update: AppUpdate, app_path: String) -> Result<()> {
        updates::install(&self.http, &update, Path::new(&app_path), &self.paths.data_dir().join("tmp/update"))
    }

    // ---- Settings -----------------------------------------------------------------

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// Saves new settings and returns them as stored (blank paths cleared, limits applied).
    pub fn update_settings(&self, settings: Settings) -> Result<Settings> {
        let settings = settings.cleaned();
        settings.save(&self.paths.data_dir())?;
        let previous = std::mem::replace(&mut *lock(&self.settings), settings.clone());
        if previous.unity_cli_path != settings.unity_cli_path {
            *lock(&self.cli) = None;
        }
        Ok(settings)
    }

    pub fn reset_settings(&self) -> Result<Settings> {
        self.update_settings(Settings::default())
    }

    // ---- Projects -----------------------------------------------------------------

    pub fn list_projects(&self) -> Vec<Project> {
        self.store().list()
    }

    pub fn add_project(&self, path: String) -> Result<Project> {
        self.store().add(&path)
    }

    pub fn remove_project(&self, path: String) -> Result<()> {
        self.store().remove(&path)
    }

    pub fn set_favorite(&self, path: String, favorite: bool) -> Result<()> {
        self.store().update(&path, |p| p.is_favorite = favorite)
    }

    /// Everything for the details panel: settings, render pipeline, build target,
    /// cloud project and source control. Runs git and, if installed, the Unity CLI,
    /// so call it off the main thread.
    pub fn project_details(&self, path: String) -> Result<ProjectDetails> {
        self.known_project(&path)?;
        Ok(details::load(Path::new(&path), self.cli_path().ok().as_deref()))
    }

    /// Disk usage by folder. Can take several seconds on large projects.
    pub fn project_size(&self, path: String) -> Result<ProjectSize> {
        self.known_project(&path)?;
        Ok(details::size(Path::new(&path), self.cli_path().ok().as_deref()))
    }

    /// What "Clean Up" would remove: git-ignored files in a git repository, otherwise
    /// the generated folders and files at the project root. Measures every item, so call
    /// it off the main thread.
    pub fn clean_plan(&self, path: String) -> Result<CleanPlan> {
        self.existing_project(&path)?;
        Ok(clean::plan(Path::new(&path)))
    }

    /// Deletes the given items of `clean_plan` for good. Refused while the project is
    /// open in Unity; paths that are not clean-up candidates are skipped and reported.
    pub fn clean_project(&self, path: String, items: Vec<String>) -> Result<CleanResult> {
        self.existing_project(&path)?;
        clean::clean(Path::new(&path), &items)
    }

    /// Adds projects from the official Hub that UniHub does not list yet.
    pub fn import_from_hub(&self) -> Result<u32> {
        match self.paths.hub_data_dir() {
            Some(hub) => self.store().import_from_hub(&hub),
            None => Ok(0),
        }
    }

    // ---- Editors ------------------------------------------------------------------

    pub fn list_editors(&self) -> Vec<Editor> {
        editors::discover(&self.editor_search_paths())
    }

    /// The folder new editors go into.
    pub fn editor_install_dir(&self) -> String {
        lock(&self.settings)
            .editor_install_dir
            .clone()
            .unwrap_or_else(|| paths::DEFAULT_EDITOR_ROOT.to_string())
    }

    /// Opens a project in an editor. With no `editor_version`, uses the version the
    /// project was last saved with.
    pub fn open_project(&self, path: String, editor_version: Option<String>) -> Result<()> {
        let project = self
            .store()
            .get(&path)
            .ok_or_else(|| UniHubError::UnknownProject { path: path.clone() })?;
        if !project.exists {
            return Err(UniHubError::NotAProject { path });
        }
        let wanted = editor_version
            .or(project.editor_version)
            .ok_or_else(|| UniHubError::NoEditorVersion { path: path.clone() })?;
        let editor = self
            .list_editors()
            .into_iter()
            .find(|e| e.version == wanted)
            .ok_or(UniHubError::EditorNotInstalled { version: wanted })?;

        launch(&editor, &path)?;
        self.store().update(&path, |p| p.last_opened_ms = Some(now_ms()))
    }

    // ---- Releases -----------------------------------------------------------------

    /// One page (at most 25) of releases with an Apple Silicon build, newest first.
    pub fn fetch_releases(
        &self,
        stream: ReleaseStream,
        limit: u32,
        offset: u32,
        version_prefix: Option<String>,
    ) -> Result<ReleasePage> {
        releases::fetch(&self.http, Some(stream), limit, offset, version_prefix.as_deref())
    }

    /// The newest release of each line Unity still supports (Supported and LTS), newest
    /// first: the Hub's "Official releases". One is flagged `recommended`.
    pub fn recommended_releases(&self) -> Result<Vec<Release>> {
        releases::recommended(&self.http)
    }

    /// Searches every stream for versions starting with `query` (`2022.3`, `2019.4.4`).
    /// Finds releases without an Apple Silicon build too (their Intel build).
    pub fn search_releases(&self, query: String, limit: u32, offset: u32) -> Result<ReleasePage> {
        releases::fetch(&self.http, None, limit, offset, Some(&query))
    }

    // ---- Installing editors and modules (through the Unity CLI) ---------------------

    /// Installs an editor with the given modules in the background and returns the job
    /// id. `modules` are module ids to install with it (children the user kept ticked
    /// included; hidden dependencies are added by the CLI). `accept_terms` must be true
    /// only after the user accepted the license terms of those modules.
    pub fn install_editor(
        &self,
        version: String,
        architecture: String,
        modules: Vec<String>,
        accept_terms: bool,
    ) -> Result<u64> {
        if self.list_editors().iter().any(|e| e.version == version && e.architecture == architecture) {
            return Err(UniHubError::InvalidData { message: format!("Unity {version} is already installed") });
        }
        if self.jobs.is_running(&version, &architecture) {
            return Err(UniHubError::InvalidData { message: format!("Unity {version} is already being installed") });
        }
        let cli = self.cli_path()?;
        let mut args = vec!["install".to_string(), version.clone(), "--no-child-modules".into()];
        args.extend(cli_architecture(&architecture));
        if accept_terms {
            args.push("--accept-eula".into());
        }
        // The CLI installs into its (and the Hub's) install path unless told otherwise.
        let root = self.editor_install_dir();
        if unity_cli::install_path(&cli).as_deref() != Some(root.as_str()) {
            args.extend(["--download-directory".into(), root]);
        }
        if !modules.is_empty() {
            args.push("--module".into());
            args.extend(modules.iter().cloned());
        }
        self.jobs.start(&cli, args, JobKind::InstallEditor, &version, &architecture, modules)
    }

    /// Every module of an installed editor, installed or not. Editors without
    /// `modules.json` get the release's catalogue from Unity's API, with the modules
    /// found on disk marked installed.
    pub fn editor_modules(&self, version: String, architecture: String) -> Result<Vec<Module>> {
        let editor = self.find_editor(&version, &architecture)?;
        if modules::has_catalog(Path::new(&editor.install_dir)) {
            return Ok(editor.modules);
        }
        match releases::find(&self.http, &version)? {
            Some(release) => {
                let mut list = release.modules;
                modules::merge_installed(&mut list, &editor.modules);
                Ok(list)
            }
            None => Ok(editor.modules),
        }
    }

    /// Adds modules to an installed editor in the background; returns the job id.
    /// `accept_terms`: as for `install_editor`.
    pub fn install_modules(
        &self,
        version: String,
        architecture: String,
        modules: Vec<String>,
        accept_terms: bool,
    ) -> Result<u64> {
        if modules.is_empty() {
            return Err(UniHubError::InvalidData { message: "no modules selected".into() });
        }
        let editor = self.find_editor(&version, &architecture)?;
        let cli = self.cli_path()?;
        let mut args =
            vec!["install-modules".to_string(), "--editor-version".into(), version.clone(), "--no-child-modules".into()];
        args.extend(cli_architecture(&editor.architecture));
        if accept_terms {
            args.push("--accept-eula".into());
        }
        // Editors outside the CLI's install path: point it at their root folder.
        let root = Path::new(&editor.install_dir).parent().map(paths::path_string).unwrap_or_default();
        if unity_cli::install_path(&cli).as_deref() != Some(root.as_str()) {
            args.extend(["--download-directory".into(), root]);
        }
        args.push("--module".into());
        args.extend(modules.iter().cloned());
        self.jobs.start(&cli, args, JobKind::InstallModules, &version, &editor.architecture, modules)
    }

    /// Removes installed modules from an editor (their folders and their children).
    pub fn remove_modules(&self, version: String, architecture: String, modules: Vec<String>) -> Result<()> {
        if modules.is_empty() {
            return Ok(());
        }
        let editor = self.find_editor(&version, &architecture)?;
        if self.jobs.is_running(&version, &editor.architecture) {
            return Err(UniHubError::InvalidData {
                message: format!("Unity {version} is being installed or changed; wait until it finishes"),
            });
        }
        let cli = self.cli_path()?;
        let mut args = vec!["editor", "module", "remove", version.as_str(), "--yes"];
        let arch = cli_architecture(&editor.architecture);
        args.extend(arch.iter().map(String::as_str));
        args.push("--module");
        args.extend(modules.iter().map(String::as_str));
        unity_cli::run_json(&cli, &args).map(|_| ())
    }

    /// Uninstalls an editor (deletes its folder) with the Unity CLI.
    pub fn uninstall_editor(&self, version: String, architecture: String) -> Result<()> {
        let editor = self.find_editor(&version, &architecture)?;
        if self.jobs.is_running(&version, &editor.architecture) {
            return Err(UniHubError::InvalidData {
                message: format!("Unity {version} is being installed or changed; wait until it finishes"),
            });
        }
        let cli = self.cli_path()?;
        let mut args = vec!["uninstall", version.as_str(), "--yes"];
        let arch = cli_architecture(&editor.architecture);
        args.extend(arch.iter().map(String::as_str));
        unity_cli::run_json(&cli, &args).map(|_| ())
    }

    /// Installs started in this session, running and finished.
    pub fn jobs(&self) -> Vec<Job> {
        self.jobs.list()
    }

    /// Stops a running install. What was already installed stays.
    pub fn cancel_job(&self, id: u64) {
        self.jobs.cancel(id);
    }

    /// Removes a finished install from `jobs()`.
    pub fn dismiss_job(&self, id: u64) {
        self.jobs.dismiss(id);
    }

    // ---- Unity CLI ----------------------------------------------------------------

    /// Looks for the Unity CLI again and returns it, or `None` when it is not installed.
    pub fn unity_cli(&self) -> Option<CliInfo> {
        let override_path = lock(&self.settings).unity_cli_path.clone();
        let found = unity_cli::locate(override_path.as_deref());
        *lock(&self.cli) = found.clone();
        found
    }

    /// Installs the Unity CLI with Unity's official installer script. The script
    /// verifies the binary's SHA-256, installs it to `~/.unity/bin/unity` and adds that
    /// folder to PATH in the user's shell configuration.
    pub fn install_unity_cli(&self) -> Result<CliInfo> {
        let channel = lock(&self.settings).cli_channel;
        let script = self.http.get(CLI_INSTALL_SCRIPT).send()?.error_for_status()?.bytes()?;
        let dir = self.paths.data_dir().join("tmp");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("unity-cli-install.sh");
        std::fs::write(&file, &script)?;

        let mut cmd = Command::new("/bin/bash");
        cmd.arg(&file).env("NO_COLOR", "1").stdin(Stdio::null());
        if channel == CliChannel::Beta {
            cmd.env("UNITY_CLI_CHANNEL", "beta");
        }
        let output = cmd.output();
        let _ = std::fs::remove_file(&file);
        let output = output?;
        if !output.status.success() {
            let text = String::from_utf8_lossy(&output.stderr);
            return Err(UniHubError::CliFailed {
                code: output.status.code().map_or_else(|| "signal".into(), |c| format!("exit-{c}")),
                message: last_line(&text).unwrap_or("the installer failed").to_string(),
            });
        }
        self.unity_cli().ok_or(UniHubError::CliNotInstalled)
    }

    pub fn check_unity_cli_update(&self) -> Result<CliUpdate> {
        unity_cli::update_check(&self.cli_path()?)
    }

    pub fn update_unity_cli(&self) -> Result<CliInfo> {
        let channel = lock(&self.settings).cli_channel;
        let cli = self.cli_path()?;
        let mut args = vec!["self-update", "--yes"];
        if channel == CliChannel::Beta {
            // Without a flag the CLI stays on its current channel.
            args.extend(["--channel", channel.name()]);
        }
        unity_cli::run_json(&cli, &args)?;
        self.unity_cli().ok_or(UniHubError::CliNotInstalled)
    }

    /// Removes the CLI binary and its PATH setup. Keeps its data (sign-in, settings).
    pub fn uninstall_unity_cli(&self) -> Result<()> {
        unity_cli::run_json(&self.cli_path()?, &["self-uninstall", "--yes"])?;
        *lock(&self.cli) = None;
        Ok(())
    }

    // ---- Account --------------------------------------------------------------------

    pub fn auth_status(&self) -> Result<AuthStatus> {
        unity_cli::auth_status(&self.cli_path()?)
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        unity_cli::accounts(&self.cli_path()?)
    }

    /// Opens the Unity sign-in page in the browser and waits until the user finishes
    /// (or `cancel_sign_in` is called). Blocks: call it off the main thread.
    pub fn sign_in(&self) -> Result<AuthStatus> {
        let cli = self.cli_path()?;
        self.sign_in.run(&cli)?;
        unity_cli::auth_status(&cli)
    }

    pub fn cancel_sign_in(&self) {
        self.sign_in.cancel();
    }

    /// Signs out one account (by id or email), or every account when `None`.
    pub fn sign_out(&self, account: Option<String>) -> Result<()> {
        let cli = self.cli_path()?;
        let mut args = vec!["auth", "logout", "--yes"];
        if let Some(a) = account.as_deref() {
            args.push(a);
        }
        unity_cli::run_json(&cli, &args).map(|_| ())
    }

    pub fn switch_account(&self, account: String) -> Result<()> {
        unity_cli::run_json(&self.cli_path()?, &["auth", "switch", &account]).map(|_| ())
    }

    // ---- Licenses -------------------------------------------------------------------

    pub fn license_status(&self) -> Result<LicenseStatus> {
        unity_cli::license_status(&self.cli_path()?)
    }

    /// Activates the free Unity Personal license. `accept_terms` must be true: the UI
    /// passes it only after the user has agreed to the Unity Personal terms.
    pub fn activate_personal_license(&self, accept_terms: bool) -> Result<()> {
        if !accept_terms {
            return Err(UniHubError::InvalidData {
                message: "the Unity Personal license terms must be accepted first".into(),
            });
        }
        unity_cli::run_json(&self.cli_path()?, &["license", "activate", "--personal", "--accept-eula"])
            .map(|_| ())
    }

    pub fn return_licenses(&self) -> Result<()> {
        unity_cli::run_json(&self.cli_path()?, &["license", "return", "--yes"]).map(|_| ())
    }

    // ---- Tools (CLI plugins) --------------------------------------------------------

    /// Tools the Unity CLI can install. `refresh` re-downloads the registry.
    pub fn plugins(&self, refresh: bool) -> Result<Vec<Plugin>> {
        unity_cli::plugins(&self.cli_path()?, refresh)
    }

    /// Installs a tool. `accept_terms` must be true: the UI passes it only after the
    /// user has agreed to the tool's license terms.
    pub fn install_plugin(&self, id: String, accept_terms: bool) -> Result<()> {
        let mut args = vec!["plugin", "install", id.as_str()];
        if accept_terms {
            args.push("--accept-eula");
        }
        unity_cli::run_json(&self.cli_path()?, &args).map(|_| ())
    }

    pub fn upgrade_plugin(&self, id: String, accept_terms: bool) -> Result<()> {
        let mut args = vec!["plugin", "upgrade", id.as_str()];
        if accept_terms {
            args.push("--accept-eula");
        }
        unity_cli::run_json(&self.cli_path()?, &args).map(|_| ())
    }

    /// Removes a tool the Unity CLI installed. Tools installed some other way are reported
    /// as an error instead of silently staying.
    pub fn remove_plugin(&self, id: String) -> Result<()> {
        let cli = self.cli_path()?;
        let data = match unity_cli::run_json(&cli, &["plugin", "remove", &id, "--yes"]) {
            // Older CLIs have no --yes here; they do not ask when not interactive either.
            Err(UniHubError::CliFailed { message, .. }) if message.contains("--yes") => {
                unity_cli::run_json(&cli, &["plugin", "remove", &id])?
            }
            other => other?,
        };
        unity_cli::check_plugin_removed(&data, &id)
    }
}

impl UniHub {
    fn store(&self) -> MutexGuard<'_, ProjectStore> {
        lock(&self.store)
    }

    fn known_project(&self, path: &str) -> Result<()> {
        match self.store().get(path) {
            Some(_) => Ok(()),
            None => Err(UniHubError::UnknownProject { path: path.to_string() }),
        }
    }

    /// An installed editor by version and architecture (any architecture when it is
    /// empty or there is a single build of that version).
    fn find_editor(&self, version: &str, architecture: &str) -> Result<Editor> {
        let matching: Vec<Editor> = self.list_editors().into_iter().filter(|e| e.version == version).collect();
        matching
            .iter()
            .find(|e| e.architecture == architecture)
            .or_else(|| if matching.len() == 1 { matching.first() } else { None })
            .cloned()
            .ok_or_else(|| UniHubError::EditorNotInstalled { version: version.to_string() })
    }

    /// A listed project whose folder is still a Unity project.
    fn existing_project(&self, path: &str) -> Result<()> {
        match self.store().get(path) {
            Some(p) if p.exists => Ok(()),
            Some(_) => Err(UniHubError::NotAProject { path: path.to_string() }),
            None => Err(UniHubError::UnknownProject { path: path.to_string() }),
        }
    }

    /// The cached CLI if its binary is still there, otherwise a fresh search.
    fn cli_path(&self) -> Result<PathBuf> {
        let cached = lock(&self.cli).as_ref().map(|i| PathBuf::from(&i.path));
        if let Some(path) = cached.filter(|p| p.is_file()) {
            return Ok(path);
        }
        self.unity_cli().map(|i| PathBuf::from(i.path)).ok_or(UniHubError::CliNotInstalled)
    }

    /// Built-in editor roots plus the ones from settings.
    fn editor_search_paths(&self) -> UniHubPaths {
        let settings = lock(&self.settings);
        let mut paths = self.paths.clone();
        let extra = settings.editor_install_dir.iter().chain(settings.extra_editor_dirs.iter());
        for dir in extra {
            if !paths.editor_roots.contains(dir) {
                paths.editor_roots.push(dir.clone());
            }
        }
        paths
    }
}

/// `--architecture <arch>` for the Unity CLI, which knows `arm64` and `x86_64` only.
fn cli_architecture(architecture: &str) -> Vec<String> {
    match architecture {
        "arm64" | "x86_64" => vec!["--architecture".into(), architecture.into()],
        _ => Vec::new(),
    }
}

/// A panic while holding a lock leaves the data intact, so keep going.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the editor detached from UniHub. A background thread reaps it on exit so
/// no zombie process is left behind.
fn launch(editor: &Editor, project_path: &str) -> Result<()> {
    let mut child = Command::new(&editor.executable_path)
        .arg("-projectPath")
        .arg(Path::new(project_path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).last()
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}
