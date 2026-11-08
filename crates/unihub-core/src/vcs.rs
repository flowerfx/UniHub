//! Source control detection for projects: GitHub, GitLab, other git hosts,
//! Unity Version Control and Perforce.
//!
//! `detect` only reads small files (`.git/HEAD`, `.git/config`, `.plastic/plastic.selector`)
//! so it is cheap enough to run for every row of the project list. `git_status` runs the
//! `git` binary and is meant for the details panel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::paths::path_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VcsProvider {
    GitHub,
    GitLab,
    /// Git with another host (Bitbucket, Gitea, self-hosted…) or no remote at all.
    Git,
    UnityVersionControl,
    Perforce,
}

impl VcsProvider {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Git => "Git",
            Self::UnityVersionControl => "Unity Version Control",
            Self::Perforce => "Perforce",
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SourceControl {
    pub provider: VcsProvider,
    /// Current branch, or a short commit hash when the HEAD is detached.
    pub branch: Option<String>,
    pub detached: bool,
    /// Repository root, which may be above the project folder.
    pub root: String,
    /// Project folder relative to the root; empty when they are the same.
    pub path_in_repository: String,
    pub remote_name: Option<String>,
    /// Remote URL with any credentials removed.
    pub remote_url: Option<String>,
    pub host: Option<String>,
    /// Owner / group path, e.g. `KurubyGames` or `group/subgroup`.
    pub owner: Option<String>,
    pub repository: Option<String>,
    /// Links to the hosting site, when the host is known to serve them.
    pub web_url: Option<String>,
    pub branch_url: Option<String>,
    /// Pull requests (GitHub) or merge requests (GitLab).
    pub reviews_url: Option<String>,
    pub commits_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GitStatus {
    pub last_commit_hash: Option<String>,
    pub last_commit_subject: Option<String>,
    pub last_commit_author: Option<String>,
    /// ISO-8601.
    pub last_commit_date: Option<String>,
    /// Files with uncommitted changes, including untracked ones.
    pub changed_files: u32,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

/// Finds the source control system a project uses, if any.
pub fn detect(project: &Path) -> Option<SourceControl> {
    if let Some(sc) = detect_git(project) {
        return Some(sc);
    }
    if let Some(sc) = detect_plastic(project) {
        return Some(sc);
    }
    detect_perforce(project)
}

// ---- Git ------------------------------------------------------------------------

struct GitDirs {
    root: PathBuf,
    /// Holds HEAD (differs from `common` in a linked worktree).
    git_dir: PathBuf,
    /// Holds config.
    common_dir: PathBuf,
}

/// The working tree that contains `project`, if any (reads `.git` only).
pub(crate) fn git_root(project: &Path) -> Option<PathBuf> {
    find_git(project).map(|d| d.root)
}

fn find_git(project: &Path) -> Option<GitDirs> {
    for dir in project.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(GitDirs { root: dir.to_path_buf(), git_dir: dot_git.clone(), common_dir: dot_git });
        }
        if dot_git.is_file() {
            // Linked worktree or submodule: ".git" holds "gitdir: <path>".
            let text = fs::read_to_string(&dot_git).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = dir.join(target);
            let common_dir = fs::read_to_string(git_dir.join("commondir"))
                .map(|c| git_dir.join(c.trim()))
                .unwrap_or_else(|_| git_dir.clone());
            return Some(GitDirs { root: dir.to_path_buf(), git_dir, common_dir });
        }
    }
    None
}

fn detect_git(project: &Path) -> Option<SourceControl> {
    let dirs = find_git(project)?;
    let head = fs::read_to_string(dirs.git_dir.join("HEAD")).unwrap_or_default();
    let (branch, detached) = match head.trim().strip_prefix("ref:") {
        Some(r) => (Some(r.trim().trim_start_matches("refs/heads/").to_string()), false),
        None if head.trim().len() >= 7 => (Some(head.trim()[..7].to_string()), true),
        None => (None, false),
    };

    let config = fs::read_to_string(dirs.common_dir.join("config")).unwrap_or_default();
    let remotes = parse_remotes(&config);
    let chosen = remotes
        .iter()
        .find(|(name, _)| name == "origin")
        .or_else(|| remotes.first())
        .cloned();

    let remote = chosen.as_ref().and_then(|(_, url)| parse_remote_url(url));
    let provider = match remote.as_ref().map(|r| r.host.to_ascii_lowercase()) {
        Some(h) if h == "github.com" || h.ends_with(".github.com") => VcsProvider::GitHub,
        Some(h) if h.contains("gitlab") => VcsProvider::GitLab,
        _ => VcsProvider::Git,
    };

    let path_in_repository = project
        .strip_prefix(&dirs.root)
        .map(|p| path_string(p))
        .unwrap_or_default();

    let mut sc = SourceControl {
        provider,
        branch: branch.clone(),
        detached,
        root: path_string(&dirs.root),
        path_in_repository,
        remote_name: chosen.as_ref().map(|(n, _)| n.clone()),
        remote_url: chosen.as_ref().map(|(_, u)| strip_credentials(u)),
        host: remote.as_ref().map(|r| r.host.clone()),
        owner: remote.as_ref().map(|r| r.owner.clone()),
        repository: remote.as_ref().map(|r| r.repo.clone()),
        web_url: None,
        branch_url: None,
        reviews_url: None,
        commits_url: None,
    };
    if let Some(r) = &remote {
        let base = format!("https://{}/{}/{}", r.host, r.owner, r.repo);
        let branch = branch.filter(|_| !detached);
        match provider {
            VcsProvider::GitHub => {
                sc.branch_url = branch.as_ref().map(|b| format!("{base}/tree/{b}"));
                sc.reviews_url = Some(format!("{base}/pulls"));
                sc.commits_url = Some(match &branch {
                    Some(b) => format!("{base}/commits/{b}"),
                    None => format!("{base}/commits"),
                });
            }
            VcsProvider::GitLab => {
                sc.branch_url = branch.as_ref().map(|b| format!("{base}/-/tree/{b}"));
                sc.reviews_url = Some(format!("{base}/-/merge_requests"));
                sc.commits_url = Some(match &branch {
                    Some(b) => format!("{base}/-/commits/{b}"),
                    None => format!("{base}/-/commits"),
                });
            }
            // Other hosts (Bitbucket Server, Gitea, …) use different URL layouts,
            // so no links are guessed for them.
            _ => {}
        }
        if matches!(provider, VcsProvider::GitHub | VcsProvider::GitLab) {
            sc.web_url = Some(base);
        }
    }
    Some(sc)
}

/// `[remote "origin"]` sections and their `url`, in file order.
pub(crate) fn parse_remotes(config: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current = line
                .strip_prefix("[remote")
                .and_then(|r| r.trim().strip_prefix('"'))
                .and_then(|r| r.split('"').next())
                .map(str::to_string);
        } else if let (Some(name), Some((key, value))) = (&current, line.split_once('=')) {
            if key.trim() == "url" {
                out.push((name.clone(), value.trim().to_string()));
            }
        }
    }
    out
}

