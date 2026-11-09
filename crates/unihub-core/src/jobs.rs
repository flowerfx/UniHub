//! Long-running work in the background: Unity CLI commands (editor and module installs)
//! and creating projects. The CLI is started with `--format ndjson`, which prints one JSON object per
//! line: `{"type":"progress","pct":50,"msg":"Installing iOS Build Support...","phase":"install"}`
//! while it works and `{"type":"result","success":…,"errors":[…]}` at the end. The app
//! polls `UniHub::jobs()` to show progress.
//!
//! Several installs may be started; the CLI itself waits while another install (its own
//! or the Hub's) is running, so they run one after another.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::error::{Result, UniHubError};
use crate::unity_cli;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum JobKind {
    InstallEditor,
    InstallModules,
    /// Downloading the template if needed, then starting the editor on the new project.
    CreateProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum JobState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Job {
    pub id: u64,
    pub kind: JobKind,
    /// Editor version, e.g. `6000.0.85f1`.
    pub version: String,
    pub architecture: String,
    /// Module ids requested (for an editor install: the modules installed with it).
    pub module_ids: Vec<String>,
    /// The project being created (`CreateProject`).
    pub project_path: Option<String>,
    pub state: JobState,
    /// Latest progress message, e.g. "Downloading iOS Build Support...".
    pub message: String,
    /// `download`, `install`, … as reported by the CLI.
    pub phase: Option<String>,
    /// 0–100 for the current step, when the CLI reports it.
    pub percent: Option<f64>,
    pub error: Option<String>,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
}

#[derive(Default)]
pub(crate) struct JobRunner {
    jobs: Arc<Mutex<Vec<Job>>>,
    /// Process (group) id of each running job's CLI.
    pids: Arc<Mutex<HashMap<u64, u32>>>,
    /// Cancel flags of running tasks (`spawn_task`).
    flags: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
    next_id: AtomicU64,
}

impl JobRunner {
    /// Starts `unity <args> --format ndjson` in the background and returns the job id.
    pub fn start(
        &self,
        cli: &Path,
        args: Vec<String>,
        kind: JobKind,
        version: &str,
        architecture: &str,
        module_ids: Vec<String>,
    ) -> Result<u64> {
        let mut cmd = unity_cli::base_command(cli);
        cmd.args(&args)
            .args(["--format", "ndjson"])
            .env("UNITY_NON_INTERACTIVE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Own process group, so cancelling also stops the installers the CLI starts.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;

        let id = self.add(kind, version, architecture, module_ids, None);
        lock(&self.pids).insert(id, child.id());

        let stdout = child.stdout.take();
        let stderr = unity_cli::drain(child.stderr.take());
        let jobs = Arc::clone(&self.jobs);
        let pids = Arc::clone(&self.pids);
        std::thread::spawn(move || {
            let mut outcome = Outcome::default();
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(std::result::Result::ok) {
                    if let Some(progress) = outcome.read_line(&line) {
                        update(&jobs, id, |job| {
                            job.message = progress.message;
                            job.phase = progress.phase;
                            job.percent = progress.percent;
                        });
                    }
                }
            }
            let status = child.wait();
            let stderr = stderr.join().unwrap_or_default();
            lock(&pids).remove(&id);
            let exit = status.ok().and_then(|s| s.code());
            let (state, error) = outcome.finish(exit, &String::from_utf8_lossy(&stderr));
            update(&jobs, id, |job| {
                if job.state == JobState::Running {
                    job.state = state;
                    job.error = error;
                    if state == JobState::Succeeded {
                        job.message = "Installed".into();
                        job.percent = Some(100.0);
                    }
                }
                job.finished_ms = Some(crate::now_ms());
            });
        });
        Ok(id)
    }

    /// Runs `work` on a background thread as a job and returns its id. `work` reports
    /// progress through the `Task`, checks it for cancellation, and returns the message
    /// shown when it succeeds.
    pub fn spawn_task<F>(
        &self,
        kind: JobKind,
        version: &str,
        architecture: &str,
        project_path: Option<String>,
        work: F,
    ) -> u64
    where
        F: FnOnce(&Task) -> Result<String> + Send + 'static,
    {
        let id = self.add(kind, version, architecture, Vec::new(), project_path);
        let cancelled = Arc::new(AtomicBool::new(false));
        lock(&self.flags).insert(id, Arc::clone(&cancelled));
        let task = Task { id, jobs: Arc::clone(&self.jobs), pids: Arc::clone(&self.pids), cancelled };
        let flags = Arc::clone(&self.flags);
        std::thread::spawn(move || {
            let result = work(&task);
            lock(&flags).remove(&id);
            lock(&task.pids).remove(&id);
            update(&task.jobs, id, |job| {
                if job.state == JobState::Running {
                    match result {
                        Ok(message) => {
                            job.state = JobState::Succeeded;
                            job.message = message;
                            job.percent = Some(100.0);
                        }
                        Err(UniHubError::Cancelled) => {
                            job.state = JobState::Cancelled;
                            job.message = "Cancelled".into();
                        }
                        Err(e) => {
                            job.state = JobState::Failed;
                            job.error = Some(e.to_string());
                        }
                    }
                }
                job.finished_ms = Some(crate::now_ms());
            });
        });
        id
    }

    fn add(
        &self,
        kind: JobKind,
        version: &str,
        architecture: &str,
        module_ids: Vec<String>,
        project_path: Option<String>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.jobs).push(Job {
            id,
            kind,
            version: version.to_string(),
            architecture: architecture.to_string(),
            module_ids,
            project_path,
            state: JobState::Running,
            message: "Starting…".into(),
            phase: None,
            percent: None,
            error: None,
            started_ms: crate::now_ms(),
            finished_ms: None,
        });
        id
    }

    pub fn list(&self) -> Vec<Job> {
        lock(&self.jobs).clone()
    }

    /// Stops a running job: its CLI (and the installers it started), or a task, which
    /// stops at its next check and has the process it started stopped.
    pub fn cancel(&self, id: u64) {
        let pid = lock(&self.pids).get(&id).copied();
        let flag = lock(&self.flags).get(&id).cloned();
        if pid.is_none() && flag.is_none() {
            return;
        }
        update(&self.jobs, id, |job| {
            if job.state == JobState::Running {
                job.state = JobState::Cancelled;
                job.message = "Cancelled".into();
            }
        });
        if let Some(flag) = flag {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(pid) = pid {
            // Negative pid: the whole process group.
            let _ = Command::new("/bin/kill").args(["-TERM", "--", &format!("-{pid}")]).status();
        }
    }

    /// Forgets a finished job.
    pub fn dismiss(&self, id: u64) {
        lock(&self.jobs).retain(|j| j.id != id || j.state == JobState::Running);
    }

    pub fn is_running(&self, version: &str, architecture: &str) -> bool {
        lock(&self.jobs)
            .iter()
            .any(|j| j.state == JobState::Running && j.version == version && j.architecture == architecture)
    }
}

/// A running `spawn_task` job, as its work sees it.
pub(crate) struct Task {
    id: u64,
    jobs: Arc<Mutex<Vec<Job>>>,
    pids: Arc<Mutex<HashMap<u64, u32>>>,
    cancelled: Arc<AtomicBool>,
}

impl Task {
    pub fn progress(&self, message: impl Into<String>, phase: Option<&str>, percent: Option<f64>) {
        let message = message.into();
        update(&self.jobs, self.id, |job| {
            job.message = message;
            job.phase = phase.map(str::to_string);
            job.percent = percent.map(|p| p.clamp(0.0, 100.0));
        });
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once the job was cancelled.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(UniHubError::Cancelled) } else { Ok(()) }
    }

    /// The process group cancelling should stop, or `None` once there is nothing to stop.
    pub fn set_process_group(&self, pid: Option<u32>) {
        match pid {
            Some(pid) => lock(&self.pids).insert(self.id, pid),
            None => lock(&self.pids).remove(&self.id),
        };
    }
}

