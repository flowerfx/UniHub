//! Editor release catalogue, from Unity's public release API:
//! <https://services.api.unity.com/unity/editor/release/v1/releases>

use serde::Deserialize;

use crate::error::Result;

pub const RELEASES_API: &str = "https://services.api.unity.com/unity/editor/release/v1/releases";

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReleaseStream {
    Lts,
    Supported,
    Tech,
    Beta,
    Alpha,
}

impl ReleaseStream {
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Lts => "LTS",
            Self::Supported => "SUPPORTED",
            Self::Tech => "TECH",
            Self::Beta => "BETA",
            Self::Alpha => "ALPHA",
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Release {
    pub version: String,
    pub revision: Option<String>,
    /// API stream name, e.g. `LTS`.
    pub stream: String,
    /// ISO-8601 timestamp.
    pub release_date: String,
    pub recommended: bool,
    /// `unityhub://<version>/<revision>`; opens this release in the official Hub.
    pub hub_deep_link: Option<String>,
    pub release_notes_url: Option<String>,
    /// Editor installer for this Mac: the Apple Silicon build, else the Intel one.
    /// `None` when the release has no macOS build at all.
    pub editor: Option<Download>,
    /// Architecture of `editor`: `arm64`, or `x86_64` for releases without an Apple
    /// Silicon build (they run with Rosetta).
    pub architecture: String,
    /// Unity no longer supports this release line (e.g. 2022.3 after May 2026).
    pub support_ended: bool,
    /// ISO-8601 end of support of the line, when published.
    pub end_of_life_date: Option<String>,
    pub modules: Vec<Module>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Download {
    pub url: String,
    /// `PKG`, `ZIP`, `DMG`, …
    pub kind: String,
    /// e.g. `md5-1djhIaHagavyAT750GazIg==`
    pub integrity: Option<String>,
    pub download_size: u64,
    pub installed_size: u64,
}

/// An editor module: a build platform, language pack, documentation or dev tool.
/// Used both for a release's catalogue and for an installed editor (`installed`).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub description: String,
    /// As published, e.g. `PLATFORM` (release API) or `Platforms` (modules.json).
    pub category: String,
    pub kind: ModuleKind,
    pub download: Download,
    /// Install target; may contain `{UNITY_PATH}` (the editor's install folder).
    pub destination: Option<String>,
    pub required: bool,
    /// Pulled in as a dependency of its parent and never listed on its own
    /// (e.g. the Android NDK under "Android SDK & NDK Tools").
    pub hidden: bool,
    pub pre_selected: bool,
    /// Id of the module this one belongs to, for sub-modules.
    pub parent_id: Option<String>,
    /// License terms the user must accept before this module is installed.
    pub terms: Vec<LicenseTerms>,
    /// Installed in the editor this list belongs to (always false in a release).
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ModuleKind {
    Platform,
    LanguagePack,
    Documentation,
    DevTool,
    Other,
}

impl ModuleKind {
    /// Accepts both spellings: `PLATFORM` / `Platforms`, `LANGUAGE_PACK` /
    /// `Language packs (Preview)`, `DEV_TOOL` / `Dev tools`, `DOCUMENTATION`.
    pub fn from_category(category: &str) -> Self {
        let c = category.to_ascii_lowercase().replace('_', " ");
        if c.starts_with("platform") {
            Self::Platform
        } else if c.starts_with("language") {
            Self::LanguagePack
        } else if c.starts_with("documentation") {
            Self::Documentation
        } else if c.starts_with("dev tool") {
            Self::DevTool
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LicenseTerms {
    pub label: String,
    pub url: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ReleasePage {
    pub total: u32,
    pub offset: u32,
    pub releases: Vec<Release>,
}

/// One page of macOS releases, newest first.
///
/// With a `stream`, lists that stream's Apple Silicon releases. With no `stream`, searches
/// every stream and also returns releases that only have an Intel build (2020.3 and
/// older, and their recent re-releases), so older versions can be found.
/// `version_prefix` matches the start of the version (`2022.3`, `6000.0.8`).
pub fn fetch(
    http: &reqwest::blocking::Client,
    stream: Option<ReleaseStream>,
    limit: u32,
    offset: u32,
    version_prefix: Option<&str>,
) -> Result<ReleasePage> {
    let mut query: Vec<(&str, String)> = vec![
        ("platform", "MAC_OS".into()),
        ("order", "RELEASE_DATE_DESC".into()),
        ("limit", limit.clamp(1, 25).to_string()),
        ("offset", offset.to_string()),
    ];
    if let Some(stream) = stream {
        query.push(("stream", stream.api_name().into()));
        query.push(("architecture", "ARM64".into()));
    }
    if let Some(v) = version_prefix.map(str::trim).filter(|v| !v.is_empty()) {
        query.push(("version", v.to_string()));
    }
    let response = http.get(RELEASES_API).query(&query).send()?;
    // The API answers 400/404 for searches it cannot match (e.g. letters): no releases.
    if matches!(response.status().as_u16(), 400 | 404) {
        return Ok(ReleasePage { total: 0, offset, releases: Vec::new() });
    }
    let page: ApiPage = response.error_for_status()?.json()?;
    Ok(page.into())
}

/// What the Hub lists as "Official releases": the newest release of each Supported and
/// LTS line whose support has not ended, newest line first (e.g. 6000.6 Supported,
/// 6000.3 LTS, 6000.0 LTS). Unity flags one of them `recommended`.
pub fn recommended(http: &reqwest::blocking::Client) -> Result<Vec<Release>> {
    let mut lines = std::collections::HashSet::new();
    let mut out = Vec::new();
    for stream in [ReleaseStream::Supported, ReleaseStream::Lts] {
        // Lines still supported have a recent release, well within the first 50.
        for page in 0..2 {
            let releases = fetch(http, Some(stream), 25, page * 25, None)?.releases;
            let count = releases.len();
            for r in releases {
                // The first release seen of a line is its newest; older ones are skipped,
                // and so is the whole line when its support has ended.
                if lines.insert(release_line(&r.version)) && !r.support_ended {
                    out.push(r);
                }
            }
            if count < 25 {
                break;
            }
        }
    }
    out.sort_by(|a, b| crate::version::compare(&b.version, &a.version));
    Ok(out)
}

/// `6000.3.28f1` → `6000.3`.
pub fn release_line(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// The release with exactly this version, if the API has it.
pub fn find(http: &reqwest::blocking::Client, version: &str) -> Result<Option<Release>> {
    let page = fetch(http, None, 25, 0, Some(version))?;
    Ok(page.releases.into_iter().find(|r| r.version == version))
}

// ---- Wire format ---------------------------------------------------------------

/// `null` reads as the default value (Unity's JSON writes `"eula": null`, for example).
pub(crate) fn nullable<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
pub(crate) struct ApiPage {
    #[serde(default, deserialize_with = "nullable")]
    offset: u32,
    #[serde(default, deserialize_with = "nullable")]
    total: u32,
    #[serde(default, deserialize_with = "nullable")]
    results: Vec<ApiRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiRelease {
    version: String,
    #[serde(default, deserialize_with = "nullable")]
    release_date: String,
    #[serde(default, deserialize_with = "nullable")]
    stream: String,
    short_revision: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    recommended: bool,
    unity_hub_deep_link: Option<String>,
    release_notes: Option<ApiLink>,
    #[serde(default, deserialize_with = "nullable")]
    downloads: Vec<ApiDownload>,
    #[serde(default, deserialize_with = "nullable")]
    has_support_ended: bool,
    end_of_life_date: Option<String>,
}

#[derive(Deserialize)]
struct ApiLink {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiDownload {
    #[serde(default, deserialize_with = "nullable")]
    url: String,
    #[serde(rename = "type", default, deserialize_with = "nullable")]
    kind: String,
    #[serde(default, deserialize_with = "nullable")]
    platform: String,
    #[serde(default, deserialize_with = "nullable")]
    architecture: String,
    integrity: Option<String>,
    download_size: Option<ApiSize>,
    installed_size: Option<ApiSize>,
    #[serde(default, deserialize_with = "nullable")]
    modules: Vec<ApiModule>,
}

/// Sizes are usually whole bytes, but some releases publish fractions (658719121.2).
#[derive(Deserialize)]
struct ApiSize {
    #[serde(default, deserialize_with = "nullable")]
    value: f64,
}

#[derive(Deserialize)]
struct ApiEula {
    #[serde(default, deserialize_with = "nullable")]
    url: String,
    #[serde(default, deserialize_with = "nullable")]
    label: String,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiModule {
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
    download_size: Option<ApiSize>,
    installed_size: Option<ApiSize>,
    destination: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    required: bool,
    #[serde(default, deserialize_with = "nullable")]
    hidden: bool,
    #[serde(default, deserialize_with = "nullable")]
    pre_selected: bool,
    #[serde(default, deserialize_with = "nullable")]
    sub_modules: Vec<ApiModule>,
    #[serde(default, deserialize_with = "nullable")]
    eula: Vec<ApiEula>,
}

fn size(s: &Option<ApiSize>) -> u64 {
    s.as_ref().map_or(0, |s| s.value.max(0.0).round() as u64)
}

impl From<ApiPage> for ReleasePage {
    fn from(p: ApiPage) -> Self {
        Self { total: p.total, offset: p.offset, releases: p.results.into_iter().map(Into::into).collect() }
    }
}

impl From<ApiRelease> for Release {
    fn from(r: ApiRelease) -> Self {
        let mut downloads: Vec<ApiDownload> =
            r.downloads.into_iter().filter(|d| d.platform == "MAC_OS").collect();
        // Apple Silicon first, then Intel.
        downloads.sort_by_key(|d| if d.architecture == "ARM64" { 0 } else { 1 });
        let download = downloads.into_iter().find(|d| matches!(d.architecture.as_str(), "ARM64" | "X86_64"));
        let architecture = match download.as_ref().map(|d| d.architecture.as_str()) {
            Some("X86_64") => "x86_64",
            _ => "arm64",
        }
        .to_string();
        let (editor, modules) = match download {
            Some(d) => {
                let mut modules = Vec::new();
                flatten_modules(d.modules, None, &mut modules);
                let editor = Download {
                    url: d.url,
                    kind: d.kind,
                    integrity: d.integrity,
                    download_size: size(&d.download_size),
                    installed_size: size(&d.installed_size),
                };
                (Some(editor), modules)
            }
            None => (None, Vec::new()),
        };
        Self {
            version: r.version,
            revision: r.short_revision,
            stream: r.stream,
            release_date: r.release_date,
            recommended: r.recommended,
            hub_deep_link: r.unity_hub_deep_link,
            release_notes_url: r.release_notes.map(|n| n.url),
            editor,
            architecture,
            support_ended: r.has_support_ended,
            end_of_life_date: r.end_of_life_date,
            modules,
        }
    }
}

fn flatten_modules(src: Vec<ApiModule>, parent: Option<&str>, out: &mut Vec<Module>) {
    for m in src {
        let id = m.id.clone();
        out.push(Module {
            id: m.id,
            name: m.name,
            description: m.description,
            kind: ModuleKind::from_category(&m.category),
            category: m.category,
            download: Download {
                url: m.url,
                kind: m.kind,
                integrity: m.integrity,
                download_size: size(&m.download_size),
                installed_size: size(&m.installed_size),
            },
            destination: m.destination,
            required: m.required,
            hidden: m.hidden,
            pre_selected: m.pre_selected,
            parent_id: parent.map(str::to_string),
            terms: m
                .eula
                .into_iter()
                .filter(|e| !e.url.is_empty() || !e.label.is_empty())
                .map(|e| LicenseTerms { label: e.label, url: e.url, message: e.message })
                .collect(),
            installed: false,
        });
        flatten_modules(m.sub_modules, Some(&id), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_api_response() {
        let json = serde_json::json!({
            "offset": 0, "limit": 1, "total": 209,
            "results": [{
                "version": "6000.0.85f2",
                "releaseDate": "2026-11-03T16:15:34.550Z",
                "stream": "LTS",
                "shortRevision": "b028303d8e7d",
                "recommended": false,
                "unityHubDeepLink": "unityhub://6000.0.85f2/b028303d8e7d",
                "releaseNotes": { "url": "https://example.com/notes.md", "type": "MD" },
                "downloads": [{
                    "url": "https://example.com/Unity.pkg", "type": "PKG",
                    "platform": "MAC_OS", "architecture": "ARM64",
                    "integrity": "md5-abc", "downloadSize": { "value": 10, "unit": "BYTE" },
                    "installedSize": { "value": 20, "unit": "BYTE" },
                    "modules": [{
                        "id": "android", "name": "Android Build Support", "category": "PLATFORM",
                        "url": "https://example.com/android.pkg", "type": "PKG",
                        "destination": "{UNITY_PATH}/PlaybackEngines",
                        "downloadSize": { "value": 5, "unit": "BYTE" },
                        "subModules": [{ "id": "android-sdk-ndk-tools", "url": "https://example.com/sdk.zip", "type": "ZIP" }]
                    }]
                }]
            }]
        });
        let page: ReleasePage = serde_json::from_value::<ApiPage>(json).unwrap().into();
        assert_eq!(page.total, 209);
        let r = &page.releases[0];
        assert_eq!(r.version, "6000.0.85f2");
        assert_eq!(r.editor.as_ref().unwrap().download_size, 10);
        assert_eq!(r.release_notes_url.as_deref(), Some("https://example.com/notes.md"));
        let ids: Vec<_> = r.modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["android", "android-sdk-ndk-tools"]);
        assert_eq!(r.modules[1].parent_id.as_deref(), Some("android"));
        assert_eq!(r.modules[0].kind, ModuleKind::Platform);
        assert_eq!(r.architecture, "arm64");
    }

    #[test]
    fn accepts_fractional_sizes_eula_and_intel_only_builds() {
        let json = serde_json::json!({
            "total": 1,
            "results": [{
                "version": "2019.4.41f2",
                "downloads": [
                    { "url": "https://example.com/win.exe", "platform": "WINDOWS", "architecture": "X86_64" },
                    {
                        "url": "https://example.com/Unity.pkg", "type": "PKG",
                        "platform": "MAC_OS", "architecture": "X86_64",
                        "downloadSize": { "value": 10.4, "unit": "BYTE" },
                        "modules": [{
                            "id": "documentation", "category": "DOCUMENTATION",
                            "installedSize": { "value": 658719121.2, "unit": "BYTE" },
                            "eula": [{ "url": "https://example.com/terms", "label": "Terms", "message": "Accept" }]
                        }]
                    }
                ]
            }]
        });
        let page: ReleasePage = serde_json::from_value::<ApiPage>(json).unwrap().into();
        let r = &page.releases[0];
        assert_eq!(r.architecture, "x86_64");
        assert_eq!(r.editor.as_ref().unwrap().url, "https://example.com/Unity.pkg");
        assert_eq!(r.editor.as_ref().unwrap().download_size, 10);
        assert_eq!(r.modules[0].download.installed_size, 658719121);
        assert_eq!(r.modules[0].kind, ModuleKind::Documentation);
        assert_eq!(r.modules[0].terms[0].label, "Terms");
    }

    #[test]
    fn reads_support_status_and_lines() {
        let json = serde_json::json!({
            "total": 1,
            "results": [{ "version": "2022.3.62f3", "hasSupportEnded": true,
                          "endOfLifeDate": "2026-05-08T00:00:00.000Z", "downloads": null }]
        });
        let page: ReleasePage = serde_json::from_value::<ApiPage>(json).unwrap().into();
        assert!(page.releases[0].support_ended);
        assert_eq!(page.releases[0].end_of_life_date.as_deref(), Some("2026-05-08T00:00:00.000Z"));
        assert!(page.releases[0].editor.is_none());
        assert_eq!(release_line("6000.3.28f1"), "6000.3");
        assert_eq!(release_line("2022.3.62f3"), "2022.3");
    }

    #[test]
    fn reads_both_category_spellings() {
        assert_eq!(ModuleKind::from_category("PLATFORM"), ModuleKind::Platform);
        assert_eq!(ModuleKind::from_category("Platforms"), ModuleKind::Platform);
        assert_eq!(ModuleKind::from_category("LANGUAGE_PACK"), ModuleKind::LanguagePack);
        assert_eq!(ModuleKind::from_category("Language packs (Preview)"), ModuleKind::LanguagePack);
        assert_eq!(ModuleKind::from_category("DEV_TOOL"), ModuleKind::DevTool);
        assert_eq!(ModuleKind::from_category("Dev tools"), ModuleKind::DevTool);
        assert_eq!(ModuleKind::from_category("Documentation"), ModuleKind::Documentation);
        assert_eq!(ModuleKind::from_category("Something"), ModuleKind::Other);
    }
}