#[derive(Debug, PartialEq)]
pub(crate) struct RemoteUrl {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

/// Understands `git@host:owner/repo.git`, `ssh://git@host:22/owner/repo.git` and
/// `https://user@host/group/sub/repo.git`.
pub(crate) fn parse_remote_url(url: &str) -> Option<RemoteUrl> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host_port, path) = rest.split_once('/')?;
        (host_port.split(':').next()?.to_string(), path.to_string())
    } else {
        // scp-like syntax: [user@]host:path
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    };
    let path = path.trim_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.rsplit_once('/')?;
    (!host.is_empty() && !owner.is_empty() && !repo.is_empty())
        .then(|| RemoteUrl { host, owner: owner.to_string(), repo: repo.to_string() })
}

/// Removes `user:password@` from http(s) URLs so tokens never reach the UI.
pub(crate) fn strip_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) if scheme.starts_with("http") => {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            format!("{scheme}://{host}/{path}")
        }
        _ => url.to_string(),
    }
}

/// Last commit, uncommitted changes and distance to upstream, using the `git` binary.
pub fn git_status(root: &Path) -> Option<GitStatus> {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0") // never take locks a running editor might need
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    };

    let log = git(&["log", "-1", "--format=%h%x1f%s%x1f%an%x1f%cI"]).unwrap_or_default();
    let mut parts = log.split('\u{1f}').map(|s| (!s.is_empty()).then(|| s.to_string()));
    let changed = git(&["status", "--porcelain=v1"]).map_or(0, |s| s.lines().count() as u32);
    let upstream = git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
    let (behind, ahead) = match upstream {
        Some(_) => git(&["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
            .and_then(|s| {
                let mut n = s.split_whitespace().map(|x| x.parse::<u32>().ok());
                Some((n.next()??, n.next()??))
            })
            .map_or((None, None), |(b, a)| (Some(b), Some(a))),
        None => (None, None),
    };
    Some(GitStatus {
        last_commit_hash: parts.next().flatten(),
        last_commit_subject: parts.next().flatten(),
        last_commit_author: parts.next().flatten(),
        last_commit_date: parts.next().flatten(),
        changed_files: changed,
        upstream,
        ahead,
        behind,
    })
}

// ---- Unity Version Control (Plastic SCM) ------------------------------------------

/// `.plastic/plastic.selector` looks like:
/// `repository "Repo@Org@cloud"` / `path "/"` / `smartbranch "/main"`.
fn detect_plastic(project: &Path) -> Option<SourceControl> {
    let root = project.ancestors().find(|d| d.join(".plastic").is_dir())?;
    let selector = fs::read_to_string(root.join(".plastic/plastic.selector")).unwrap_or_default();
    let quoted = |key: &str| -> Option<String> {
        selector.lines().find_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix(key)?.trim();
            Some(rest.trim_matches('"').to_string())
        })
    };
    let repo_spec = quoted("repository");
    let mut spec = repo_spec.as_deref().unwrap_or("").split('@');
    let repository = spec.next().filter(|s| !s.is_empty()).map(str::to_string);
    let owner = spec.next().filter(|s| !s.is_empty()).map(str::to_string);
    Some(SourceControl {
        provider: VcsProvider::UnityVersionControl,
        branch: quoted("smartbranch").or_else(|| quoted("branch")),
        detached: false,
        root: path_string(root),
        path_in_repository: project.strip_prefix(root).map(|p| path_string(p)).unwrap_or_default(),
        remote_name: None,
        remote_url: repo_spec,
        host: None,
        owner,
        repository,
        web_url: None,
        branch_url: None,
        reviews_url: None,
        commits_url: None,
    })
}

