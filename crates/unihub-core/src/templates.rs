//! Project templates for new projects.
//!
//! A template is a UPM package (`com.unity.template.*`) shipped as a `.tgz`. Each editor
//! has its built-in ones in `Unity.app/Contents/Resources/PackageManager/ProjectTemplates`;
//! templates downloaded by the Hub or the Unity CLI go to the Hub's `Templates` folder,
//! whose `manifest.json` says which editor each was downloaded for. The archive holds
//! `package/package.json` (name, displayName, version, description, dependencies) and the
//! project itself under `package/ProjectData~/`.
//!
//! When the Unity CLI is installed, `unity templates --editor-version <v> --json` gives the
//! full catalogue for that editor, with Unity's display names, categories, preview images
//! and the templates that still have to be downloaded. Without it, UniHub lists the
//! templates found on disk.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;

use crate::paths::path_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TemplateKind {
    /// Starting points: blank 2D/3D, URP, HDRP, VR, AR, multiplayer…
    Core,
    /// Sample projects showing off a feature or a pipeline.
    Sample,
    /// Microgames and learning projects.
    Learning,
    /// Templates the user made from their own projects.
    Custom,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Template {
    /// Package name, e.g. `com.unity.template.urp-blank`.
    pub id: String,
    /// e.g. "Universal 3D".
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub kind: TemplateKind,
    /// The template archive on disk, or `None` when it has to be downloaded first.
    pub archive_path: Option<String>,
    /// Download size when the template is not on disk yet.
    pub download_bytes: Option<u64>,
    /// Where to download it from (Unity's package registry), when it is not on disk yet.
    pub download_url: Option<String>,
    /// Published checksum of the download, e.g. `sha1-<base64>`.
    pub integrity: Option<String>,
    /// `URP`, `HDRP` or `BUILT_IN`, when known.
    pub render_pipeline: Option<String>,
    pub preview_image_url: Option<String>,
    pub icon_url: Option<String>,
    /// Build platforms the template is made for (`IOS`, `ANDROID`, `WEBGL`…), when known.
    pub build_platforms: Vec<String>,
    /// Unity marks it as new.
    pub is_new: bool,
}

impl Template {
    pub fn is_downloaded(&self) -> bool {
        self.archive_path.is_some()
    }
}

/// Folder of an editor's built-in templates.
pub fn builtin_dir(app_path: &Path) -> PathBuf {
    app_path.join("Contents/Resources/PackageManager/ProjectTemplates")
}

/// Templates on disk for an editor: its built-in ones, then the ones downloaded for it into
/// each of `download_dirs` (UniHub's template folder, the Hub's `Templates` folder).
/// Archives in `known` (already described by the catalogue) are not opened.
pub fn local(app_path: &Path, editor_version: &str, download_dirs: &[PathBuf], known: &HashSet<PathBuf>) -> Vec<Template> {
    let mut archives: Vec<PathBuf> = fs::read_dir(builtin_dir(app_path))
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| is_tgz(p)).collect())
        .unwrap_or_default();
    archives.sort();
    for dir in download_dirs {
        archives.extend(downloaded_archives(dir, editor_version).into_iter().map(|(_, _, p)| p));
    }

    let mut out: Vec<Template> = Vec::new();
    for archive in archives {
        if known.contains(&archive) {
            continue;
        }
        let Some(pkg) = read_package(&archive) else { continue };
        if pkg.kind.as_deref().is_some_and(|k| k != "template") {
            continue;
        }
        // Several archives of one template: keep the newest version.
        if let Some(existing) = out.iter_mut().find(|t| t.id == pkg.name) {
            if compare_versions(&pkg.version, &existing.version).is_gt() {
                *existing = template_from_package(pkg, &archive);
            }
            continue;
        }
        out.push(template_from_package(pkg, &archive));
    }
    out
}

/// The archive of template `id` on disk for an editor. Downloaded templates are found from
/// the manifests without opening them; built-in ones are read.
pub fn find_local(app_path: &Path, editor_version: &str, download_dirs: &[PathBuf], id: &str) -> Option<PathBuf> {
    let downloaded = download_dirs
        .iter()
        .flat_map(|dir| downloaded_archives(dir, editor_version))
        .filter(|(template, _, _)| template == id)
        .max_by(|a, b| compare_versions(&a.1, &b.1))
        .map(|(_, _, path)| path);
    downloaded.or_else(|| {
        local(app_path, editor_version, &[], &HashSet::new())
            .into_iter()
            .find(|t| t.id == id)
            .and_then(|t| t.archive_path.map(PathBuf::from))
    })
}

