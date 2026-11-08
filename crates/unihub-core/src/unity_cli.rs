//! Integration with Unity's official command-line tool (`unity`).
//!
//! UniHub never handles Unity credentials itself. Sign-in, accounts, licenses and
//! managed tools all go through the official CLI, which keeps tokens in the macOS
//! keychain. UniHub only runs it with `--json` and reads the result.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Result, UniHubError};
use crate::paths::path_string;

// ---- Public records -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CliInfo {
    pub path: String,
    pub version: String,
    /// `stable` or `beta`.
    pub channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CliUpdate {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AuthStatus {
    pub signed_in: bool,
    /// e.g. `fresh`, `expired`
    pub session_state: Option<String>,
    /// `oauth` for a person, `service-account` for a service account.
    pub mode: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Account {
    /// Stable account id (the CLI calls it `foreignKey`).
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub primary_org: Option<String>,
    pub is_active: bool,
    pub signed_in: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LicenseStatus {
    pub active: bool,
    pub signed_in: bool,
    pub licenses: Vec<License>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct License {
    /// e.g. `Unity Personal`
    pub product: String,
    /// e.g. `Assigned`, `Serial`, `Floating`
    pub kind: Option<String>,
    pub organization: Option<String>,
    pub expires: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Plugin {
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// `tool` (something you run) or `runtime` (used by the CLI itself).
    pub kind: String,
    /// e.g. `resolved`, `not-installed`
    pub state: String,
    pub installed: bool,
    pub path: Option<String>,
    /// Version on disk, when installed.
    pub installed_version: Option<String>,
    /// Version the CLI would install.
    pub available_version: Option<String>,
    /// Set when the registry has a newer build than the installed one.
    pub newer_version: Option<String>,
    /// Set when this plugin needs a newer CLI first.
    pub blocked_by_cli_version: Option<String>,
}

// ---- Locating the CLI -----------------------------------------------------------

/// Places the official installers put the binary. GUI apps do not inherit the
/// shell's PATH, so these are checked explicitly.
fn candidate_paths(override_path: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".unity/bin/unity"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/unity"));
    out.push(PathBuf::from("/usr/local/bin/unity"));
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|d| d.join("unity")));
    }
    out
}

/// Finds the Unity CLI, checking that the binary really is it (`unity version`).
pub fn locate(override_path: Option<&str>) -> Option<CliInfo> {
    for path in candidate_paths(override_path) {
        if !path.is_file() {
            continue;
        }
        let Ok(data) = run_json(&path, &["version"]) else { continue };
        if let Some(version) = data.get("version").and_then(Value::as_str) {
            return Some(CliInfo {
                path: path_string(&path),
                version: version.to_string(),
                channel: str_field(&data, "channel"),
            });
        }
    }
    None
}

// ---- Running commands -----------------------------------------------------------

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<EnvelopeError>,
}

#[derive(Deserialize)]
struct EnvelopeError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

fn base_command(cli: &Path) -> Command {
    let mut cmd = Command::new(cli);
    cmd.env("UNITY_NO_BANNER", "1")
        .env("UNITY_NO_PAGER", "1")
        .env("UNITY_COLOR", "never")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null());
    cmd
}

/// Runs `unity <args> --json` non-interactively and returns the `data` field.
pub fn run_json(cli: &Path, args: &[&str]) -> Result<Value> {
    let output = base_command(cli)
        .args(args)
        .arg("--json")
        .env("UNITY_NON_INTERACTIVE", "1")
        .output()?;
    parse_output(&output.stdout, &output.stderr, output.status.code())
}

