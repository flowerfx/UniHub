//! Finding installed editors.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::modules;
use crate::paths::{UniHubPaths, path_string};
use crate::releases::Module;
use crate::version;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Editor {
    /// e.g. `6000.0.85f1`
    pub version: String,
    /// Revision hash, e.g. `ee7790f1426a`, when the bundle reports it.
    pub revision: Option<String>,
    /// Path to `Unity.app`.
    pub app_path: String,
    /// Path to the binary inside the bundle.
    pub executable_path: String,
    /// `arm64`, `x86_64`, `universal` or `unknown`.
    pub architecture: String,
    /// Folder holding this version, e.g. `/Applications/Unity/Hub/Editor/6000.0.85f1`.
    pub install_dir: String,
    /// Modules of this version from `modules.json` (installed ones have `installed`),
    /// or only the installed ones found on disk when there is no `modules.json`.
    pub modules: Vec<Module>,
}

/// Scans every editor root plus editors the official Hub lists as added by hand.
/// The result is sorted newest first, without duplicates.
pub fn discover(paths: &UniHubPaths) -> Vec<Editor> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in &paths.editor_roots {
        if let Ok(entries) = fs::read_dir(root) {
            candidates.extend(entries.flatten().map(|e| e.path().join("Unity.app")));
        }
    }
    if let Some(hub) = paths.hub_data_dir() {
        candidates.extend(hub_manual_editors(&hub));
    }

    let mut editors: Vec<Editor> = candidates.iter().filter_map(|app| inspect(app)).collect();
    editors.sort_by(|a, b| version::compare(&b.version, &a.version));
    editors.dedup_by(|a, b| a.app_path == b.app_path);
    editors
}

/// Reads one `Unity.app` bundle. Returns `None` if it is not an editor.
pub fn inspect(app: &Path) -> Option<Editor> {
    let executable = app.join("Contents/MacOS/Unity");
    if !executable.is_file() {
        return None;
    }
    let info: plist::Dictionary = plist::from_file(app.join("Contents/Info.plist")).ok()?;
    let get = |k: &str| info.get(k).and_then(|v| v.as_string()).map(str::to_string);

    let install_dir = app.parent().map(Path::to_path_buf).unwrap_or_default();
    let version = get("CFBundleVersion")
        .filter(|v| version::UnityVersion::parse(v).is_some())
        .or_else(|| install_dir.file_name().map(|n| n.to_string_lossy().into_owned()))?;

    let architecture = hub_metadata_arch(&install_dir)
        .unwrap_or_else(|| macho_arch(&executable).to_string());

    Some(Editor {
        version,
        revision: get("UnityBuildNumber"),
        app_path: path_string(app),
        executable_path: path_string(&executable),
        architecture,
        modules: modules::for_editor(&install_dir),
        install_dir: path_string(&install_dir),
    })
}

/// `metadata.hub.json` is written by the official Hub next to each editor it installs.
fn hub_metadata_arch(install_dir: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Meta {
        architecture: Option<String>,
    }
    let text = fs::read_to_string(install_dir.join("metadata.hub.json")).ok()?;
    serde_json::from_str::<Meta>(&text).ok()?.architecture.filter(|a| !a.is_empty())
}

/// Reads the Mach-O header to tell arm64 from x86_64 builds.
fn macho_arch(executable: &Path) -> &'static str {
    let mut header = [0u8; 8];
    let Ok(mut f) = File::open(executable) else { return "unknown" };
    if f.read_exact(&mut header).is_err() {
        return "unknown";
    }
    let magic = u32::from_be_bytes(header[0..4].try_into().unwrap());
    match magic {
        0xcafe_babe | 0xcafe_babf => "universal",
        0xcffa_edfe => match u32::from_le_bytes(header[4..8].try_into().unwrap()) {
            0x0100_000c => "arm64",
            0x0100_0007 => "x86_64",
            _ => "unknown",
        },
        _ => "unknown",
    }
}

/// `editors-v2.json` lists editors added with "Locate". Entries may be bare paths or
/// objects with a `location` (string or list), pointing at the `.app` or its folder.
fn hub_manual_editors(hub_dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(hub_dir.join("editors-v2.json")) else { return vec![] };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return vec![] };
    let Some(items) = json.get("data").and_then(|d| d.as_array()) else { return vec![] };

    let mut out = Vec::new();
    for item in items {
        let locations: Vec<&str> = match item {
            serde_json::Value::String(s) => vec![s.as_str()],
            serde_json::Value::Object(o) => match o.get("location") {
                Some(serde_json::Value::String(s)) => vec![s.as_str()],
                Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect(),
                _ => vec![],
            },
            _ => vec![],
        };
        for loc in locations {
            let p = PathBuf::from(loc);
            out.push(if p.extension().is_some_and(|e| e == "app") { p } else { p.join("Unity.app") });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_editor(root: &Path, version: &str) {
        let app = root.join(version).join("Unity.app/Contents");
        fs::create_dir_all(app.join("MacOS")).unwrap();
        // Minimal arm64 Mach-O header.
        let mut bin = 0xfeed_facf_u32.to_le_bytes().to_vec();
        bin.extend(0x0100_000c_u32.to_le_bytes());
        fs::write(app.join("MacOS/Unity"), bin).unwrap();
        fs::write(
            app.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleVersion</key><string>{version}</string>
<key>UnityBuildNumber</key><string>deadbeef</string>
</dict></plist>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn discovers_and_sorts_editors() {
        let tmp = tempfile::tempdir().unwrap();
        fake_editor(tmp.path(), "2022.3.10f1");
        fake_editor(tmp.path(), "6000.0.85f1");
        fs::create_dir_all(tmp.path().join("not-an-editor")).unwrap();

        let paths = UniHubPaths {
            data_dir: String::new(),
            hub_data_dir: None,
            editor_roots: vec![path_string(tmp.path())],
        };
        let editors = discover(&paths);
        let versions: Vec<_> = editors.iter().map(|e| e.version.as_str()).collect();
        assert_eq!(versions, ["6000.0.85f1", "2022.3.10f1"]);
        assert_eq!(editors[0].architecture, "arm64");
        assert_eq!(editors[0].revision.as_deref(), Some("deadbeef"));
    }
}
