//! Modules of installed editors (build platforms, language packs, documentation).
//!
//! Editors installed by the Unity CLI or Unity Hub have `modules.json` next to
//! `Unity.app`: the module catalogue of that version, with `selected: true` on the modules
//! that are installed (the Unity CLI flips it when it adds or removes one). For editors
//! without it, installed platforms are read from the `PlaybackEngines` folder and
//! language packs from `Unity.app/Contents/Localization`.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::releases::{Download, LicenseTerms, Module, ModuleKind, nullable};

/// Every module of an installed editor, installed or not. Falls back to the installed
/// modules found on disk when there is no `modules.json`.
pub fn for_editor(install_dir: &Path) -> Vec<Module> {
    from_modules_json(install_dir).unwrap_or_else(|| scan_installed(install_dir))
}

/// True when `for_editor` came from `modules.json` (so it lists modules that can be added).
pub fn has_catalog(install_dir: &Path) -> bool {
    install_dir.join("modules.json").is_file()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonModule {
    id: String,
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    description: String,
    #[serde(default, deserialize_with = "nullable")]
    category: String,
    #[serde(default, deserialize_with = "nullable")]
    url: String,
    #[serde(rename = "type", default, deserialize_with = "nullable")]
    kind: String,
    integrity: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    download_size: f64,
    #[serde(default, deserialize_with = "nullable")]
    installed_size: f64,
    destination: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    required: bool,
    #[serde(default, deserialize_with = "nullable")]
    hidden: bool,
    visible: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    pre_selected: bool,
    #[serde(default, deserialize_with = "nullable")]
    parent: String,
    #[serde(default, deserialize_with = "nullable")]
    eula: Vec<JsonEula>,
    #[serde(default, deserialize_with = "nullable")]
    selected: bool,
}

#[derive(Deserialize)]
struct JsonEula {
    #[serde(default, deserialize_with = "nullable")]
    url: String,
    #[serde(default, deserialize_with = "nullable")]
    label: String,
    message: Option<String>,
}

fn from_modules_json(install_dir: &Path) -> Option<Vec<Module>> {
    let text = fs::read_to_string(install_dir.join("modules.json")).ok()?;
    let list: Vec<JsonModule> = serde_json::from_str(&text).ok()?;
    Some(
        list.into_iter()
            .map(|m| Module {
                kind: ModuleKind::from_category(&m.category),
                name: if m.name.is_empty() { m.id.clone() } else { m.name },
                id: m.id,
                description: m.description,
                category: m.category,
                download: Download {
                    url: m.url,
                    kind: m.kind,
                    integrity: m.integrity.filter(|i| !i.is_empty()),
                    download_size: m.download_size.max(0.0).round() as u64,
                    installed_size: m.installed_size.max(0.0).round() as u64,
                },
                destination: m.destination,
                required: m.required,
                hidden: m.hidden || m.visible == Some(false),
                pre_selected: m.pre_selected,
                parent_id: Some(m.parent).filter(|p| !p.is_empty()),
                terms: m
                    .eula
                    .into_iter()
                    .filter(|e| !e.url.is_empty() || !e.label.is_empty())
                    .map(|e| LicenseTerms { label: e.label, url: e.url, message: e.message })
                    .collect(),
                installed: m.selected,
            })
            .collect(),
    )
}

/// Installed modules recognised from folders, for editors without `modules.json`.
/// `MacStandaloneSupport` is left out: recent editors ship it without the IL2CPP module.
pub(crate) fn scan_installed(install_dir: &Path) -> Vec<Module> {
    const PLATFORMS: &[(&str, &str, &str)] = &[
        ("AndroidPlayer", "android", "Android Build Support"),
        ("iOSSupport", "ios", "iOS Build Support"),
        ("AppleTVSupport", "appletv", "tvOS Build Support"),
        ("VisionOSPlayer", "visionos", "visionOS Build Support"),
        ("WebGLSupport", "webgl", "Web Build Support"),
        ("WindowsStandaloneSupport", "windows-mono", "Windows Build Support"),
        ("LinuxStandaloneSupport", "linux-mono", "Linux Build Support"),
    ];
    const LANGUAGES: &[(&str, &str)] = &[
        ("ja", "日本語"),
        ("ko", "한국어"),
        ("zh-hans", "简体中文"),
        ("zh-hant", "繁體中文"),
    ];

    let mut out = Vec::new();
    let engines = install_dir.join("PlaybackEngines");
    for (folder, id, name) in PLATFORMS {
        if engines.join(folder).is_dir() {
            out.push(found(id, name, ModuleKind::Platform));
        }
    }
    if let Ok(entries) = fs::read_dir(install_dir.join("Unity.app/Contents/Localization")) {
        let mut codes: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".po").map(str::to_ascii_lowercase))
            .collect();
        codes.sort();
        for code in codes {
            let name = LANGUAGES.iter().find(|(c, _)| *c == code).map_or(code.as_str(), |(_, n)| n);
            out.push(found(&format!("language-{code}"), name, ModuleKind::LanguagePack));
        }
    }
    if install_dir.join("Documentation").is_dir() {
        out.push(found("documentation", "Documentation", ModuleKind::Documentation));
    }
    out
}

