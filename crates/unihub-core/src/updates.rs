//! UniHub's own updates, from the GitHub releases of the UniHub repository.
//!
//! A release is tagged `v<version>` and carries two assets made by `scripts/make-dmg.sh`:
//! `UniHub-<version>.dmg` and `UniHub-<version>.dmg.sha256`. Updating downloads the DMG,
//! checks its SHA-256 against the checksum file, mounts it read-only, checks the app inside
//! (bundle id, version, valid code signature), puts it in place of the running app (the old
//! one is restored if that fails) and leaves relaunching to the caller.
//!
//! Files downloaded this way carry no quarantine flag, so the updated app opens without a
//! Gatekeeper prompt even though it is only ad-hoc signed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::error::{Result, UniHubError};
use crate::releases::nullable;

/// Latest published (non-draft, non-prerelease) release of UniHub.
pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/flowerfx/UniHub/releases/latest";
pub const BUNDLE_ID: &str = "io.github.unihub.app";

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppUpdate {
    /// e.g. `0.2.0`
    pub version: String,
    /// The running version, e.g. `0.1.0`.
    pub current_version: String,
    /// Release title.
    pub name: String,
    /// Release notes (Markdown).
    pub notes: String,
    /// ISO-8601.
    pub published_at: Option<String>,
    /// The release's page on GitHub.
    pub page_url: String,
    pub dmg_url: String,
    pub dmg_size: u64,
    /// URL of `<dmg>.sha256`; without it the update cannot be installed.
    pub checksum_url: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    #[serde(default, deserialize_with = "nullable")]
    tag_name: String,
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    body: String,
    #[serde(default, deserialize_with = "nullable")]
    html_url: String,
    published_at: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    draft: bool,
    #[serde(default, deserialize_with = "nullable")]
    prerelease: bool,
    #[serde(default, deserialize_with = "nullable")]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    browser_download_url: String,
    #[serde(default, deserialize_with = "nullable")]
    size: u64,
}

/// The latest release if it is newer than `current` and has a DMG; `None` otherwise
/// (also when the repository has no release yet).
pub fn check(http: &reqwest::blocking::Client, current: &str) -> Result<Option<AppUpdate>> {
    // Debug builds can point at a test server: UNIHUB_UPDATE_URL=http://localhost:8765/latest.json
    let url = match std::env::var("UNIHUB_UPDATE_URL") {
        Ok(url) if cfg!(debug_assertions) => url,
        _ => LATEST_RELEASE_API.to_string(),
    };
    let response = http.get(url).header("Accept", "application/vnd.github+json").send()?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    let release: ApiRelease = response.error_for_status()?.json()?;
    Ok(from_release(release, current))
}

fn from_release(release: ApiRelease, current: &str) -> Option<AppUpdate> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, current) {
        return None;
    }
    let dmg = release.assets.iter().find(|a| a.name.to_ascii_lowercase().ends_with(".dmg"))?;
    let checksum = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&format!("{}.sha256", dmg.name)))
        .map(|a| a.browser_download_url.clone());
    Some(AppUpdate {
        name: if release.name.is_empty() { format!("UniHub {version}") } else { release.name.clone() },
        version,
        current_version: current.to_string(),
        notes: release.body,
        published_at: release.published_at,
        page_url: release.html_url,
        dmg_url: dmg.browser_download_url.clone(),
        dmg_size: dmg.size,
        checksum_url: checksum,
    })
}

/// Numeric comparison of dotted versions (`0.10.0` > `0.9.2`); a `-suffix` is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches(['v', 'V'])
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    let len = a.len().max(b.len());
    let pad = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..len).map(|i| pad(&a, i).cmp(&pad(&b, i))).find(|o| o.is_ne()) == Some(std::cmp::Ordering::Greater)
}

/// Downloads, verifies and installs `update` in place of the app at `app_path`
/// (`/Applications/UniHub.app`). `work_dir` holds the download. Relaunching is up to
/// the caller.
pub fn install(http: &reqwest::blocking::Client, update: &AppUpdate, app_path: &Path, work_dir: &Path) -> Result<()> {
    check_target(app_path)?;
    let checksum_url = update.checksum_url.as_deref().ok_or_else(|| {
        invalid("this release has no .sha256 checksum file, so it cannot be verified; download it from GitHub instead")
    })?;
    let expected = parse_checksum(&http.get(checksum_url).send()?.error_for_status()?.text()?)
        .ok_or_else(|| invalid("the release's checksum file is not a SHA-256 checksum"))?;

    fs::create_dir_all(work_dir)?;
    let dmg = work_dir.join(format!("UniHub-{}.dmg", update.version));
    let mut response = http.get(&update.dmg_url).send()?.error_for_status()?;
    let mut file = fs::File::create(&dmg)?;
    io::copy(&mut response, &mut file)?;
    drop(file);

    let result = (|| {
        if sha256(&dmg)? != expected {
            return Err(invalid("the downloaded update does not match its checksum"));
        }
        install_dmg(&dmg, app_path, &update.version, work_dir)
    })();
    let _ = fs::remove_file(&dmg);
    result
}

