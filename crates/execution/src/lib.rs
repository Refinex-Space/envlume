//! Local execution with a closed command vocabulary and bounded, redacted results.
mod caller;
mod limit;
mod tree;
pub use caller::{ParentWatch, disconnected};
use envlume_domain::{EvidenceId, RequestId, UtcDateTime};
pub use limit::{Slot, cancel};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, Read},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tree::Tree;

pub struct Launch {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: BTreeMap<OsString, OsString>,
    pub(crate) versions: BTreeMap<String, String>,
}
const STREAM_LIMIT: u64 = 32 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSummary {
    pub bytes: u64,
    pub truncated: bool,
}
fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<io::Result<OutputSummary>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut bytes = 0u64;
            let mut buf = [0; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => bytes = bytes.saturating_add(n as u64),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            // The public summary retains sizes only. Arbitrary repository text
            // may contain credentials; never persist or forward the raw stream.
            Ok(OutputSummary {
                bytes,
                truncated: bytes > STREAM_LIMIT,
            })
        })();
        let _ = tx.send(result);
    });
    rx
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub schema_version: u8,
    pub request_id: RequestId,
    pub evidence_id: EvidenceId,
    pub task_id: String,
    pub timestamp_utc: String,
    pub duration_ms: u64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub versions: BTreeMap<String, String>,
    pub stdout_summary: OutputSummary,
    pub stderr_summary: OutputSummary,
}
pub fn execute(
    launch: Launch,
    task: envlume_application::tasks::Template,
    request: RequestId,
    timeout: Duration,
    mut cancelled: impl FnMut() -> io::Result<bool>,
) -> io::Result<RunResult> {
    let start = Instant::now();
    let timestamp = UtcDateTime::now()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(io::Error::other)?;
    let mut cmd = Command::new(&launch.program);
    cmd.current_dir(launch.cwd)
        .args(launch.args)
        .env_clear()
        .envs(launch.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cancelled()? {
        return Err(io::ErrorKind::Interrupted.into());
    }
    #[cfg(unix)]
    let mut child = Tree::spawn(&mut cmd)?;
    #[cfg(windows)]
    let mut child = Tree::spawn(cmd)?;
    let (out, err) = child.output();
    let stdout = drain(out);
    let stderr = drain(err);
    let (status, exit_code) = loop {
        if cancelled()? {
            child.stop()?;
            break ("cancelled", None);
        }
        if start.elapsed() >= timeout {
            child.stop()?;
            break ("timeout", None);
        }
        if let Some(status) = child.finished()? {
            break (
                if status.success() {
                    "success"
                } else {
                    "failed"
                },
                status.code(),
            );
        }
        thread::sleep(Duration::from_millis(15));
    };
    let summary = |reader: mpsc::Receiver<io::Result<OutputSummary>>| {
        reader
            .recv_timeout(Duration::from_secs(2))
            .map_err(io::Error::other)?
    };
    Ok(RunResult {
        schema_version: 1,
        request_id: request,
        evidence_id: EvidenceId::generate(),
        task_id: task.key().into(),
        timestamp_utc: timestamp,
        duration_ms: start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        status: status.into(),
        exit_code,
        versions: launch.versions,
        stdout_summary: summary(stdout)?,
        stderr_summary: summary(stderr)?,
    })
}
mod plan;
pub use plan::{Review, prepare, review};