struct Progress {
    message: String,
    phase: Option<String>,
    percent: Option<f64>,
}

/// What the CLI printed, read line by line.
#[derive(Default)]
struct Outcome {
    result: Option<Value>,
    /// Modules the CLI did not know ("Missing module \"x\". Skipping..").
    skipped: Vec<String>,
    last_text: Option<String>,
}

impl Outcome {
    /// Returns a progress update for progress lines; remembers the result and notes.
    fn read_line(&mut self, line: &str) -> Option<Progress> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if line.starts_with('{')
            && let Ok(value) = serde_json::from_str::<Value>(line)
        {
            match value.get("type").and_then(Value::as_str) {
                Some("progress") => {
                    let message = value.get("msg").and_then(Value::as_str).unwrap_or_default();
                    let message = match message.strip_suffix("...") {
                        Some(m) => format!("{m}…"),
                        None => message.to_string(),
                    };
                    return Some(Progress {
                        message,
                        phase: value.get("phase").and_then(Value::as_str).map(str::to_string),
                        percent: value.get("pct").and_then(Value::as_f64).map(|p| p.clamp(0.0, 100.0)),
                    });
                }
                Some("result") => self.result = Some(value),
                _ => {}
            }
            return None;
        }
        if let Some(rest) = line.strip_prefix("Missing module") {
            let name = rest.split('"').nth(1).unwrap_or(rest).trim().to_string();
            self.skipped.push(name);
        } else if !line.starts_with('<') {
            self.last_text = Some(line.to_string());
        }
        None
    }

    fn finish(&self, exit: Option<i32>, stderr: &str) -> (JobState, Option<String>) {
        match &self.result {
            Some(r) if r.get("success").and_then(Value::as_bool) == Some(true) => {
                if self.skipped.is_empty() {
                    (JobState::Succeeded, None)
                } else {
                    (JobState::Failed, Some(format!("Unknown modules were skipped: {}", self.skipped.join(", "))))
                }
            }
            Some(r) => {
                let first = r.get("errors").and_then(Value::as_array).and_then(|e| e.first());
                let message = first
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the install failed");
                (JobState::Failed, Some(message.to_string()))
            }
            None => {
                let text = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string);
                let message = text.or_else(|| self.last_text.clone()).unwrap_or_else(|| match exit {
                    Some(code) => format!("the Unity CLI stopped with exit code {code}"),
                    None => "the Unity CLI was stopped".into(),
                });
                (JobState::Failed, Some(message))
            }
        }
    }
}

