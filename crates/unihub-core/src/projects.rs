//! The project list. UniHub keeps its own list in `<data_dir>/projects.json` and can
//! import entries from the official Hub's `projects-v1.json`. It never writes to the
//! Hub's files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::cloud::{self, CloudLink};
use crate::error::{Result, UniHubError};
use crate::paths::path_string;
use crate::vcs::{self, SourceControl};

/// A project as shown in the UI. Built fresh from disk each time it is listed.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Project {
    pub path: String,
    pub title: String,
    /// From `ProjectSettings/ProjectVersion.txt`, e.g. `6000.0.85f1`.
    pub editor_version: Option<String>,
    /// Revision hash of that editor, e.g. `b731fd3ae857`.
    pub changeset: Option<String>,
    pub is_favorite: bool,
    /// Milliseconds since the Unix epoch.
    pub last_opened_ms: Option<i64>,
    /// Latest modification time of the project's top-level folders, in ms.
    pub modified_ms: Option<i64>,
    /// False when the folder is missing or no longer looks like a Unity project.
    pub exists: bool,
    /// macOS refused to let UniHub read the folder (privacy settings for external drives,
    /// Documents, Desktop…), so nothing about the project could be read.
    pub access_denied: bool,
    /// Git (GitHub, GitLab, other hosts), Unity Version Control or Perforce, if used.
    pub source_control: Option<SourceControl>,
    /// The Unity Cloud project it is connected to, from `ProjectSettings.asset`.
    pub cloud: CloudLink,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct StoredProject {
    pub is_favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_ms: Option<i64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    projects: BTreeMap<String, StoredProject>,
}

pub(crate) struct ProjectStore {
    file: PathBuf,
    projects: BTreeMap<String, StoredProject>,
}

impl ProjectStore {
    /// Loads the store. If it does not exist yet, seeds it from the official Hub.
    pub fn open(data_dir: &Path, hub_data_dir: Option<&Path>) -> Result<Self> {
        let file = data_dir.join("projects.json");
        if file.exists() {
            let parsed: StoreFile = serde_json::from_str(&fs::read_to_string(&file)?)?;
            return Ok(Self { file, projects: parsed.projects });
        }
        let mut store = Self { file, projects: BTreeMap::new() };
        if let Some(hub) = hub_data_dir {
            store.import_from_hub(hub)?;
        }
        store.save()?;
        Ok(store)
    }

    pub fn list(&self) -> Vec<Project> {
        self.projects.iter().map(|(path, stored)| build_project(path, stored)).collect()
    }

    pub fn get(&self, path: &str) -> Option<Project> {
        self.projects.get(path).map(|s| build_project(path, s))
    }

    pub fn add(&mut self, path: &str) -> Result<Project> {
        let path = normalize(path);
        if !is_unity_project(Path::new(&path)) {
            return Err(UniHubError::NotAProject { path });
        }
        self.projects.entry(path.clone()).or_default();
        self.save()?;
        Ok(self.get(&path).expect("just inserted"))
    }

    /// Adds a project that is still being created, so it is listed right away. `path` must
    /// already be absolute and canonical (the folder may not exist yet).
    pub fn add_pending(&mut self, path: &str, now_ms: i64) -> Result<()> {
        let entry = self.projects.entry(path.to_string()).or_default();
        entry.last_opened_ms = Some(now_ms);
        self.save()
    }

    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.projects
            .remove(path)
            .ok_or_else(|| UniHubError::UnknownProject { path: path.to_string() })?;
        self.save()
    }

    pub fn update(&mut self, path: &str, f: impl FnOnce(&mut StoredProject)) -> Result<()> {
        let entry = self
            .projects
            .get_mut(path)
            .ok_or_else(|| UniHubError::UnknownProject { path: path.to_string() })?;
        f(entry);
        self.save()
    }

    /// Adds every project the official Hub knows about that UniHub does not have yet.
    /// Returns how many were added.
    pub fn import_from_hub(&mut self, hub_data_dir: &Path) -> Result<u32> {
        let file = hub_data_dir.join("projects-v1.json");
        if !file.exists() {
            return Ok(0);
        }
        let parsed: HubProjectsFile = serde_json::from_str(&fs::read_to_string(&file)?)?;
        let mut added = 0;
        for (path, entry) in parsed.data {
            if self.projects.contains_key(&path) {
                continue;
            }
            self.projects.insert(
                path,
                StoredProject { is_favorite: entry.is_favorite, last_opened_ms: None },
            );
            added += 1;
        }
        if added > 0 {
            self.save()?;
        }
        Ok(added)
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let body = StoreFile { version: 1, projects: self.projects.clone() };
        let tmp = self.file.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&body)?)?;
        fs::rename(&tmp, &self.file)?;
        Ok(())
    }
}

