use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use unihub_core::{CleanMethod, JobState, ReleaseStream, UniHub, UniHubError};

/// UniHub: manage Unity projects and editors from the terminal.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List known projects.
    Projects,
    /// Add a project folder to the list.
    Add { path: String },
    /// Remove a project from the list (the folder is not touched).
    Remove { path: String },
    /// Open a project (path or exact title) in its editor, or in --editor.
    Open {
        project: String,
        #[arg(long)]
        editor: Option<String>,
    },
    /// Show a project's details: settings, render pipeline, build target, source control.
    Info {
        project: String,
        /// Also measure disk usage (can take a few seconds).
        #[arg(long)]
        size: bool,
    },
    /// Free disk space by removing files Unity and IDEs regenerate (Library, Temp, Logs,
    /// .sln/.csproj…). In a git repository: the files git ignores. Lists them unless --yes.
    Clean {
        project: String,
        /// Delete the listed files for good (otherwise only list them).
        #[arg(long)]
        yes: bool,
        /// Also remove ignored files nothing regenerates, e.g. builds and local settings.
        #[arg(long)]
        all: bool,
    },
    /// List installed editors.
    Editors,
    /// List editor releases for macOS (Apple Silicon, or Intel for older versions).
    /// With --version and no --stream, searches every stream.
    Releases {
        #[arg(long, value_enum)]
        stream: Option<Stream>,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Only versions starting with this, e.g. 6000.0 or 2019.4
        #[arg(long)]
        version: Option<String>,
        /// The newest release of each supported line (the Hub's "Official releases").
        #[arg(long, conflicts_with_all = ["stream", "version"])]
        recommended: bool,
    },
    /// Install an editor (and modules) with the Unity CLI, showing progress.
    Install {
        version: String,
        /// arm64 or x86_64 (default: what the release offers, Apple Silicon first).
        #[arg(long)]
        arch: Option<String>,
        /// Module ids to install with it, e.g. -m ios android android-sdk-ndk-tools
        #[arg(short, long, num_args = 1..)]
        module: Vec<String>,
        /// Accept the license terms of the modules (required for some, e.g. the Android SDK).
        #[arg(long)]
        accept_terms: bool,
    },
    /// Uninstall an editor.
    Uninstall {
        version: String,
        #[arg(long, default_value = "")]
        arch: String,
    },
    /// List an installed editor's modules (✓ installed).
    Modules {
        version: String,
        #[arg(long, default_value = "")]
        arch: String,
    },
    /// Add modules to an installed editor, showing progress.
    ModuleAdd {
        version: String,
        #[arg(short, long, num_args = 1.., required = true)]
        module: Vec<String>,
        #[arg(long, default_value = "")]
        arch: String,
        #[arg(long)]
        accept_terms: bool,
    },
    /// Remove modules from an installed editor.
    ModuleRemove {
        version: String,
        #[arg(short, long, num_args = 1.., required = true)]
        module: Vec<String>,
        #[arg(long, default_value = "")]
        arch: String,
    },
    /// Import projects from the official Hub's list.
    Import,
    /// Show the signed-in Unity account and license (uses the Unity CLI).
    Account,
    /// Sign in to Unity in the browser (uses the Unity CLI).
    Login,
    /// Sign out of one account (id or email), or all accounts.
    Logout { account: Option<String> },
    /// List the tools the Unity CLI can install.
    Tools {
        /// Re-download the tool registry first.
        #[arg(long)]
        refresh: bool,
    },
    /// Install a tool by id. You must pass --accept-terms to agree to its license.
    ToolInstall {
        id: String,
        #[arg(long)]
        accept_terms: bool,
    },
    /// Remove a tool by id.
    ToolRemove { id: String },
    /// Show the Unity CLI UniHub uses, and whether an update is available.
    Cli,
    /// Install the Unity CLI with Unity's official installer.
    CliInstall,
    /// Print UniHub's settings.
    Settings,
}

#[derive(Clone, Copy, ValueEnum)]
enum Stream {
    Lts,
    Supported,
    Tech,
    Beta,
    Alpha,
}

