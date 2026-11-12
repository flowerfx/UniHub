//! Connecting a project to Unity Cloud, and disconnecting it.
//!
//! A project is connected when the PlayerSettings block of
//! `ProjectSettings/ProjectSettings.asset` holds both `cloudProjectId` and `organizationId`
//! (`projectName` names the cloud project). Connecting fills in those three fields and
//! disconnecting empties them, the same edit Unity Hub makes; nothing on Unity's servers
//! changes. Organizations and cloud projects are listed through the official Unity CLI,
//! which holds the sign-in. See `docs/hub-behavior.md` §12.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::clean;
use crate::error::{Result, UniHubError};
use crate::unity_cli;

const SETTINGS_FILE: &str = "ProjectSettings/ProjectSettings.asset";
const PROJECT_ID_KEY: &str = "cloudProjectId";
const ORGANIZATION_KEY: &str = "organizationId";
const NAME_KEY: &str = "projectName";

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CloudOrganization {
    /// Unity Cloud id (a UUID).
    pub id: String,
    /// The older numeric id. Unity Hub writes this one into the project.
    pub genesis_id: Option<String>,
    pub name: String,
    /// e.g. `owner`, `manager`, `user`
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CloudProject {
    /// The cloud project id (a UUID), stored as `cloudProjectId`.
    pub id: String,
    pub name: String,
    pub organization_id: String,
    pub organization_genesis_id: Option<String>,
}

/// The project's connection as written in its settings file.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct CloudLink {
    pub cloud_project_id: Option<String>,
    /// As stored: the organization's numeric id, UUID or name, depending on what wrote it.
    pub organization_id: Option<String>,
    pub project_name: Option<String>,
}

impl CloudLink {
    /// Connected only when both ids are set, as in Unity Hub.
    pub fn is_connected(&self) -> bool {
        self.cloud_project_id.is_some() && self.organization_id.is_some()
    }
}

// ---- Reading and writing the settings file ------------------------------------------

pub fn read_link(project: &Path) -> CloudLink {
    fs::read_to_string(project.join(SETTINGS_FILE)).map(|t| parse_link(&t)).unwrap_or_default()
}

pub(crate) fn parse_link(settings: &str) -> CloudLink {
    CloudLink {
        cloud_project_id: field(settings, PROJECT_ID_KEY),
        organization_id: field(settings, ORGANIZATION_KEY),
        project_name: field(settings, NAME_KEY),
    }
}

/// The value of a PlayerSettings field, without YAML quotes; `None` when empty or absent.
fn field(settings: &str, key: &str) -> Option<String> {
    settings.lines().find_map(|line| value_of(line, key)).map(unquote).filter(|v| !v.is_empty())
}

/// `Some(raw value)` when `line` is `<indent><key>: <value>`.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(key)?.strip_prefix(':')?;
    Some(rest.trim())
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        value.to_string()
    }
}

/// Writes `value` as a plain YAML scalar, single-quoted when it would otherwise not read
/// back the same (as Unity itself does, e.g. `'Game: Remastered'`).
fn yaml_scalar(value: &str) -> String {
    let needs_quotes = value != value.trim()
        || value.contains(": ")
        || value.contains(" #")
        || value.ends_with(':')
        || value.starts_with(['!', '&', '*', '{', '}', '[', ']', ',', '#', '|', '>', '@', '`', '"', '\'', '%', '-', '?'])
        || value.contains(['\n', '\r', '\t']);
    if needs_quotes { format!("'{}'", value.replace('\'', "''")) } else { value.to_string() }
}

