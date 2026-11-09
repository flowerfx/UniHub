//! Creating a project from a template.
//!
//! Like the official Hub, UniHub starts the editor with
//! `-createProject <project folder> -cloneFromTemplate <template .tgz>`: the editor
//! creates the folder, unpacks the template into it, resolves its packages and opens it. Templates
//! that are not on disk yet are first downloaded from Unity's package registry into
//! UniHub's own template folder and checked against their published checksum.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::editors::Editor;
use crate::error::{Result, UniHubError};
use crate::jobs::Task;
use crate::paths::path_string;
use crate::projects::is_unity_project;

/// The name the dialog suggests, numbered when the folder is taken ("My project (1)").
pub const DEFAULT_NAME: &str = "My project";

/// `base`, or `base (n)` with `n` one above the highest number already used in `parent`,
/// comparing names without regard to case, as the Hub does ("My project (3)" → "(4)").
pub fn unique_name(parent: &Path, base: &str) -> String {
    let base = base.trim();
    let base = if base.is_empty() { DEFAULT_NAME } else { base };
    let lower = base.to_lowercase();
    let mut taken = false;
    let mut highest = 0u32;
    for entry in fs::read_dir(parent).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == lower {
            taken = true;
        } else if let Some(n) = name
            .strip_prefix(&lower)
            .and_then(|rest| rest.strip_prefix(" ("))
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|n| n.parse::<u32>().ok())
        {
            highest = highest.max(n);
        }
    }
    if !taken && highest == 0 {
        base.to_string()
    } else {
        format!("{base} ({})", highest + 1)
    }
}

/// Longest project name the Hub accepts (255 minus room for Unity's own files).
const MAX_NAME_LEN: usize = 223;

/// Names Windows reserves; the Hub refuses them so projects can be shared across systems.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1",
    "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Checks a new project's name and location; returns the folder it will be created in.
/// Follows the Hub's rules: a name that works on macOS, Windows and Linux, and a folder
/// that does not exist yet (the editor creates it).
pub fn check(parent: &str, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    let invalid = |message: &str| Err(UniHubError::InvalidData { message: message.to_string() });
    if name.is_empty() {
        return invalid("Enter a project name.");
    }
    if name.starts_with('.') {
        return invalid("The project name cannot start with a dot.");
    }
    if name.ends_with('.') {
        return invalid("The project name cannot end with a dot.");
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"/\|?*`%"#.contains(*c) || c.is_control()) {
        let shown = if c.is_control() { "control characters".to_string() } else { format!("“{c}”") };
        return invalid(&format!("The project name cannot contain {shown}."));
    }
    if RESERVED_NAMES.contains(&name.to_lowercase().as_str()) {
        return invalid(&format!("“{name}” is a reserved name on Windows; choose another."));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return invalid(&format!("The project name is too long (at most {MAX_NAME_LEN} characters)."));
    }
    let parent = parent.trim();
    if parent.is_empty() {
        return invalid("Choose where to create the project.");
    }
    let parent = Path::new(parent);
    if !parent.is_dir() {
        return invalid(&format!("The folder {} does not exist.", parent.display()));
    }
    // Canonical, like the paths in the project list (`/tmp` → `/private/tmp`).
    let project = fs::canonicalize(parent)?.join(name);
    if project.symlink_metadata().is_ok() {
        return invalid(&format!("{} already exists. Choose another name.", project.display()));
    }
    // Reading also fails when macOS privacy settings keep UniHub out of the folder.
    if fs::read_dir(parent).is_err() {
        return invalid(&format!("UniHub cannot open {}.", parent.display()));
    }
    Ok(project)
}

/// `check`, plus a test that a file can be created next to the project. Writes to the
/// folder, so it runs once when creating, not while the name is typed.
pub fn check_before_creating(parent: &str, name: &str) -> Result<PathBuf> {
    let project = check(parent, name)?;
    let dir = project.parent().unwrap_or(&project);
    if !is_writable(dir) {
        return Err(UniHubError::InvalidData { message: format!("UniHub cannot write to {}.", dir.display()) });
    }
    Ok(project)
}

/// Whether a file can be created in `dir` (also catches macOS privacy refusals).
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".unihub-write-test-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ---- Downloading templates ----------------------------------------------------------