pub(crate) fn parse_output(stdout: &[u8], stderr: &[u8], exit: Option<i32>) -> Result<Value> {
    // The CLI prints one JSON document; anything after it (progress noise) is ignored.
    let envelope = serde_json::Deserializer::from_slice(stdout).into_iter::<Envelope>().next();
    match envelope {
        Some(Ok(env)) if env.success => Ok(env.data.unwrap_or(Value::Null)),
        Some(Ok(env)) => {
            let first = env.errors.into_iter().next();
            Err(UniHubError::CliFailed {
                code: first.as_ref().map_or_else(|| "UNKNOWN".into(), |e| e.code.clone()),
                message: first.map_or_else(|| "the command failed".into(), |e| e.message),
            })
        }
        _ => {
            let text = String::from_utf8_lossy(if stderr.is_empty() { stdout } else { stderr });
            let message = text.trim().lines().last().unwrap_or("no output").to_string();
            Err(UniHubError::CliFailed {
                code: exit.map_or_else(|| "signal".into(), |c| format!("exit-{c}")),
                message,
            })
        }
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ---- Typed commands -------------------------------------------------------------

pub fn auth_status(cli: &Path) -> Result<AuthStatus> {
    match run_json(cli, &["auth", "status"]) {
        Ok(data) => Ok(parse_auth_status(&data)),
        // Signed out is a normal state, not an error.
        Err(UniHubError::CliFailed { code, .. }) if code == "AUTH_ERROR" || code == "exit-3" => {
            Ok(AuthStatus { signed_in: false, session_state: None, mode: None, name: None, email: None })
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn parse_auth_status(data: &Value) -> AuthStatus {
    let user = data.get("user").cloned().unwrap_or(Value::Null);
    AuthStatus {
        signed_in: bool_field(data, "loggedIn"),
        session_state: str_field(data, "sessionState"),
        mode: str_field(data, "mode"),
        name: str_field(&user, "name"),
        email: str_field(&user, "email"),
    }
}

pub fn accounts(cli: &Path) -> Result<Vec<Account>> {
    let data = run_json(cli, &["auth", "accounts"])?;
    Ok(data
        .get("accounts")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Account {
                        id: str_field(a, "foreignKey")?,
                        name: str_field(a, "name"),
                        email: str_field(a, "email"),
                        primary_org: str_field(a, "primaryOrg"),
                        is_active: bool_field(a, "isActive"),
                        signed_in: bool_field(a, "signedIn"),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn license_status(cli: &Path) -> Result<LicenseStatus> {
    let data = run_json(cli, &["license", "status"])?;
    let licenses = data
        .get("licenses")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|l| {
                    Some(License {
                        product: str_field(l, "product")?,
                        kind: str_field(l, "type"),
                        organization: str_field(l, "organization"),
                        expires: str_field(l, "expires"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(LicenseStatus {
        active: bool_field(&data, "active"),
        signed_in: bool_field(&data, "signedIn"),
        licenses,
    })
}

pub fn plugins(cli: &Path, refresh: bool) -> Result<Vec<Plugin>> {
    let data = if refresh { run_json(cli, &["plugin", "--refresh"]) } else { run_json(cli, &["plugin"]) }?;
    Ok(parse_plugins(&data))
}

pub(crate) fn parse_plugins(data: &Value) -> Vec<Plugin> {
    let Some(list) = data.get("components").and_then(Value::as_array) else { return vec![] };
    list.iter()
        .filter_map(|p| {
            let installed = bool_field(p, "installed");
            Some(Plugin {
                id: str_field(p, "id")?,
                display_name: str_field(p, "displayName").unwrap_or_default(),
                description: str_field(p, "description").unwrap_or_default(),
                kind: str_field(p, "kind").unwrap_or_else(|| "tool".into()),
                state: str_field(p, "state").unwrap_or_default(),
                installed,
                path: str_field(p, "path"),
                installed_version: if installed {
                    str_field(p, "version").or_else(|| str_field(p, "resolvedVersion"))
                } else {
                    None
                },
                available_version: str_field(p, "pinnedVersion").or_else(|| str_field(p, "resolvedVersion")),
                newer_version: str_field(p, "newestVersion"),
                blocked_by_cli_version: str_field(p, "blockedByCliVersion"),
            })
        })
        .collect()
}

pub fn update_check(cli: &Path) -> Result<CliUpdate> {
    let data = run_json(cli, &["self-update", "--check"])?;
    let current = str_field(&data, "currentVersion").unwrap_or_default();
    let latest = str_field(&data, "latestVersion");
    Ok(CliUpdate {
        update_available: latest.as_ref().is_some_and(|l| *l != current),
        current_version: current,
        latest_version: latest,
    })
}

// ---- Interactive sign-in ----------------------------------------------------------

/// Tracks the running `unity auth login` so it can be cancelled.
#[derive(Default)]
pub(crate) struct SignIn {
    child: Mutex<Option<Child>>,
}

impl SignIn {
    /// Runs `unity auth login`, which opens the browser, and waits until the user
    /// finishes or `cancel` is called.
    pub fn run(&self, cli: &Path) -> Result<()> {
        self.cancel(); // only one sign-in at a time
        let mut child = base_command(cli)
            .args(["auth", "login", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Drain the pipes on their own threads so the CLI never blocks on a full pipe.
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let pid = child.id();
        *self.lock() = Some(child);

        // Poll without holding the lock between checks, so `cancel` can kill the process.
        let status = loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let mut guard = self.lock();
            let Some(child) = guard.as_mut().filter(|c| c.id() == pid) else {
                return Err(UniHubError::Cancelled);
            };
            if let Some(status) = child.try_wait()? {
                guard.take();
                break status;
            }
        };
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        parse_output(&stdout, &stderr, status.code()).map(|_| ())
    }

    pub fn cancel(&self) {
        if let Some(mut child) = self.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Child>> {
        self.child.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn drain<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_success_and_failure_envelopes() {
        let ok = br#"{"success":true,"command":"version","data":{"version":"1.0.0"},"errors":[]}
trailing noise"#;
        assert_eq!(parse_output(ok, b"", Some(0)).unwrap()["version"], "1.0.0");

        let err = br#"{"success":false,"data":null,"errors":[{"code":"AUTH_ERROR","message":"Not signed in"}]}"#;
        match parse_output(err, b"", Some(3)) {
            Err(UniHubError::CliFailed { code, message }) => {
                assert_eq!(code, "AUTH_ERROR");
                assert_eq!(message, "Not signed in");
            }
            other => panic!("unexpected {other:?}"),
        }

        match parse_output(b"", b"boom\nreal reason", Some(6)) {
            Err(UniHubError::CliFailed { code, message }) => {
                assert_eq!(code, "exit-6");
                assert_eq!(message, "real reason");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn maps_auth_and_plugins() {
        let status = parse_auth_status(&json!({
            "loggedIn": true, "sessionState": "fresh", "mode": "oauth",
            "user": { "name": "Ada", "email": "ada@example.com" }
        }));
        assert!(status.signed_in);
        assert_eq!(status.email.as_deref(), Some("ada@example.com"));

        let plugins = parse_plugins(&json!({ "components": [
            { "id": "ugs", "displayName": "UGS CLI", "kind": "tool", "state": "not-installed",
              "installed": false, "pinnedVersion": "2.0.0", "resolvedVersion": "2.0.0" },
            { "id": "plastic", "displayName": "Plastic SCM", "kind": "tool", "state": "resolved",
              "installed": true, "path": "/x/cm", "version": null,
              "pinnedVersion": "11.0.16.10512", "resolvedVersion": "11.0.16+10371" }
        ]}));
        assert_eq!(plugins[0].installed_version, None);
        assert_eq!(plugins[0].available_version.as_deref(), Some("2.0.0"));
        assert_eq!(plugins[1].installed_version.as_deref(), Some("11.0.16+10371"));
    }
}