fn update(jobs: &Mutex<Vec<Job>>, id: u64, change: impl FnOnce(&mut Job)) {
    if let Some(job) = lock(jobs).iter_mut().find(|j| j.id == id) {
        change(job);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &[&str]) -> Outcome {
        let mut o = Outcome::default();
        for l in lines {
            o.read_line(l);
        }
        o
    }

    #[test]
    fn reads_progress_and_success() {
        let mut o = Outcome::default();
        let p = o
            .read_line(r#"{"type":"progress","pct":50,"msg":"Installing 日本語...","phase":"install"}"#)
            .unwrap();
        assert_eq!(p.message, "Installing 日本語…");
        assert_eq!(p.percent, Some(50.0));
        assert_eq!(p.phase.as_deref(), Some("install"));
        o.read_line(r#"{"type":"result","success":true,"command":"install-modules","data":{},"errors":[]}"#);
        assert_eq!(o.finish(Some(0), ""), (JobState::Succeeded, None));
    }

    #[test]
    fn reports_cli_errors_and_skipped_modules() {
        let o = feed(&[r#"{"type":"result","success":false,"errors":[{"code":"EULA","message":"Accept the Android SDK terms first"}]}"#]);
        assert_eq!(o.finish(Some(1), ""), (JobState::Failed, Some("Accept the Android SDK terms first".into())));

        let o = feed(&[
            r#"Missing module "nope". Skipping.."#,
            r#"{"type":"result","success":true,"data":{"completedUids":[]},"errors":[]}"#,
        ]);
        assert_eq!(o.finish(Some(0), "").0, JobState::Failed);
        assert!(o.finish(Some(0), "").1.unwrap().contains("nope"));

        let o = feed(&["Adding module cmake as dependency of android-sdk-ndk-tools."]);
        assert_eq!(o.finish(Some(2), "boom\n").1.as_deref(), Some("boom"));
    }

    #[test]
    fn runs_a_command_in_the_background() {
        // A stand-in "CLI" that prints ndjson like the Unity CLI does.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("unity");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"type\":\"progress\",\"pct\":10,\"msg\":\"Downloading x...\",\"phase\":\"download\"}'\n\
             echo '{\"type\":\"result\",\"success\":true,\"errors\":[]}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let runner = JobRunner::default();
        let id = runner
            .start(&script, vec!["install-modules".into()], JobKind::InstallModules, "6000.0.1f1", "arm64", vec!["x".into()])
            .unwrap();
        for _ in 0..100 {
            if runner.list()[0].state != JobState::Running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let job = &runner.list()[0];
        assert_eq!(job.id, id);
        assert_eq!(job.state, JobState::Succeeded);
        assert_eq!(job.percent, Some(100.0));
        runner.dismiss(id);
        assert!(runner.list().is_empty());
    }

    #[test]
    fn runs_and_cancels_tasks() {
        let runner = JobRunner::default();
        let done = runner.spawn_task(JobKind::CreateProject, "6000.0.1f1", "arm64", Some("/p".into()), |task| {
            task.progress("Downloading…", Some("download"), Some(150.0));
            Ok("Created".into())
        });
        let failing = runner.spawn_task(JobKind::CreateProject, "6000.0.1f1", "arm64", None, |_| {
            Err(UniHubError::InvalidData { message: "boom".into() })
        });
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let slow = runner.spawn_task(JobKind::CreateProject, "6000.0.1f1", "arm64", None, move |task| {
            rx.recv().unwrap();
            task.check()?;
            Ok("never".into())
        });
        runner.cancel(slow);
        tx.send(()).unwrap();
        for _ in 0..100 {
            if runner.list().iter().all(|j| j.finished_ms.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let get = |id| runner.list().into_iter().find(|j| j.id == id).unwrap();
        assert_eq!(get(done).state, JobState::Succeeded);
        assert_eq!(get(done).message, "Created");
        assert_eq!(get(done).project_path.as_deref(), Some("/p"));
        assert_eq!(get(failing).state, JobState::Failed);
        assert!(get(failing).error.unwrap().contains("boom"));
        assert_eq!(get(slow).state, JobState::Cancelled);
    }

    #[test]
    fn cancels_a_running_command() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("unity");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let runner = JobRunner::default();
        let id = runner.start(&script, vec![], JobKind::InstallEditor, "6000.0.1f1", "arm64", vec![]).unwrap();
        assert!(runner.is_running("6000.0.1f1", "arm64"));
        std::thread::sleep(std::time::Duration::from_millis(100));
        runner.cancel(id);
        for _ in 0..100 {
            if runner.list()[0].finished_ms.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let job = &runner.list()[0];
        assert_eq!(job.state, JobState::Cancelled);
        assert!(job.finished_ms.is_some());
    }
}