/// Replaces the value of every `key:` line, keeping its indentation and line ending. An
/// empty value leaves `key: `, which is how Unity writes an unset field. Returns the keys
/// that were not found.
pub(crate) fn rewrite_fields(settings: &str, values: &[(&str, &str)]) -> (String, Vec<String>) {
    let mut found = vec![false; values.len()];
    let mut out = String::with_capacity(settings.len() + 128);
    for line in settings.split_inclusive('\n') {
        let (body, ending) = match line.strip_suffix("\r\n") {
            Some(b) => (b, "\r\n"),
            None => line.strip_suffix('\n').map_or((line, ""), |b| (b, "\n")),
        };
        match values.iter().position(|(key, _)| value_of(body, key).is_some()) {
            Some(i) => {
                found[i] = true;
                let indent = &body[..body.len() - body.trim_start().len()];
                out.push_str(&format!("{indent}{}: {}{ending}", values[i].0, yaml_scalar(values[i].1)));
            }
            None => out.push_str(line),
        }
    }
    let missing = values.iter().zip(&found).filter(|(_, f)| !**f).map(|((k, _), _)| k.to_string()).collect();
    (out, missing)
}

fn write_link(project: &Path, values: &[(&str, &str)]) -> Result<()> {
    let file = project.join(SETTINGS_FILE);
    let settings = fs::read_to_string(&file)?;
    let (updated, missing) = rewrite_fields(&settings, values);
    if !missing.is_empty() {
        return Err(UniHubError::InvalidData {
            message: format!(
                "{} has no {} field. Open the project in Unity once so it updates the file, then try again.",
                SETTINGS_FILE,
                missing.join(", ")
            ),
        });
    }
    if updated == settings {
        return Ok(());
    }
    // Write next to the file and rename, so Unity or the Hub never read half a file.
    let tmp = file.with_extension("asset.unihub-tmp");
    fs::write(&tmp, &updated)?;
    if let Ok(meta) = fs::metadata(&file) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, &file)?;
    Ok(())
}

/// Cloud project ids are UUIDs (version 1–5), which Unity Hub checks before linking.
pub(crate) fn is_cloud_project_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    parts.len() == 5
        && parts.iter().zip(lengths).all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
        && matches!(parts[2].as_bytes()[0], b'1'..=b'5')
        && matches!(parts[3].as_bytes()[0].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

fn ensure_closed(project: &Path) -> Result<()> {
    if clean::open_in_editor(project) {
        return Err(UniHubError::ProjectInUse { path: project.to_string_lossy().into_owned() });
    }
    Ok(())
}

// ---- Connecting and disconnecting ---------------------------------------------------

/// Connects the project to `cloud`, writing what Unity Hub writes: the cloud project's
/// id and name, and the organization's numeric id (its UUID when there is none). Refused
/// while the project is open in Unity, which would write its own settings back.
pub fn connect(project: &Path, cloud: &CloudProject) -> Result<()> {
    if !is_cloud_project_id(&cloud.id) {
        return Err(UniHubError::InvalidData { message: format!("not a Unity Cloud project id: {}", cloud.id) });
    }
    ensure_closed(project)?;
    let organization = cloud.organization_genesis_id.as_deref().filter(|g| !g.is_empty()).unwrap_or(&cloud.organization_id);
    write_link(project, &[(ORGANIZATION_KEY, organization), (PROJECT_ID_KEY, &cloud.id), (NAME_KEY, &cloud.name)])
}

/// Disconnects the project. With the Unity CLI this first runs `unity projects unlink
/// cloud`, which empties the same fields and also clears the project's entry in Unity
/// Hub's list. The CLI refuses projects the Hub does not list; those, and Macs without
/// the CLI, get the edit directly. Refused while open in Unity.
///
/// A running Unity Hub keeps the ids it has in memory when a project's file loses them,
/// and saves them back to its list, so its list may go on showing the project as
/// connected (this happens with the Unity CLI alone too). The editor reads the file.
pub fn disconnect(project: &Path, cli: Option<&Path>) -> Result<()> {
    ensure_closed(project)?;
    if let Some(cli) = cli {
        // Its failure ("No registered project found…") is covered by the edit below.
        let _ = unity_cli::run_json(cli, &["projects", "unlink", "cloud", &project.to_string_lossy()]);
    }
    if read_link(project) != CloudLink::default() {
        write_link(project, &[(ORGANIZATION_KEY, ""), (PROJECT_ID_KEY, ""), (NAME_KEY, "")])?;
    }
    Ok(())
}

// ---- Listing organizations and cloud projects --------------------------------------

/// The signed-in user's organizations, by name.
pub fn organizations(cli: &Path) -> Result<Vec<CloudOrganization>> {
    let data = unity_cli::run_json(cli, &["cloud", "org"])?;
    let mut list = parse_organizations(&data);
    list.sort_by_key(|o| o.name.to_lowercase());
    Ok(list)
}

pub(crate) fn parse_organizations(data: &Value) -> Vec<CloudOrganization> {
    let list = data.get("organizations").or_else(|| data.get("items")).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|o| {
            Some(CloudOrganization {
                id: text(o, "id")?,
                genesis_id: text(o, "genesisId"),
                name: text(o, "name").unwrap_or_default(),
                role: text(o, "role"),
            })
        })
        .collect()
}