/// Parses `unity templates --json` output (`data.items`).
pub fn parse_cli(data: &Value) -> Vec<Template> {
    let Some(items) = data.get("items").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|t| {
            let id = str_field(t, "name")?;
            let archive = str_field(t, "tar").filter(|p| Path::new(p).is_file());
            let kind = match str_field(t, "type").as_deref().map(str::to_ascii_uppercase).as_deref() {
                Some("SAMPLE") => TemplateKind::Sample,
                Some("LEARNING") => TemplateKind::Learning,
                Some("CUSTOM") => TemplateKind::Custom,
                _ if t.get("isUserGenerated").and_then(Value::as_bool) == Some(true) => TemplateKind::Custom,
                _ => TemplateKind::Core,
            };
            let download_bytes = if archive.is_some() {
                None
            } else {
                number(t, "remoteSize").or_else(|| number(t, "size"))
            };
            let download_url = str_field(t, "remoteTar").filter(|_| archive.is_none());
            Some(Template {
                integrity: str_field(t, "integrity").filter(|_| download_url.is_some()),
                download_url,
                display_name: str_field(t, "displayName").unwrap_or_else(|| id.clone()),
                description: str_field(t, "description").unwrap_or_default(),
                version: str_field(t, "remoteVersion")
                    .filter(|_| archive.is_none())
                    .or_else(|| str_field(t, "version"))
                    .unwrap_or_default(),
                kind,
                archive_path: archive,
                download_bytes,
                render_pipeline: str_field(t, "renderPipeline"),
                preview_image_url: str_field(t, "previewImage"),
                icon_url: str_field(t, "dynamicIcon"),
                build_platforms: t
                    .get("buildPlatforms")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
                is_new: t.get("isNew").and_then(Value::as_bool).unwrap_or(false),
                id,
            })
        })
        .collect()
}

/// The catalogue first (its order is Unity's), then templates found only on disk.
pub fn merge(catalogue: Vec<Template>, local: Vec<Template>) -> Vec<Template> {
    let mut out = catalogue;
    for t in local {
        match out.iter_mut().find(|c| c.id == t.id) {
            // The catalogue does not know this archive (e.g. it lists another version).
            Some(c) if c.archive_path.is_none() && compare_versions(&t.version, &c.version).is_ge() => {
                c.archive_path = t.archive_path;
                c.version = t.version;
                c.download_bytes = None;
                c.download_url = None;
                c.integrity = None;
            }
            Some(_) => {}
            None => out.push(t),
        }
    }
    out
}

// ---- Reading template archives -----------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

fn template_from_package(pkg: PackageJson, archive: &Path) -> Template {
    let render_pipeline = if pkg.dependencies.contains_key("com.unity.render-pipelines.high-definition") {
        "HDRP"
    } else if pkg.dependencies.contains_key("com.unity.render-pipelines.universal") {
        "URP"
    } else {
        "BUILT_IN"
    };
    Template {
        display_name: pkg.display_name.filter(|n| !n.is_empty()).unwrap_or_else(|| pkg.name.clone()),
        description: pkg.description.unwrap_or_default(),
        version: pkg.version,
        kind: TemplateKind::Core,
        archive_path: Some(path_string(archive)),
        download_bytes: None,
        download_url: None,
        integrity: None,
        render_pipeline: Some(render_pipeline.into()),
        preview_image_url: None,
        icon_url: None,
        build_platforms: Vec::new(),
        is_new: false,
        id: pkg.name,
    }
}

type CacheKey = (PathBuf, u64, Option<SystemTime>);

/// Reading `package.json` means decompressing most of the archive (it is near the end), so
/// the result is kept for as long as the file does not change.
fn read_package(archive: &Path) -> Option<PackageJson> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Option<PackageJson>>>> = OnceLock::new();
    let meta = fs::metadata(archive).ok()?;
    let key = (archive.to_path_buf(), meta.len(), meta.modified().ok());
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return hit.clone();
    }
    let parsed = read_package_uncached(archive);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, parsed.clone());
    parsed
}

