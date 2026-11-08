use std::path::{Path, PathBuf};

/// Where UniHub keeps its data and where it looks for editors.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UniHubPaths {
    /// UniHub's own data folder (project list, settings).
    pub data_dir: String,
    /// The official Hub's data folder. Only ever read, to import projects and install paths.
    pub hub_data_dir: Option<String>,
    /// Folders that contain one sub-folder per editor version.
    pub editor_roots: Vec<String>,
}

pub const DEFAULT_EDITOR_ROOT: &str = "/Applications/Unity/Hub/Editor";

impl UniHubPaths {
    /// Standard macOS locations, plus the official Hub's custom install path if it set one.
    pub fn detect() -> Self {
        let app_support = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        let data_dir = app_support.join("UniHub");
        let hub_dir = app_support.join("UnityHub");
        let hub_data_dir = hub_dir.is_dir().then(|| path_string(&hub_dir));

        let mut editor_roots = vec![DEFAULT_EDITOR_ROOT.to_string()];
        if let Some(secondary) = read_secondary_install_path(&hub_dir) {
            if !editor_roots.contains(&secondary) {
                editor_roots.push(secondary);
            }
        }

        Self { data_dir: path_string(&data_dir), hub_data_dir, editor_roots }
    }

    pub fn data_dir(&self) -> PathBuf {
        PathBuf::from(&self.data_dir)
    }

    pub fn hub_data_dir(&self) -> Option<PathBuf> {
        self.hub_data_dir.as_ref().map(PathBuf::from)
    }
}

/// `secondaryInstallPath.json` holds a bare JSON string; `""` means "use the default".
fn read_secondary_install_path(hub_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(hub_dir.join("secondaryInstallPath.json")).ok()?;
    let path: String = serde_json::from_str(&text).ok()?;
    (!path.trim().is_empty()).then_some(path)
}

pub(crate) fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