/// The organization's cloud projects that are not archived, by name.
pub fn projects(cli: &Path, organization_id: &str) -> Result<Vec<CloudProject>> {
    let data = unity_cli::run_json(cli, &["cloud", "project", "--cloud-org", organization_id])?;
    let mut list = parse_projects(&data);
    list.sort_by_key(|p| p.name.to_lowercase());
    Ok(list)
}

pub(crate) fn parse_projects(data: &Value) -> Vec<CloudProject> {
    let list = data.get("projects").or_else(|| data.get("items")).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter(|p| p.get("archivedAt").is_none_or(Value::is_null))
        .filter_map(|p| {
            Some(CloudProject {
                id: text(p, "id")?,
                name: text(p, "name").unwrap_or_default(),
                organization_id: text(p, "organizationId").unwrap_or_default(),
                organization_genesis_id: text(p, "organizationGenesisId"),
            })
        })
        .collect()
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The cloud part of a real `ProjectSettings.asset` (Unity 6), unset.
    const UNSET: &str = "PlayerSettings:\n  productName: Hexa Block\n  cloudProjectId: \n  framebufferDepthMemorylessMode: 0\n  qualitySettingsNames: []\n  projectName: \n  organizationId: \n  cloudEnabled: 0\n  legacyClampBlendShapeWeights: 0\n";

    fn cloud_project() -> CloudProject {
        CloudProject {
            id: "b478d020-1677-4f20-84c5-28017b3f3e34".into(),
            name: "Color Block Jam".into(),
            organization_id: "42a7e308-711a-4dcf-0000-1140717ce56c".into(),
            organization_genesis_id: Some("18968479589740".into()),
        }
    }

    fn make_project(settings: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Assets")).unwrap();
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        fs::write(dir.path().join(SETTINGS_FILE), settings).unwrap();
        dir
    }

    #[test]
    fn connects_like_the_hub_and_disconnects_back_to_the_original_bytes() {
        let dir = make_project(UNSET);
        assert!(!read_link(dir.path()).is_connected());

        connect(dir.path(), &cloud_project()).unwrap();
        let text = fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap();
        assert!(text.contains("\n  cloudProjectId: b478d020-1677-4f20-84c5-28017b3f3e34\n"));
        assert!(text.contains("\n  organizationId: 18968479589740\n"));
        assert!(text.contains("\n  projectName: Color Block Jam\n"));
        assert!(text.contains("\n  cloudEnabled: 0\n"), "other fields are left alone");
        let link = read_link(dir.path());
        assert!(link.is_connected());
        assert_eq!(link.project_name.as_deref(), Some("Color Block Jam"));

        disconnect(dir.path(), None).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap(), UNSET);
        assert_eq!(read_link(dir.path()), CloudLink::default());
    }

    #[test]
    fn falls_back_to_the_uuid_without_a_numeric_organization_id() {
        let dir = make_project(UNSET);
        let cloud = CloudProject { organization_genesis_id: None, ..cloud_project() };
        connect(dir.path(), &cloud).unwrap();
        assert_eq!(read_link(dir.path()).organization_id.as_deref(), Some("42a7e308-711a-4dcf-0000-1140717ce56c"));
    }

    #[test]
    fn keeps_crlf_line_endings_and_quotes_names_yaml_would_misread() {
        let crlf = UNSET.replace('\n', "\r\n");
        let (out, missing) = rewrite_fields(&crlf, &[(NAME_KEY, "Game: Remastered"), (PROJECT_ID_KEY, "x")]);
        assert!(missing.is_empty());
        assert!(out.contains("\r\n  projectName: 'Game: Remastered'\r\n"));
        assert!(out.contains("\r\n  cloudProjectId: x\r\n"));
        assert_eq!(out.matches("\r\n").count(), crlf.matches("\r\n").count());
        assert_eq!(parse_link(&out).project_name.as_deref(), Some("Game: Remastered"));
        assert_eq!(yaml_scalar("It's Game 2"), "It's Game 2");
        assert_eq!(yaml_scalar("Game #1"), "'Game #1'");
        assert_eq!(yaml_scalar("Bob's: game"), "'Bob''s: game'");
    }

    #[test]
    fn refuses_files_without_the_fields_and_bad_ids() {
        let dir = make_project("PlayerSettings:\n  productName: Old\n");
        let err = connect(dir.path(), &cloud_project()).unwrap_err();
        assert!(err.to_string().contains("cloudProjectId"), "{err}");
        assert_eq!(fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap(), "PlayerSettings:\n  productName: Old\n");

        let bad = CloudProject { id: "my-project".into(), ..cloud_project() };
        assert!(matches!(connect(dir.path(), &bad), Err(UniHubError::InvalidData { .. })));
        assert!(is_cloud_project_id("07d78893-480e-404d-b426-0e6dac7ef187"));
        assert!(!is_cloud_project_id("07d78893-480e-604d-b426-0e6dac7ef187"));
        assert!(!is_cloud_project_id("07d78893-480e-404d-c426-0e6dac7ef187"));
    }

    #[test]
    fn reads_what_the_editor_writes() {
        let link = parse_link("  cloudProjectId: 07d78893-480e-404d-b426-0e6dac7ef187\n  projectName: 'Block: Puzzle'\n  organizationId: kurubycoding\n");
        assert!(link.is_connected());
        assert_eq!(link.organization_id.as_deref(), Some("kurubycoding"));
        assert_eq!(link.project_name.as_deref(), Some("Block: Puzzle"));
        // A name alone is not a connection.
        assert!(!parse_link("  projectName: Game\n  cloudProjectId: \n  organizationId: \n").is_connected());
    }

    #[test]
    fn maps_the_cli_listings() {
        let orgs = parse_organizations(&json!({ "organizations": [
            { "id": "42a7e308-711a-4dcf-0000-1140717ce56c", "genesisId": "18968479589740", "name": "studio", "role": "owner" }
        ]}));
        assert_eq!(orgs[0].genesis_id.as_deref(), Some("18968479589740"));
        assert_eq!(orgs[0].role.as_deref(), Some("owner"));

        let projects = parse_projects(&json!({ "projects": [
            { "id": "b478d020-1677-4f20-84c5-28017b3f3e34", "name": "Live", "organizationId": "42a7e308",
              "organizationGenesisId": "18968479589740", "archivedAt": null },
            { "id": "5451a126-3ad2-4f3b-a7fa-163aa2afbedc", "name": "Old", "organizationId": "42a7e308",
              "archivedAt": "2026-01-01T00:00:00Z" }
        ]}));
        assert_eq!(projects.len(), 1, "archived projects are left out");
        assert_eq!(projects[0].name, "Live");
    }
}