/// A published checksum, `<algorithm>-<base64>`. Unity's registry puts the base64 of the
/// hex digest there (`sha1-YWE0NWI0…`); npm puts the base64 of the raw digest. Returns the
/// `shasum` algorithm number and the hex digest.
pub(crate) fn parse_integrity(integrity: &str) -> Option<(&'static str, String)> {
    let (algorithm, encoded) = integrity.trim().split_once('-')?;
    let (shasum_algorithm, digest_len) = match algorithm.to_ascii_lowercase().as_str() {
        "sha1" => ("1", 20),
        "sha256" => ("256", 32),
        "sha512" => ("512", 64),
        _ => return None,
    };
    let bytes = base64_decode(encoded)?;
    let hex = if bytes.len() == digest_len * 2 && bytes.iter().all(u8::is_ascii_hexdigit) {
        String::from_utf8(bytes).ok()?.to_ascii_lowercase()
    } else if bytes.len() == digest_len {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    } else {
        return None;
    };
    Some((shasum_algorithm, hex))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.trim_end_matches('=').bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// Downloads a template archive to `dest`, reporting progress and stopping when the job is
/// cancelled. With an `integrity`, a download that does not match it is deleted.
pub(crate) fn download(
    http: &reqwest::blocking::Client,
    url: &str,
    integrity: Option<&str>,
    dest: &Path,
    task: &Task,
) -> Result<()> {
    let expected = match integrity {
        Some(i) => Some(parse_integrity(i).ok_or_else(|| UniHubError::InvalidData {
            message: format!("the template's checksum has an unknown format: {i}"),
        })?),
        None => None,
    };
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = dest.with_extension("tgz.part");
    let result = (|| {
        let mut response = http
            .get(url)
            .timeout(Duration::from_secs(60 * 60))
            .send()?
            .error_for_status()?;
        let total = response.content_length();
        let mut file = fs::File::create(&partial)?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        let mut last_report = 0u64;
        loop {
            task.check()?;
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n])?;
            done += n as u64;
            if done - last_report >= 1 << 20 || Some(done) == total {
                last_report = done;
                let percent = total.filter(|t| *t > 0).map(|t| done as f64 * 100.0 / t as f64);
                task.progress(format!("Downloading the template… {}", progress_text(done, total)), Some("download"), percent);
            }
        }
        file.sync_all()?;
        drop(file);
        if let Some((algorithm, hex)) = &expected {
            task.progress("Checking the template…", Some("verify"), None);
            if shasum(&partial, algorithm)? != *hex {
                return Err(UniHubError::InvalidData {
                    message: "the downloaded template does not match its checksum".into(),
                });
            }
        }
        fs::rename(&partial, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn progress_text(done: u64, total: Option<u64>) -> String {
    let mb = |b: u64| b as f64 / 1_000_000.0;
    match total {
        Some(t) if t > 0 => format!("{:.0} of {:.0} MB", mb(done), mb(t)),
        _ => format!("{:.0} MB", mb(done)),
    }
}

fn shasum(path: &Path, algorithm: &str) -> Result<String> {
    let out = Command::new("/usr/bin/shasum").args(["-a", algorithm]).arg(path).stdin(Stdio::null()).output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .next()
        .filter(|h| out.status.success() && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| UniHubError::InvalidData { message: "could not compute the template's checksum".into() })
}

/// Records a downloaded template in `<dir>/manifest.json`, in the same shape as the Hub's
/// (`{"<editor version>": {"dependencies": {"<template id>": "<version>"}}}`), so the
/// template is listed as downloaded for that editor next time.
pub(crate) fn record_download(dir: &Path, editor_version: &str, id: &str, version: &str) -> Result<()> {
    let file = dir.join("manifest.json");
    let mut manifest: serde_json::Value = fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    manifest[editor_version]["dependencies"][id] = serde_json::Value::from(version);
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&manifest)?)?;
    fs::rename(&tmp, &file)?;
    Ok(())
}

// ---- Starting the editor -------------------------------------------------------------

/// Starts `editor` to create `project` from `archive`, then waits until the project's
/// folder holds a Unity project (the editor goes on importing and opens it). Fails if the
/// editor quits first. Cancelling stops the editor.
pub(crate) fn launch(editor: &Editor, project: &Path, archive: &Path, task: &Task) -> Result<String> {
    task.progress(format!("Starting Unity {}…", editor.version), Some("launch"), None);
    let mut cmd = Command::new(&editor.executable_path);
    // The Hub passes empty cloud ids when the project is not connected to Unity Cloud.
    cmd.arg("-createProject")
        .arg(project)
        .arg("-cloneFromTemplate")
        .arg(archive)
        .args(["-cloudOrganization", "", "-cloudProject", "", "-cloudEnvironment", "production"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Own process group, so cancelling stops the editor and the helpers it starts.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn()?;
    task.set_process_group(Some(child.id()));

    task.progress("Unity is creating the project…", Some("create"), None);
    let outcome = loop {
        if task.is_cancelled() {
            break Err(UniHubError::Cancelled);
        }
        if is_created(project) {
            break Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // In batch mode (or on an error) the editor may quit right after creating it.
                if is_created(project) {
                    break Ok(());
                }
                let code = status.code().map_or_else(|| "a signal".to_string(), |c| format!("code {c}"));
                break Err(UniHubError::InvalidData {
                    message: format!(
                        "Unity quit (exit {code}) before creating the project. Its log is ~/Library/Logs/Unity/Editor.log."
                    ),
                });
            }
            Ok(None) => {}
            Err(e) => break Err(e.into()),
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    task.set_process_group(None);
    if matches!(outcome, Err(UniHubError::Cancelled)) {
        // Also when the job was cancelled before the editor's process group was known.
        let _ = Command::new("/bin/kill").args(["-TERM", "--", &format!("-{}", child.id())]).status();
    }
    // The editor keeps running with the new project; reap it when it quits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    outcome.map(|()| format!("Created {}. Unity is opening it.", project.file_name().map(path_string_os).unwrap_or_default()))
}

fn path_string_os(name: &std::ffi::OsStr) -> String {
    path_string(Path::new(name))
}

/// The editor has written the project: Assets, ProjectSettings and its version file.
fn is_created(project: &Path) -> bool {
    is_unity_project(project) && project.join("ProjectSettings/ProjectVersion.txt").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_a_free_name() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(unique_name(tmp.path(), DEFAULT_NAME), "My project");
        fs::create_dir(tmp.path().join("My project")).unwrap();
        assert_eq!(unique_name(tmp.path(), DEFAULT_NAME), "My project (1)");
        // One above the highest number, case-insensitively, like the Hub.
        fs::create_dir(tmp.path().join("my PROJECT (3)")).unwrap();
        fs::write(tmp.path().join("My project (x)"), "").unwrap();
        assert_eq!(unique_name(tmp.path(), "My project"), "My project (4)");
        assert_eq!(unique_name(tmp.path(), "  "), "My project (4)");
        assert_eq!(unique_name(tmp.path(), "Game"), "Game");
        assert_eq!(unique_name(&tmp.path().join("missing"), "Game"), "Game");
    }

    #[test]
    fn checks_name_and_location() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = path_string(tmp.path());
        let canonical = fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(check(&parent, " Game ").unwrap(), canonical.join("Game"));
        assert!(check(&parent, "My Game (2) – ünïcode").is_ok());
        let long = "x".repeat(MAX_NAME_LEN + 1);
        for bad in ["", "  ", ".hidden", "trailing.", "a/b", "a:b", "a\\b", "a*b", "a?b", "a|b", "a%b", "a`b", "a\nb", "CON", "lpt1", &long] {
            assert!(check(&parent, bad).is_err(), "{bad:?} should be refused");
        }
        assert!(check(&parent, &"x".repeat(MAX_NAME_LEN)).is_ok());
        assert!(check("", "Game").is_err());
        assert!(check(&path_string(&tmp.path().join("missing")), "Game").is_err());

        // The folder must not exist yet, not even empty: the editor creates it.
        fs::create_dir(tmp.path().join("Empty")).unwrap();
        assert!(check(&parent, "Empty").is_err());
        fs::write(tmp.path().join("AFile"), "x").unwrap();
        assert!(check(&parent, "AFile").is_err());
        // Checking writes nothing; the check before creating leaves nothing behind.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
        assert!(check_before_creating(&parent, "Game").is_ok());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    #[test]
    fn parses_integrity_strings() {
        // Unity's registry: base64 of the hex digest.
        let unity = "sha1-YWE0NWI0NzViMzg1N2Q0MTEwMDkwNGY4MzdmMzUyYzBhMzc1NGJjNQ==";
        assert_eq!(parse_integrity(unity), Some(("1", "aa45b475b3857d41100904f837f352c0a3754bc5".into())));
        // npm: base64 of the raw digest (sha1 of "hello").
        assert_eq!(
            parse_integrity("sha1-qvTGHdzF6KLavt4PO0gs2a6pQ00="),
            Some(("1", "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".into()))
        );
        assert_eq!(parse_integrity("md5-abc"), None);
        assert_eq!(parse_integrity("sha1-!!"), None);
        assert_eq!(parse_integrity("sha1-YWJj"), None); // wrong length
    }

    #[test]
    fn records_downloads_like_the_hub() {
        let tmp = tempfile::tempdir().unwrap();
        record_download(tmp.path(), "6000.0.85f1", "com.unity.template.fps", "5.1.1").unwrap();
        record_download(tmp.path(), "6000.0.85f1", "com.unity.template.vr", "9.2.3").unwrap();
        record_download(tmp.path(), "6000.6.4f1", "com.unity.template.fps", "5.2.0").unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tmp.path().join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["6000.0.85f1"]["dependencies"]["com.unity.template.vr"], "9.2.3");
        assert_eq!(manifest["6000.0.85f1"]["dependencies"]["com.unity.template.fps"], "5.1.1");
        assert_eq!(manifest["6000.6.4f1"]["dependencies"]["com.unity.template.fps"], "5.2.0");
    }

    #[test]
    fn computes_checksums_with_shasum() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("hello");
        fs::write(&file, "hello").unwrap();
        assert_eq!(shasum(&file, "1").unwrap(), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }
}