/// Mounts `dmg`, checks the UniHub app inside and swaps it in at `app_path`.
pub(crate) fn install_dmg(dmg: &Path, app_path: &Path, version: &str, work_dir: &Path) -> Result<()> {
    check_target(app_path)?;
    let mount = Mount::attach(dmg, &work_dir.join(format!("mount-{}", std::process::id())))?;
    let new_app = find_app(&mount.path).ok_or_else(|| invalid("the update has no UniHub app inside"))?;
    let found = bundle_version(&new_app).unwrap_or_default();
    if found != version {
        return Err(invalid(&format!("the update contains UniHub {found}, expected {version}")));
    }
    run("/usr/bin/codesign", &["--verify", "--deep", "--strict", &new_app.to_string_lossy()])
        .map_err(|e| invalid(&format!("the updated app's code signature is not valid: {e}")))?;

    // Copy next to the current app (same volume), then swap with two renames.
    let parent = app_path.parent().ok_or_else(|| invalid("UniHub's location has no parent folder"))?;
    let stem = format!(".UniHub-{}", std::process::id());
    let staged = parent.join(format!("{stem}-new.app"));
    let backup = parent.join(format!("{stem}-old.app"));
    let _ = fs::remove_dir_all(&staged);
    run("/usr/bin/ditto", &[&new_app.to_string_lossy(), &staged.to_string_lossy()])?;
    drop(mount);

    if let Err(e) = fs::rename(app_path, &backup) {
        let _ = fs::remove_dir_all(&staged);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(&staged, app_path) {
        let _ = fs::rename(&backup, app_path); // put the old app back
        let _ = fs::remove_dir_all(&staged);
        return Err(e.into());
    }
    // The running process keeps its open files; the old bundle can go.
    let _ = fs::remove_dir_all(&backup);
    Ok(())
}

/// The running app must be a `.app` bundle in a folder we can write.
fn check_target(app_path: &Path) -> Result<()> {
    if app_path.extension().is_none_or(|e| e != "app") || !app_path.is_dir() {
        return Err(invalid("UniHub is not running from an app bundle, so it cannot update itself"));
    }
    let parent = app_path.parent().ok_or_else(|| invalid("UniHub's location has no parent folder"))?;
    let probe = parent.join(format!(".unihub-write-test-{}", std::process::id()));
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(invalid(&format!(
            "UniHub cannot write to {}; install the update from the DMG by hand",
            parent.display()
        ))),
    }
}

fn find_app(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().is_some_and(|e| e == "app")
            && !p.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(true)
            && bundle_id(p).as_deref() == Some(BUNDLE_ID)
    })
}

fn info_plist(app: &Path) -> Option<plist::Dictionary> {
    plist::from_file(app.join("Contents/Info.plist")).ok()
}

fn bundle_id(app: &Path) -> Option<String> {
    info_plist(app)?.get("CFBundleIdentifier")?.as_string().map(str::to_string)
}

fn bundle_version(app: &Path) -> Option<String> {
    info_plist(app)?.get("CFBundleShortVersionString")?.as_string().map(str::to_string)
}

/// First 64-hex-digit word of a `shasum -a 256` line.
fn parse_checksum(text: &str) -> Option<String> {
    text.split_whitespace()
        .next()
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

fn sha256(path: &Path) -> Result<String> {
    let out = run("/usr/bin/shasum", &["-a", "256", &path.to_string_lossy()])?;
    parse_checksum(&out).ok_or_else(|| invalid("could not compute the update's checksum"))
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(invalid(err.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("command failed")))
    }
}

fn invalid(message: &str) -> UniHubError {
    UniHubError::InvalidData { message: message.to_string() }
}

/// A read-only DMG mount, detached when dropped.
struct Mount {
    path: PathBuf,
}