fn found(id: &str, name: &str, kind: ModuleKind) -> Module {
    Module {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        category: String::new(),
        kind,
        download: Download { url: String::new(), kind: String::new(), integrity: None, download_size: 0, installed_size: 0 },
        destination: None,
        required: false,
        hidden: false,
        pre_selected: false,
        parent_id: None,
        terms: Vec::new(),
        installed: true,
    }
}

/// Marks the modules of a release catalogue that `installed` (from `scan_installed`)
/// already has, for editors without `modules.json`.
pub fn merge_installed(catalogue: &mut [Module], installed: &[Module]) {
    for m in catalogue.iter_mut() {
        m.installed = installed.iter().any(|i| i.id == m.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_modules_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("modules.json"),
            r#"[
              {"id":"android","name":"Android Build Support","category":"Platforms","downloadSize":682957379,
               "installedSize":2167524000,"parent":"","selected":true,"visible":true,"eula":[]},
              {"id":"android-sdk-ndk-tools","name":"Android SDK & NDK Tools","category":"Platforms","parent":"android",
               "selected":true,"visible":true,
               "eula":[{"url":"https://example.com/sdk","label":"Android SDK and NDK License Terms from Google","message":"Accept"}]},
              {"id":"android-ndk-r27c","name":"Android NDK","category":"Platforms","parent":"android-sdk-ndk-tools",
               "hidden":true,"visible":false,"selected":true},
              {"id":"language-ja","name":"日本語","category":"Language packs (Preview)","installedSize":2936241.4,"selected":false,
               "eula":null,"description":null,"destination":null}
            ]"#,
        )
        .unwrap();
        let modules = for_editor(dir.path());
        assert!(has_catalog(dir.path()));
        assert_eq!(modules.len(), 4);
        assert!(modules[0].installed && modules[0].parent_id.is_none());
        assert_eq!(modules[0].kind, ModuleKind::Platform);
        assert_eq!(modules[1].parent_id.as_deref(), Some("android"));
        assert_eq!(modules[1].terms.len(), 1);
        assert!(modules[2].hidden);
        assert!(!modules[3].installed);
        assert_eq!(modules[3].kind, ModuleKind::LanguagePack);
        assert_eq!(modules[3].download.installed_size, 2936241);
    }

    #[test]
    fn scans_folders_without_modules_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("PlaybackEngines/iOSSupport")).unwrap();
        fs::create_dir_all(dir.path().join("PlaybackEngines/MacStandaloneSupport")).unwrap();
        fs::create_dir_all(dir.path().join("Unity.app/Contents/Localization")).unwrap();
        fs::write(dir.path().join("Unity.app/Contents/Localization/ja.po"), "").unwrap();
        let modules = for_editor(dir.path());
        assert!(!has_catalog(dir.path()));
        let ids: Vec<_> = modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["ios", "language-ja"]);
        assert!(modules.iter().all(|m| m.installed));
        assert_eq!(modules[1].name, "日本語");
    }
}