fn read_package_uncached(archive: &Path) -> Option<PackageJson> {
    let file = File::open(archive).ok()?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().ok()? {
        let entry = entry.ok()?;
        let path = entry.path().ok()?;
        // npm-style archives keep everything under `package/`.
        let is_manifest = path.components().count() == 2 && path.ends_with("package.json");
        if is_manifest {
            let mut text = String::new();
            entry.take(1 << 20).read_to_string(&mut text).ok()?;
            return serde_json::from_str(&text).ok();
        }
    }
    None
}

/// Archives the Hub or the Unity CLI downloaded for `editor_version`. `manifest.json` maps
/// each editor version to `{"dependencies": {"<template id>": "<version>"}}`, and the
/// archive is `<id>-<version>.tgz` next to it.
fn downloaded_archives(dir: &Path, editor_version: &str) -> Vec<(String, String, PathBuf)> {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(default)]
        dependencies: BTreeMap<String, String>,
    }
    let Ok(text) = fs::read_to_string(dir.join("manifest.json")) else { return Vec::new() };
    let Ok(manifest) = serde_json::from_str::<BTreeMap<String, Entry>>(&text) else { return Vec::new() };
    let Some(entry) = manifest.get(editor_version) else { return Vec::new() };
    entry
        .dependencies
        .iter()
        .map(|(id, version)| (id.clone(), version.clone(), dir.join(format!("{id}-{version}.tgz"))))
        .filter(|(_, _, p)| p.is_file())
        .collect()
}

fn is_tgz(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e == "tgz")
}