impl From<Stream> for ReleaseStream {
    fn from(s: Stream) -> Self {
        match s {
            Stream::Lts => Self::Lts,
            Stream::Supported => Self::Supported,
            Stream::Tech => Self::Tech,
            Stream::Beta => Self::Beta,
            Stream::Alpha => Self::Alpha,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), UniHubError> {
    let unihub = UniHub::new()?;
    match cli.command {
        Cmd::Projects => {
            let installed: Vec<String> =
                unihub.list_editors().into_iter().map(|e| e.version).collect();
            let mut projects = unihub.list_projects();
            projects.sort_by_key(|p| std::cmp::Reverse(p.modified_ms));
            for p in projects {
                let version = p.editor_version.as_deref().unwrap_or("?");
                let mark = match (&p.editor_version, p.exists) {
                    (_, false) => " (missing)",
                    (Some(v), true) if !installed.contains(v) => " (editor not installed)",
                    _ => "",
                };
                let star = if p.is_favorite { "★ " } else { "  " };
                let vcs = p.source_control.as_ref().map_or(String::new(), |sc| {
                    format!("{} {}", sc.provider.display_name(), sc.branch.clone().unwrap_or_default())
                });
                println!("{star}{:<28} {:<13} {:<34} {}{mark}", p.title, version, vcs, p.path);
            }
        }
        Cmd::Add { path } => {
            let p = unihub.add_project(path)?;
            println!("added {} ({})", p.title, p.editor_version.as_deref().unwrap_or("?"));
        }
        Cmd::Remove { path } => unihub.remove_project(path)?,
        Cmd::Open { project, editor } => {
            let path = resolve_project(&unihub, &project)?;
            unihub.open_project(path.clone(), editor)?;
            println!("opening {path}");
        }
        Cmd::Info { project, size } => {
            let path = resolve_project(&unihub, &project)?;
            let d = unihub.project_details(path.clone())?;
            let show = |label: &str, value: Option<String>| {
                if let Some(v) = value.filter(|v| !v.is_empty()) {
                    println!("{label:<18} {v}");
                }
            };
            show("Path", Some(d.path.clone()));
            show("Product", d.product_name.clone());
            show("Company", d.company_name.clone());
            show("Version", d.version.clone());
            show("Render pipeline", d.render_pipeline.clone());
            show("Build target", d.build_target.clone());
            show("Scripting", Some(d.scripting_backends.join(", ")));
            show("Packages", Some(d.package_count.to_string()));
            show("Organization", d.organization.clone());
            show("Cloud project", d.cloud_project_name.clone());
            if let Some(sc) = &d.source_control {
                show("Source control", Some(sc.provider.display_name().to_string()));
                show("Repository", sc.owner.as_ref().zip(sc.repository.as_ref()).map(|(o, r)| format!("{o}/{r}")));
                show("Branch", sc.branch.clone());
                show("Web", sc.web_url.clone());
            }
            if let Some(g) = &d.git {
                show("Last commit", g.last_commit_hash.as_ref().map(|h| {
                    format!("{h} {} ({})", g.last_commit_subject.clone().unwrap_or_default(),
                            g.last_commit_author.clone().unwrap_or_default())
                }));
                show("Changes", Some(g.changed_files.to_string()));
                show("Upstream", g.upstream.as_ref().map(|u| {
                    format!("{u} (ahead {}, behind {})", g.ahead.unwrap_or(0), g.behind.unwrap_or(0))
                }));
            }
            if size {
                let s = unihub.project_size(path)?;
                println!("{:<18} {:.2} GB", "Size on disk", s.total as f64 / 1e9);
                for f in s.folders.iter().take(5) {
                    println!("  {:<16} {:.2} GB", f.name, f.bytes as f64 / 1e9);
                }
            }
        }
        Cmd::Clean { project, yes, all } => {
            let path = resolve_project(&unihub, &project)?;
            let plan = unihub.clean_plan(path.clone())?;
            let source = match plan.method {
                CleanMethod::GitIgnored => "files ignored by git",
                CleanMethod::UnityGenerated => "generated folders and files",
            };
            if plan.items.is_empty() {
                println!("nothing to clean ({source})");
                return Ok(());
            }
            let chosen: Vec<String> = plan
                .items
                .iter()
                .filter(|i| all || i.kind.regenerated())
                .map(|i| i.path.clone())
                .collect();
            for i in &plan.items {
                let keep = !(all || i.kind.regenerated());
                println!(
                    "{} {:>10}  {}{}{}",
                    if keep { " " } else { "-" },
                    human_bytes(i.bytes),
                    i.path,
                    if i.is_dir { "/" } else { "" },
                    if keep { "  (kept: not regenerated, use --all)" } else { "" }
                );
            }
            let bytes: u64 = plan.items.iter().filter(|i| chosen.contains(&i.path)).map(|i| i.bytes).sum();
            if !yes {
                println!("would free {} from {} items ({source}); run with --yes to delete", human_bytes(bytes), chosen.len());
                if plan.open_in_editor {
                    println!("the project is open in Unity: close it first");
                }
                return Ok(());
            }
            let result = unihub.clean_project(path, chosen)?;
            println!("removed {} items, freed {}", result.removed, human_bytes(result.freed_bytes));
            for f in &result.failed {
                eprintln!("could not remove {f}");
            }
        }
        Cmd::Editors => {
            for e in unihub.list_editors() {
                println!("{:<14} {:<10} {}", e.version, e.architecture, e.app_path);
            }
        }
        Cmd::Releases { stream, limit, version, recommended } => {
            let installed: Vec<String> =
                unihub.list_editors().into_iter().map(|e| e.version).collect();
            let page = match (stream, version) {
                _ if recommended => {
                    let releases = unihub.recommended_releases()?;
                    unihub_core::ReleasePage { total: releases.len() as u32, offset: 0, releases }
                }
                (None, Some(v)) => unihub.search_releases(v, limit, 0)?,
                (stream, version) => {
                    unihub.fetch_releases(stream.unwrap_or(Stream::Lts).into(), limit, 0, version)?
                }
            };
            if page.releases.is_empty() {
                println!("no releases found");
            } else {
                println!("{} releases in total, showing {}", page.total, page.releases.len());
            }
            for r in page.releases {
                let size = r.editor.as_ref().map_or(0, |d| d.download_size);
                let mut mark = String::new();
                if r.architecture == "x86_64" {
                    mark.push_str("  Intel");
                }
                if r.recommended {
                    mark.push_str("  recommended");
                }
                if installed.contains(&r.version) {
                    mark.push_str("  installed");
                }
                println!(
                    "{:<14} {:<9} {:<10} {:>7.2} GB  {} modules{mark}",
                    r.version,
                    r.stream,
                    r.release_date.get(..10).unwrap_or(""),
                    size as f64 / 1e9,
                    r.modules.len(),
                );
            }
        }
        Cmd::Install { version, arch, module, accept_terms } => {
            let arch = match arch {
                Some(a) => a,
                None => unihub
                    .search_releases(version.clone(), 25, 0)?
                    .releases
                    .into_iter()
                    .find(|r| r.version == version)
                    .map(|r| r.architecture)
                    .ok_or_else(|| UniHubError::InvalidData { message: format!("no macOS release {version}") })?,
            };
            let id = unihub.install_editor(version, arch, module, accept_terms)?;
            follow_job(&unihub, id)?;
        }
        Cmd::Uninstall { version, arch } => {
            unihub.uninstall_editor(version.clone(), arch)?;
            println!("uninstalled Unity {version}");
        }
        Cmd::Modules { version, arch } => {
            let modules = unihub.editor_modules(version, arch)?;
            for m in modules.iter().filter(|m| !m.hidden) {
                let indent = if m.parent_id.is_some() { "    " } else { "" };
                let terms = if m.terms.is_empty() { "" } else { "  (license terms)" };
                println!(
                    "{} {indent}{:<34} {:<36} {:>9}{terms}",
                    if m.installed { "✓" } else { " " },
                    m.id,
                    m.name,
                    human_bytes(m.download.download_size),
                );
            }
        }
        Cmd::ModuleAdd { version, module, arch, accept_terms } => {
            let id = unihub.install_modules(version, arch, module, accept_terms)?;
            follow_job(&unihub, id)?;
        }
        Cmd::ModuleRemove { version, module, arch } => {
            unihub.remove_modules(version, arch, module.clone())?;
            println!("removed {}", module.join(", "));
        }
        Cmd::Import => println!("imported {} projects", unihub.import_from_hub()?),
        Cmd::Account => {
            let status = unihub.auth_status()?;
            if status.signed_in {
                let who = status.email.or(status.name).unwrap_or_else(|| "unknown user".into());
                println!("signed in as {who}");
            } else {
                println!("not signed in (run `unihub login`)");
            }
            for account in unihub.accounts().unwrap_or_default() {
                let mark = if account.is_active { "*" } else { " " };
                let name = account.email.or(account.name).unwrap_or_default();
                println!("{mark} {:<36} {}", name, account.primary_org.unwrap_or_default());
            }
            if let Ok(license) = unihub.license_status() {
                for l in license.licenses {
                    println!("  license: {} ({})", l.product, l.kind.unwrap_or_default());
                }
            }
        }
        Cmd::Login => {
            println!("finish signing in in your browser…");
            let status = unihub.sign_in()?;
            println!("signed in as {}", status.email.or(status.name).unwrap_or_default());
        }
        Cmd::Logout { account } => {
            unihub.sign_out(account)?;
            println!("signed out");
        }
        Cmd::Tools { refresh } => {
            for p in unihub.plugins(refresh)? {
                let state = match (&p.installed_version, &p.newer_version) {
                    (Some(v), Some(n)) => format!("{v} (update: {n})"),
                    (Some(v), None) => v.clone(),
                    (None, _) => format!("not installed ({})", p.available_version.unwrap_or_default()),
                };
                println!("{:<18} {:<8} {:<34} {}", p.id, p.kind, p.display_name, state);
            }
        }
        Cmd::ToolInstall { id, accept_terms } => {
            unihub.install_plugin(id.clone(), accept_terms)?;
            println!("installed {id}");
        }
        Cmd::ToolRemove { id } => {
            unihub.remove_plugin(id.clone())?;
            println!("removed {id}");
        }
        Cmd::Cli => match unihub.unity_cli() {
            Some(info) => {
                println!("{} {} ({})", info.path, info.version, info.channel.unwrap_or_default());
                if let Ok(update) = unihub.check_unity_cli_update() {
                    match update.latest_version {
                        Some(latest) if update.update_available => println!("update available: {latest}"),
                        _ => println!("up to date"),
                    }
                }
            }
            None => println!("the Unity CLI is not installed (run `unihub cli-install`)"),
        },
        Cmd::CliInstall => {
            let info = unihub.install_unity_cli()?;
            println!("installed Unity CLI {} at {}", info.version, info.path);
        }
        Cmd::Settings => println!("{:#?}", unihub.settings()),
    }
    Ok(())
}

/// Prints a background install's progress until it ends.
fn follow_job(unihub: &UniHub, id: u64) -> Result<(), UniHubError> {
    let mut last = String::new();
    loop {
        let Some(job) = unihub.jobs().into_iter().find(|j| j.id == id) else { return Ok(()) };
        let line = match job.percent {
            Some(p) => format!("{} ({p:.0}%)", job.message),
            None => job.message.clone(),
        };
        if line != last {
            println!("{line}");
            last = line;
        }
        match job.state {
            JobState::Running => std::thread::sleep(std::time::Duration::from_millis(300)),
            JobState::Succeeded => return Ok(()),
            JobState::Cancelled => return Err(UniHubError::Cancelled),
            JobState::Failed => {
                return Err(UniHubError::CliFailed {
                    code: "INSTALL_FAILED".into(),
                    message: job.error.unwrap_or_else(|| "the install failed".into()),
                });
            }
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.2} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} KB", b as f64 / 1e3),
        b => format!("{b} B"),
    }
}

/// Accepts a path or a project title.
fn resolve_project(unihub: &UniHub, query: &str) -> Result<String, UniHubError> {
    let projects = unihub.list_projects();
    if let Some(p) = projects.iter().find(|p| p.path == query) {
        return Ok(p.path.clone());
    }
    let canonical = std::fs::canonicalize(query).ok().map(|p| p.to_string_lossy().into_owned());
    if let Some(p) = projects.iter().find(|p| Some(&p.path) == canonical.as_ref()) {
        return Ok(p.path.clone());
    }
    let by_title: Vec<_> = projects.iter().filter(|p| p.title.eq_ignore_ascii_case(query)).collect();
    match by_title.as_slice() {
        [one] => Ok(one.path.clone()),
        [] => Err(UniHubError::UnknownProject { path: query.to_string() }),
        _ => Err(UniHubError::InvalidData {
            message: format!("several projects are titled {query:?}; pass the path instead"),
        }),
    }
}