/// The subset of the official Hub's `projects-v1.json` that UniHub reads.
#[derive(Deserialize)]
struct HubProjectsFile {
    data: BTreeMap<String, HubProjectEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubProjectEntry {
    #[serde(default)]
    is_favorite: bool,
}

fn normalize(path: &str) -> String {
    let p = Path::new(path.trim());
    let p = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    path_string(&p)
}

/// Reading the folder fails with "operation not permitted": a privacy refusal, not a
/// missing folder.
fn access_denied(dir: &Path) -> bool {
    matches!(fs::read_dir(dir), Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied)
}

pub fn is_unity_project(dir: &Path) -> bool {
    dir.join("Assets").is_dir() && dir.join("ProjectSettings").is_dir()
}

fn build_project(path: &str, stored: &StoredProject) -> Project {
    let dir = Path::new(path);
    let (editor_version, changeset) = read_project_version(dir).unzip();
    Project {
        path: path.to_string(),
        title: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        editor_version,
        changeset: changeset.flatten(),
        is_favorite: stored.is_favorite,
        last_opened_ms: stored.last_opened_ms,
        modified_ms: modified_ms(dir),
        exists: is_unity_project(dir),
        access_denied: access_denied(dir),
        source_control: vcs::detect(dir),
        cloud: cloud::read_link(dir),
    }
}

/// Reads `ProjectSettings/ProjectVersion.txt`:
/// ```text
/// m_EditorVersion: 6000.0.72f1
/// m_EditorVersionWithRevision: 6000.0.72f1 (b731fd3ae857)
/// ```
pub fn read_project_version(dir: &Path) -> Option<(String, Option<String>)> {
    let text = fs::read_to_string(dir.join("ProjectSettings/ProjectVersion.txt")).ok()?;
    parse_project_version(&text)
}

pub(crate) fn parse_project_version(text: &str) -> Option<(String, Option<String>)> {
    let mut version = None;
    let mut revision = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("m_EditorVersionWithRevision:") {
            let v = v.trim();
            if let Some((ver, rest)) = v.split_once(' ') {
                version = Some(ver.to_string());
                revision = Some(rest.trim_matches(|c| c == '(' || c == ')' || c == ' ').to_string());
            }
        } else if let Some(v) = line.strip_prefix("m_EditorVersion:") {
            version.get_or_insert_with(|| v.trim().to_string());
        }
    }
    version.filter(|v| !v.is_empty()).map(|v| (v, revision))
}

fn modified_ms(dir: &Path) -> Option<i64> {
    ["", "Assets", "ProjectSettings", "Packages"]
        .iter()
        .filter_map(|sub| fs::metadata(dir.join(sub)).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_project(root: &Path, name: &str, version: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(dir.join("Assets")).unwrap();
        fs::create_dir_all(dir.join("ProjectSettings")).unwrap();
        fs::write(
            dir.join("ProjectSettings/ProjectVersion.txt"),
            format!("m_EditorVersion: {version}\nm_EditorVersionWithRevision: {version} (abc123)\n"),
        )
        .unwrap();
        fs::canonicalize(dir).unwrap()
    }

    #[test]
    fn parses_project_version_file() {
        let parsed = parse_project_version("m_EditorVersion: 2022.3.1f1\n");
        assert_eq!(parsed, Some(("2022.3.1f1".into(), None)));
        let parsed = parse_project_version(
            "m_EditorVersion: 6000.0.72f1\nm_EditorVersionWithRevision: 6000.0.72f1 (b731fd3ae857)\n",
        );
        assert_eq!(parsed, Some(("6000.0.72f1".into(), Some("b731fd3ae857".into()))));
    }

    #[test]
    fn imports_from_hub_then_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let project = make_project(tmp.path(), "Game", "6000.0.85f1");
        let hub = tmp.path().join("hub");
        fs::create_dir_all(&hub).unwrap();
        fs::write(
            hub.join("projects-v1.json"),
            serde_json::json!({
                "schema_version": "v1",
                "data": { path_string(&project): { "isFavorite": true, "title": "Game" } }
            })
            .to_string(),
        )
        .unwrap();

        let data = tmp.path().join("data");
        let store = ProjectStore::open(&data, Some(&hub)).unwrap();
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Game");
        assert_eq!(list[0].editor_version.as_deref(), Some("6000.0.85f1"));
        assert_eq!(list[0].changeset.as_deref(), Some("abc123"));
        assert!(list[0].is_favorite && list[0].exists);

        // Reopening reads UniHub's own file, not the Hub's.
        fs::remove_file(hub.join("projects-v1.json")).unwrap();
        let mut store = ProjectStore::open(&data, Some(&hub)).unwrap();
        assert_eq!(store.list().len(), 1);

        store.remove(&path_string(&project)).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn lists_a_project_while_it_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::open(&tmp.path().join("data"), None).unwrap();
        let path = path_string(&fs::canonicalize(tmp.path()).unwrap().join("New"));
        store.add_pending(&path, 42).unwrap();
        let listed = store.get(&path).unwrap();
        assert!(!listed.exists);
        assert_eq!(listed.title, "New");
        assert_eq!(listed.last_opened_ms, Some(42));
        make_project(tmp.path(), "New", "6000.0.85f1");
        assert!(store.get(&path).unwrap().exists);
    }

    #[test]
    fn rejects_non_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::open(&tmp.path().join("data"), None).unwrap();
        let err = store.add(&path_string(tmp.path())).unwrap_err();
        assert!(matches!(err, UniHubError::NotAProject { .. }));
    }

    #[test]
    fn tells_a_refused_folder_from_a_missing_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("Locked");
        fs::create_dir_all(locked.join("Assets")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(access_denied(&locked));
        assert!(!access_denied(&dir.path().join("Missing")));
        assert!(!access_denied(dir.path()));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
