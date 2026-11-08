//! Everything shown in a project's details panel.
//!
//! Local files give the basics (ProjectSettings, manifest, git). When the official
//! Unity CLI is installed, `unity projects info` fills in what needs Unity's own
//! parsers, such as the active build target stored in binary Library files.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::unity_cli;
use crate::vcs::{self, GitStatus, SourceControl, VcsProvider};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectDetails {
    pub path: String,
    /// PlayerSettings `productName`.
    pub product_name: Option<String>,
    pub company_name: Option<String>,
    /// PlayerSettings `bundleVersion`.
    pub version: Option<String>,
    /// `Built-in`, `URP`, `HDRP` or `SRP`.
    pub render_pipeline: Option<String>,
    /// Active build target, e.g. `iOS`. Needs the Unity CLI.
    pub build_target: Option<String>,
    /// e.g. `Android: IL2CPP`. Needs the Unity CLI.
    pub scripting_backends: Vec<String>,
    pub package_count: u32,
    pub cloud_project_id: Option<String>,
    pub cloud_project_name: Option<String>,
    pub organization: Option<String>,
    pub source_control: Option<SourceControl>,
    pub git: Option<GitStatus>,
    /// True when the Unity CLI added to these details.
    pub from_unity_cli: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FolderSize {
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectSize {
    pub total: u64,
    /// Largest first.
    pub folders: Vec<FolderSize>,
}

pub fn load(project: &Path, cli: Option<&Path>) -> ProjectDetails {
    let settings = fs::read_to_string(project.join("ProjectSettings/ProjectSettings.asset")).unwrap_or_default();
    let manifest = fs::read_to_string(project.join("Packages/manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let source_control = vcs::detect(project);
    let git = source_control
        .as_ref()
        .filter(|sc| !matches!(sc.provider, VcsProvider::UnityVersionControl | VcsProvider::Perforce))
        .and_then(|sc| vcs::git_status(Path::new(&sc.root)));

    let mut details = ProjectDetails {
        path: project.to_string_lossy().into_owned(),
        product_name: yaml_value(&settings, "productName"),
        company_name: yaml_value(&settings, "companyName"),
        version: yaml_value(&settings, "bundleVersion"),
        render_pipeline: local_render_pipeline(project, manifest.as_ref()),
        build_target: None,
        scripting_backends: Vec::new(),
        package_count: manifest
            .as_ref()
            .and_then(|m| m.get("dependencies")?.as_object().map(|d| d.len() as u32))
            .unwrap_or(0),
        cloud_project_id: yaml_value(&settings, "cloudProjectId"),
        cloud_project_name: yaml_value(&settings, "projectName"),
        organization: yaml_value(&settings, "organizationId"),
        source_control,
        git,
        from_unity_cli: false,
    };

    if let Some(info) = cli.and_then(|c| unity_cli::run_json(c, &["projects", "info", &details.path]).ok()) {
        merge_cli_info(&mut details, &info);
    }
    details
}

fn merge_cli_info(details: &mut ProjectDetails, info: &Value) {
    let s = |k: &str| info.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    details.from_unity_cli = true;
    details.build_target = s("buildTarget").or(details.build_target.take());
    if let Some(rp) = s("renderPipeline") {
        details.render_pipeline = Some(normalize_pipeline(&rp));
    }
    if let Some(map) = info.get("scriptingBackend").and_then(Value::as_object) {
        details.scripting_backends =
            map.iter().filter_map(|(k, v)| Some(format!("{k}: {}", v.as_str()?))).collect();
    }
    if let Some(n) = info.get("packages").and_then(Value::as_object).map(|p| p.len() as u32) {
        details.package_count = n;
    }
    details.organization = s("organizationName").or(details.organization.take());
    details.cloud_project_name = s("projectName").or(details.cloud_project_name.take());
    details.cloud_project_id = s("cloudProjectId").or(details.cloud_project_id.take());
}

fn normalize_pipeline(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "builtin" | "built-in" | "built_in" => "Built-in".into(),
        "urp" => "URP".into(),
        "hdrp" => "HDRP".into(),
        "srp" => "SRP".into(),
        _ => value.to_string(),
    }
}

/// Reads `key: value` from a Unity YAML asset (first match, any indentation).
pub(crate) fn yaml_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    text.lines()
        .find_map(|l| l.trim_start().strip_prefix(&prefix).map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

/// Built-in unless GraphicsSettings or a quality level names a pipeline asset; then the
/// installed render-pipeline package tells URP from HDRP.
fn local_render_pipeline(project: &Path, manifest: Option<&Value>) -> Option<String> {
    let graphics = fs::read_to_string(project.join("ProjectSettings/GraphicsSettings.asset")).ok()?;
    let quality = fs::read_to_string(project.join("ProjectSettings/QualitySettings.asset")).unwrap_or_default();
    let uses_custom = |text: &str, key: &str| {
        text.lines().any(|l| {
            l.trim_start()
                .strip_prefix(key)
                .is_some_and(|v| v.contains("guid:") && !v.contains("fileID: 0}"))
        })
    };
    if !uses_custom(&graphics, "m_CustomRenderPipeline:") && !uses_custom(&quality, "customRenderPipeline:") {
        return Some("Built-in".into());
    }
    let deps = manifest.and_then(|m| m.get("dependencies")?.as_object());
    let has = |name: &str| deps.is_some_and(|d| d.contains_key(name));
    Some(if has("com.unity.render-pipelines.universal") {
        "URP".into()
    } else if has("com.unity.render-pipelines.high-definition") {
        "HDRP".into()
    } else {
        "SRP".into()
    })
}

/// Disk usage by top-level folder. Uses `unity projects size` when available, else walks
/// the folder itself (without following symlinks).
pub fn size(project: &Path, cli: Option<&Path>) -> ProjectSize {
    if let Some(data) = cli.and_then(|c| unity_cli::run_json(c, &["projects", "size", &project.to_string_lossy()]).ok()) {
        if let Some(total) = data.get("total").and_then(Value::as_u64) {
            let folders = data
                .get("breakdown")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|f| Some(FolderSize {
                            name: f.get("folder")?.as_str()?.to_string(),
                            bytes: f.get("bytes")?.as_u64()?,
                        }))
                        .collect()
                })
                .unwrap_or_default();
            return ProjectSize { total, folders };
        }
    }
    walk_size(project)
}

fn walk_size(project: &Path) -> ProjectSize {
    let mut folders = Vec::new();
    let mut loose = 0;
    if let Ok(entries) = fs::read_dir(project) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                folders.push(FolderSize {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    bytes: dir_size(&entry.path()),
                });
            } else if meta.is_file() {
                loose += meta.len();
            }
        }
    }
    if loose > 0 {
        folders.push(FolderSize { name: "(files)".into(), bytes: loose });
    }
    folders.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    ProjectSize { total: folders.iter().map(|f| f.bytes).sum(), folders }
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_project_settings_and_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        fs::create_dir_all(p.join("ProjectSettings")).unwrap();
        fs::create_dir_all(p.join("Packages")).unwrap();
        fs::write(
            p.join("ProjectSettings/ProjectSettings.asset"),
            "PlayerSettings:\n  companyName: Studio\n  productName: Puzzle\n  bundleVersion: 2.2.7\n  cloudProjectId: abc-123\n  organizationId: studio-org\n",
        )
        .unwrap();
        fs::write(p.join("ProjectSettings/GraphicsSettings.asset"), "  m_CustomRenderPipeline: {fileID: 11400000, guid: 1234, type: 2}\n").unwrap();
        fs::write(
            p.join("Packages/manifest.json"),
            r#"{"dependencies":{"com.unity.render-pipelines.universal":"17.0.3","com.unity.ugui":"2.0.0"}}"#,
        )
        .unwrap();

        let d = load(p, None);
        assert_eq!(d.product_name.as_deref(), Some("Puzzle"));
        assert_eq!(d.version.as_deref(), Some("2.2.7"));
        assert_eq!(d.cloud_project_id.as_deref(), Some("abc-123"));
        assert_eq!(d.render_pipeline.as_deref(), Some("URP"));
        assert_eq!(d.package_count, 2);
        assert!(!d.from_unity_cli);

        fs::write(p.join("ProjectSettings/GraphicsSettings.asset"), "  m_CustomRenderPipeline: {fileID: 0}\n").unwrap();
        assert_eq!(load(p, None).render_pipeline.as_deref(), Some("Built-in"));
    }

    #[test]
    fn merges_cli_info() {
        let mut d = load(Path::new("/nonexistent"), None);
        merge_cli_info(
            &mut d,
            &serde_json::json!({
                "buildTarget": "iOS", "renderPipeline": "URP",
                "scriptingBackend": { "Android": "IL2CPP" },
                "packages": { "a": "1", "b": "2", "c": "3" },
                "organizationName": "Kuruby"
            }),
        );
        assert_eq!(d.build_target.as_deref(), Some("iOS"));
        assert_eq!(d.scripting_backends, vec!["Android: IL2CPP".to_string()]);
        assert_eq!(d.package_count, 3);
        assert_eq!(d.organization.as_deref(), Some("Kuruby"));
        assert!(d.from_unity_cli);
    }

    #[test]
    fn walks_folder_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Assets/Sub")).unwrap();
        fs::write(tmp.path().join("Assets/Sub/a.bin"), vec![0u8; 300]).unwrap();
        fs::write(tmp.path().join("readme.txt"), vec![0u8; 20]).unwrap();
        let s = walk_size(tmp.path());
        assert_eq!(s.total, 320);
        assert_eq!(s.folders[0], FolderSize { name: "Assets".into(), bytes: 300 });
    }
}
