//! UniHub's own preferences, stored in `<data_dir>/settings.json`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Release channel used when installing or updating the Unity CLI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum CliChannel {
    /// Falls back to beta while the CLI has no stable release.
    #[default]
    Stable,
    Beta,
}

impl CliChannel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    /// Where new projects go. `None` means the user's Documents folder.
    pub default_projects_dir: Option<String>,
    /// Where editors are installed. `None` means `/Applications/Unity/Hub/Editor`.
    pub editor_install_dir: Option<String>,
    /// Where installers are downloaded. `None` means `<data_dir>/downloads`.
    pub downloads_dir: Option<String>,
    pub max_concurrent_downloads: u32,
    /// Extra folders to scan for editors, each holding one folder per version.
    pub extra_editor_dirs: Vec<String>,
    /// Add new projects from the official Hub's list each time UniHub starts.
    pub sync_hub_projects_on_launch: bool,
    /// Use this Unity CLI binary instead of looking for one.
    pub unity_cli_path: Option<String>,
    pub cli_channel: CliChannel,
    /// Hide the window when a project is opened.
    pub hide_on_project_open: bool,
    /// Confirm before removing a project from the list.
    pub ask_before_removing_project: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            default_projects_dir: None,
            editor_install_dir: None,
            downloads_dir: None,
            max_concurrent_downloads: 2,
            extra_editor_dirs: Vec::new(),
            sync_hub_projects_on_launch: true,
            unity_cli_path: None,
            cli_channel: CliChannel::Stable,
            hide_on_project_open: false,
            ask_before_removing_project: true,
        }
    }
}

impl Settings {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let file = data_dir.join("settings.json");
        if !file.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(file)?)?)
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        fs::create_dir_all(data_dir)?;
        let file = data_dir.join("settings.json");
        let tmp = file.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, &file)?;
        Ok(())
    }

    /// Normalises user input: blank paths become `None`, the download limit stays in 1…8.
    pub fn cleaned(mut self) -> Self {
        for p in [
            &mut self.default_projects_dir,
            &mut self.editor_install_dir,
            &mut self.downloads_dir,
            &mut self.unity_cli_path,
        ] {
            if p.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *p = None;
            }
        }
        self.extra_editor_dirs.retain(|d| !d.trim().is_empty());
        self.extra_editor_dirs.dedup();
        self.max_concurrent_downloads = self.max_concurrent_downloads.clamp(1, 8);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fills_missing_fields() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(tmp.path()).unwrap(), Settings::default());

        // Older files without newer fields still load.
        fs::write(tmp.path().join("settings.json"), r#"{"appearance":"dark"}"#).unwrap();
        let s = Settings::load(tmp.path()).unwrap();
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.max_concurrent_downloads, 2);

        let s = Settings { downloads_dir: Some("  ".into()), max_concurrent_downloads: 99, ..s }.cleaned();
        s.save(tmp.path()).unwrap();
        let loaded = Settings::load(tmp.path()).unwrap();
        assert_eq!(loaded.downloads_dir, None);
        assert_eq!(loaded.max_concurrent_downloads, 8);
    }
}