// ---- Perforce ---------------------------------------------------------------------

fn detect_perforce(project: &Path) -> Option<SourceControl> {
    let root = project
        .ancestors()
        .find(|d| d.join(".p4config").is_file() || d.join("p4config.txt").is_file())?;
    Some(SourceControl {
        provider: VcsProvider::Perforce,
        branch: None,
        detached: false,
        root: path_string(root),
        path_in_repository: project.strip_prefix(root).map(|p| path_string(p)).unwrap_or_default(),
        remote_name: None,
        remote_url: None,
        host: None,
        owner: None,
        repository: None,
        web_url: None,
        branch_url: None,
        reviews_url: None,
        commits_url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_urls() {
        let r = parse_remote_url("git@github.com:KurubyGames/BlastBlockPuzzle.git").unwrap();
        assert_eq!(r, RemoteUrl { host: "github.com".into(), owner: "KurubyGames".into(), repo: "BlastBlockPuzzle".into() });
        let r = parse_remote_url("https://user@github.com/flowerfx11/ColorBlockJam.git").unwrap();
        assert_eq!((r.owner.as_str(), r.repo.as_str()), ("flowerfx11", "ColorBlockJam"));
        let r = parse_remote_url("ssh://git@gitlab.example.com:2222/games/mobile/puzzle.git").unwrap();
        assert_eq!((r.host.as_str(), r.owner.as_str(), r.repo.as_str()), ("gitlab.example.com", "games/mobile", "puzzle"));
        assert!(parse_remote_url("not a url").is_none());
    }

    #[test]
    fn strips_credentials_from_https() {
        assert_eq!(
            strip_credentials("https://bob:ghp_secret@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(strip_credentials("git@github.com:a/b.git"), "git@github.com:a/b.git");
    }

    #[test]
    fn reads_remotes_from_config() {
        let cfg = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = git@github.com:o/u.git\n[remote \"origin\"]\n\turl = https://gitlab.com/g/r.git\n\tfetch = +refs/heads/*\n";
        assert_eq!(
            parse_remotes(cfg),
            vec![("upstream".into(), "git@github.com:o/u.git".into()), ("origin".into(), "https://gitlab.com/g/r.git".into())]
        );
    }

    #[test]
    fn detects_git_project_in_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature/shop\n").unwrap();
        fs::write(root.join(".git/config"), "[remote \"origin\"]\n\turl = git@gitlab.com:team/game.git\n").unwrap();
        let project = root.join("Client");
        fs::create_dir_all(&project).unwrap();

        let sc = detect(&project).unwrap();
        assert_eq!(sc.provider, VcsProvider::GitLab);
        assert_eq!(sc.branch.as_deref(), Some("feature/shop"));
        assert_eq!(sc.path_in_repository, "Client");
        assert_eq!(sc.web_url.as_deref(), Some("https://gitlab.com/team/game"));
        assert_eq!(sc.branch_url.as_deref(), Some("https://gitlab.com/team/game/-/tree/feature/shop"));
        assert_eq!(sc.reviews_url.as_deref(), Some("https://gitlab.com/team/game/-/merge_requests"));
    }

    #[test]
    fn detects_unity_version_control() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".plastic")).unwrap();
        fs::write(
            tmp.path().join(".plastic/plastic.selector"),
            "repository \"MyGame@studio@cloud\"\n  path \"/\"\n    smartbranch \"/main/dev\"\n",
        )
        .unwrap();
        let sc = detect(tmp.path()).unwrap();
        assert_eq!(sc.provider, VcsProvider::UnityVersionControl);
        assert_eq!(sc.repository.as_deref(), Some("MyGame"));
        assert_eq!(sc.owner.as_deref(), Some("studio"));
        assert_eq!(sc.branch.as_deref(), Some("/main/dev"));
    }
}