impl Mount {
    fn attach(dmg: &Path, mount_point: &Path) -> Result<Self> {
        fs::create_dir_all(mount_point)?;
        run(
            "/usr/bin/hdiutil",
            &[
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
                &mount_point.to_string_lossy(),
                &dmg.to_string_lossy(),
            ],
        )?;
        Ok(Self { path: mount_point.to_path_buf() })
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        let _ = run("/usr/bin/hdiutil", &["detach", "-quiet", &self.path.to_string_lossy()]);
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> ApiRelease {
        serde_json::from_value(serde_json::json!({
            "tag_name": tag, "name": null, "body": "Notes", "html_url": "https://github.com/x/releases/tag/v",
            "published_at": "2026-11-08T00:00:00Z", "draft": false, "prerelease": false,
            "assets": assets.iter().map(|n| serde_json::json!({
                "name": n, "browser_download_url": format!("https://example.com/{n}"), "size": 100
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn reads_the_latest_release() {
        let update = from_release(release("v0.2.0", &["UniHub-0.2.0.dmg", "UniHub-0.2.0.dmg.sha256"]), "0.1.0").unwrap();
        assert_eq!(update.version, "0.2.0");
        assert_eq!(update.name, "UniHub 0.2.0");
        assert_eq!(update.dmg_url, "https://example.com/UniHub-0.2.0.dmg");
        assert_eq!(update.checksum_url.as_deref(), Some("https://example.com/UniHub-0.2.0.dmg.sha256"));

        assert!(from_release(release("v0.1.0", &["UniHub-0.1.0.dmg"]), "0.1.0").is_none());
        assert!(from_release(release("v0.2.0", &["notes.txt"]), "0.1.0").is_none());
        let no_sum = from_release(release("v0.2.0", &["UniHub-0.2.0.dmg"]), "0.1.0").unwrap();
        assert!(no_sum.checksum_url.is_none());
    }

    #[test]
    fn parses_checksums() {
        let hex = "27ff86b04eb6020c10b637fb86ca68db4180325e370e1fa0466b078c2bee4625";
        assert_eq!(parse_checksum(&format!("{hex}  UniHub-0.1.0.dmg\n")).as_deref(), Some(hex));
        assert_eq!(parse_checksum(&hex.to_uppercase()).as_deref(), Some(hex));
        assert!(parse_checksum("not a checksum").is_none());
    }

    #[test]
    fn refuses_targets_that_are_not_app_bundles() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_target(&dir.path().join("unihub")).is_err());
        fs::create_dir_all(dir.path().join("UniHub.app")).unwrap();
        assert!(check_target(&dir.path().join("UniHub.app")).is_ok());
    }

    /// Builds a tiny signed UniHub.app into a DMG and installs it over another one.
    #[test]
    fn installs_an_update_from_a_dmg() {
        let dir = tempfile::tempdir().unwrap();
        let make_app = |root: &Path, version: &str| -> PathBuf {
            let app = root.join("UniHub.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::copy("/usr/bin/true", app.join("Contents/MacOS/UniHub")).unwrap();
            let mut info = plist::Dictionary::new();
            info.insert("CFBundleIdentifier".into(), BUNDLE_ID.into());
            info.insert("CFBundleExecutable".into(), "UniHub".into());
            info.insert("CFBundlePackageType".into(), "APPL".into());
            info.insert("CFBundleShortVersionString".into(), version.into());
            plist::to_file_xml(app.join("Contents/Info.plist"), &info).unwrap();
            run("/usr/bin/codesign", &["--force", "--sign", "-", &app.to_string_lossy()]).unwrap();
            app
        };
        let installed = make_app(&dir.path().join("Applications"), "0.1.0");
        let source = dir.path().join("dmg-src");
        make_app(&source, "0.2.0");
        let dmg = dir.path().join("UniHub-0.2.0.dmg");
        run(
            "/usr/bin/hdiutil",
            &["create", "-quiet", "-srcfolder", &source.to_string_lossy(), "-format", "UDZO", "-ov", &dmg.to_string_lossy()],
        )
        .unwrap();

        // A wrong version is refused and leaves the installed app alone.
        assert!(install_dmg(&dmg, &installed, "0.3.0", &dir.path().join("work")).is_err());
        assert_eq!(bundle_version(&installed).as_deref(), Some("0.1.0"));

        install_dmg(&dmg, &installed, "0.2.0", &dir.path().join("work")).unwrap();
        assert_eq!(bundle_version(&installed).as_deref(), Some("0.2.0"));
        let leftovers: Vec<_> = fs::read_dir(installed.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers, ["UniHub.app"]);
    }
}