/// Template versions are semver (`17.0.12`, `2.0.0-pre.3`); pre-release tags sort first.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let (core, pre) = v.split_once('-').map_or((v, false), |(c, _)| (c, true));
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), !pre)
    }
    parts(a).cmp(&parts(b))
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn number(v: &Value, key: &str) -> Option<u64> {
    let n = v.get(key)?;
    n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)).or_else(|| n.get("value").and_then(|x| x.as_f64()).map(|f| f as u64))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes a template archive like Unity's: `package/package.json` plus project data.
    pub(crate) fn write_template(path: &Path, name: &str, display: &str, version: &str, deps: &[&str]) {
        let file = File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let add = |tar: &mut tar::Builder<_>, p: &str, body: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, p, body).unwrap();
        };
        add(&mut tar, "package/ProjectData~/ProjectSettings/ProjectSettings.asset", b"%YAML 1.1\n");
        add(&mut tar, "package/ProjectData~/Assets/Readme.asset", b"readme");
        let deps: serde_json::Map<String, Value> = deps.iter().map(|d| (d.to_string(), Value::from("1.0.0"))).collect();
        let pkg = serde_json::json!({
            "name": name, "displayName": display, "version": version, "type": "template",
            "description": format!("{display} template"), "dependencies": deps,
        });
        add(&mut tar, "package/package.json", pkg.to_string().as_bytes());
        tar.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn lists_builtin_and_downloaded_templates() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Unity.app");
        let builtin = builtin_dir(&app);
        fs::create_dir_all(&builtin).unwrap();
        write_template(
            &builtin.join("com.unity.template.urp-blank-17.0.12.tgz"),
            "com.unity.template.urp-blank",
            "3D URP",
            "17.0.12",
            &["com.unity.render-pipelines.universal"],
        );
        write_template(&builtin.join("com.unity.template.3d-9.0.0.tgz"), "com.unity.template.3d", "3D", "9.0.0", &[]);
        fs::write(builtin.join("notes.txt"), "not a template").unwrap();

        let hub = tmp.path().join("hub");
        let downloaded = hub.join("Templates");
        fs::create_dir_all(&downloaded).unwrap();
        write_template(&downloaded.join("com.unity.template.vr-9.2.3.tgz"), "com.unity.template.vr", "VR", "9.2.3", &[]);
        fs::write(
            downloaded.join("manifest.json"),
            r#"{"6000.0.85f1":{"dependencies":{"com.unity.template.vr":"9.2.3"}},"2022.3.1f1":{"dependencies":{"com.unity.template.gone":"1.0.0"}}}"#,
        )
        .unwrap();

        let dirs = [tmp.path().join("unihub-templates"), downloaded.clone()];
        let list = local(&app, "6000.0.85f1", &dirs, &HashSet::new());
        let ids: Vec<_> = list.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["com.unity.template.3d", "com.unity.template.urp-blank", "com.unity.template.vr"]);
        assert_eq!(list[1].display_name, "3D URP");
        assert_eq!(list[1].render_pipeline.as_deref(), Some("URP"));
        assert_eq!(list[0].render_pipeline.as_deref(), Some("BUILT_IN"));
        assert!(list.iter().all(Template::is_downloaded));

        // Another editor gets no downloaded templates.
        assert_eq!(local(&app, "2022.3.1f1", &dirs, &HashSet::new()).len(), 2);

        // Archives the catalogue already describes are skipped.
        let known: HashSet<PathBuf> = [builtin.join("com.unity.template.3d-9.0.0.tgz")].into();
        assert_eq!(local(&app, "6000.0.85f1", &dirs, &known).len(), 2);

        let vr = find_local(&app, "6000.0.85f1", &dirs, "com.unity.template.vr").unwrap();
        assert!(vr.ends_with("com.unity.template.vr-9.2.3.tgz"));
        let urp = find_local(&app, "6000.0.85f1", &dirs, "com.unity.template.urp-blank").unwrap();
        assert!(urp.ends_with("com.unity.template.urp-blank-17.0.12.tgz"));
        assert_eq!(find_local(&app, "2022.3.1f1", &dirs, "com.unity.template.vr"), None);
    }

    #[test]
    fn parses_the_cli_catalogue_and_merges_local_templates() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = tmp.path().join("urp.tgz");
        fs::write(&tgz, b"x").unwrap();
        let data = serde_json::json!({ "items": [
            { "name": "com.unity.template.urp-blank", "displayName": "Universal 3D", "type": "CORE",
              "version": "17.0.12", "status": "READY", "tar": tgz.to_string_lossy(), "size": 397248,
              "renderPipeline": "URP", "buildPlatforms": ["IOS", "MACOS"], "isNew": false },
            { "name": "com.unity.template.fps", "displayName": "FPS Microgame", "type": "LEARNING",
              "version": "5.1.1", "remoteVersion": "5.1.1", "status": "DOWNLOADABLE", "remoteSize": 86527396,
              "remoteTar": "https://download.packages.unity.com/com.unity.template.fps/-/com.unity.template.fps-5.1.1.tgz",
              "integrity": "sha1-abc", "previewImage": "https://example.com/fps.jpg" },
            { "name": "com.unity.template.cinematic", "displayName": "Cinematic", "type": "SAMPLE",
              "version": "1.6.1", "status": "DOWNLOADABLE", "size": {"value": 1000.5} },
            { "displayName": "no id" }
        ]});
        let catalogue = parse_cli(&data);
        assert_eq!(catalogue.len(), 3);
        assert_eq!(catalogue[0].kind, TemplateKind::Core);
        assert!(catalogue[0].is_downloaded());
        assert_eq!(catalogue[0].download_bytes, None);
        assert_eq!(catalogue[0].build_platforms, ["IOS", "MACOS"]);
        assert_eq!(catalogue[1].kind, TemplateKind::Learning);
        assert!(!catalogue[1].is_downloaded());
        assert_eq!(catalogue[1].download_bytes, Some(86527396));
        assert!(catalogue[1].download_url.as_deref().unwrap().ends_with("fps-5.1.1.tgz"));
        assert_eq!(catalogue[1].integrity.as_deref(), Some("sha1-abc"));
        assert_eq!(catalogue[0].download_url, None);
        assert_eq!(catalogue[2].kind, TemplateKind::Sample);
        assert_eq!(catalogue[2].download_bytes, Some(1000));

        let mut fps_on_disk = catalogue[1].clone();
        fps_on_disk.archive_path = Some("/tmp/fps.tgz".into());
        let mut extra = catalogue[0].clone();
        extra.id = "com.example.template.mine".into();
        let merged = merge(catalogue, vec![fps_on_disk, extra]);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[1].archive_path.as_deref(), Some("/tmp/fps.tgz"));
        assert_eq!(merged[1].download_bytes, None);
        assert_eq!(merged[1].download_url, None);
        assert_eq!(merged[3].id, "com.example.template.mine");
    }

    #[test]
    fn compares_template_versions() {
        assert!(compare_versions("17.0.12", "17.0.9").is_gt());
        assert!(compare_versions("2.0.0-pre.1", "2.0.0").is_lt());
        assert!(compare_versions("1.0.0", "1.0.0").is_eq());
    }
}
