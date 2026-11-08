use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use unihub_core::{UniHub, UniHubError, ReleaseStream};

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
    /// List installed editors.
    Editors,
    /// List editor releases available for Apple Silicon.
    Releases {
        #[arg(long, value_enum, default_value = "lts")]
        stream: Stream,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Only versions starting with this, e.g. 6000.0
        #[arg(long)]
        version: Option<String>,
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
        Cmd::Editors => {
            for e in unihub.list_editors() {
                println!("{:<14} {:<10} {}", e.version, e.architecture, e.app_path);
            }
        }
        Cmd::Releases { stream, limit, version } => {
            let installed: Vec<String> =
                unihub.list_editors().into_iter().map(|e| e.version).collect();
            let page = unihub.fetch_releases(stream.into(), limit, 0, version)?;
            println!("{} releases in total, showing {}", page.total, page.releases.len());
            for r in page.releases {
                let size = r.editor.as_ref().map_or(0, |d| d.download_size);
                let mark = if installed.contains(&r.version) { "  installed" } else { "" };
                println!(
                    "{:<14} {:<10} {:>7.2} GB  {} modules{mark}",
                    r.version,
                    r.release_date.get(..10).unwrap_or(""),
                    size as f64 / 1e9,
                    r.modules.len(),
                );
            }
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
