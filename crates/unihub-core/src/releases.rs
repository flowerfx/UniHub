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
    /// Editor installer for this Mac, or `None` when there is no Apple Silicon build.
    pub editor: Option<Download>,
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

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `PLATFORM`, `DEV_TOOL`, `LANGUAGE_PACK`, `DOCUMENTATION`, …
    pub category: String,
    pub download: Download,
    /// Install target; may contain `{UNITY_PATH}` (the editor's install folder).
    pub destination: Option<String>,
    pub required: bool,
    pub hidden: bool,
    pub pre_selected: bool,
    /// Id of the module this one belongs to, for sub-modules.
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ReleasePage {
    pub total: u32,
    pub offset: u32,
    pub releases: Vec<Release>,
}

pub fn fetch(
    http: &reqwest::blocking::Client,
    stream: ReleaseStream,
    limit: u32,
    offset: u32,
    version_prefix: Option<&str>,
) -> Result<ReleasePage> {
    let mut query: Vec<(&str, String)> = vec![
        ("platform", "MAC_OS".into()),
        ("architecture", "ARM64".into()),
        ("stream", stream.api_name().into()),
        ("order", "RELEASE_DATE_DESC".into()),
        ("limit", limit.clamp(1, 25).to_string()),
        ("offset", offset.to_string()),
    ];
    if let Some(v) = version_prefix.filter(|v| !v.is_empty()) {
        query.push(("version", v.to_string()));
    }
    let page: ApiPage = http.get(RELEASES_API).query(&query).send()?.error_for_status()?.json()?;
    Ok(page.into())
}

// ---- Wire format ---------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct ApiPage {
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    total: u32,
    results: Vec<ApiRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiRelease {
    version: String,
    #[serde(default)]
    release_date: String,
    #[serde(default)]
    stream: String,
    short_revision: Option<String>,
    #[serde(default)]
    recommended: bool,
    unity_hub_deep_link: Option<String>,
    release_notes: Option<ApiLink>,
    #[serde(default)]
    downloads: Vec<ApiDownload>,
}

#[derive(Deserialize)]
struct ApiLink {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiDownload {
    url: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    architecture: String,
    integrity: Option<String>,
    download_size: Option<ApiSize>,
    installed_size: Option<ApiSize>,
    #[serde(default)]
    modules: Vec<ApiModule>,
}

#[derive(Deserialize)]
struct ApiSize {
    value: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiModule {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    url: String,
    #[serde(rename = "type", default)]
    kind: String,
    integrity: Option<String>,
    download_size: Option<ApiSize>,
    installed_size: Option<ApiSize>,
    destination: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    pre_selected: bool,
    #[serde(default)]
    sub_modules: Vec<ApiModule>,
}

fn size(s: &Option<ApiSize>) -> u64 {
    s.as_ref().map_or(0, |s| s.value)
}

impl From<ApiPage> for ReleasePage {
    fn from(p: ApiPage) -> Self {
        Self { total: p.total, offset: p.offset, releases: p.results.into_iter().map(Into::into).collect() }
    }
}

impl From<ApiRelease> for Release {
    fn from(r: ApiRelease) -> Self {
        let download = r
            .downloads
            .into_iter()
            .find(|d| d.platform == "MAC_OS" && d.architecture == "ARM64");
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
    }
}
