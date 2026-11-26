//! Local execution with a closed command vocabulary and bounded, redacted results.
mod caller;
mod limit;
mod tree;
pub use caller::Caller;
use envlume_domain::{EvidenceId, RequestId, UtcDateTime};
pub use limit::{Slot, cancel};
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
pub use envlume_audit::{OutputSummary, RunResult};
fn drain(
    mut pipe: impl Read + Send + 'static,
    maven: bool,
) -> mpsc::Receiver<io::Result<OutputSummary>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut bytes = 0u64;
            let mut prefix = Vec::new();
            let mut buf = [0; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes = bytes.saturating_add(n as u64);
                        let remaining = STREAM_LIMIT as usize - prefix.len();
                        prefix.extend_from_slice(&buf[..n.min(remaining)]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            // Forward only fixed vocabulary. Repository text and arbitrary
            // key/value data never leave this bounded, in-memory prefix.
            let mut events = Vec::new();
            let mut assertions = None;
            // Ignore a partial final line if the prefix ends in the middle of output.
            if bytes > STREAM_LIMIT {
                prefix.truncate(prefix.iter().rposition(|b| *b == b'\n').unwrap_or(0));
            }
            for line in prefix.split(|b| *b == b'\n').filter(|_| maven) {
                if let Some(counts) = envlume_audit::Assertions::from_summary(line.trim_ascii()) {
                    assertions = Some(counts);
                }
                let event = match line.trim_ascii() {
                    b"[INFO] BUILD SUCCESS" => "BUILD SUCCESS",
                    b"[INFO] BUILD FAILURE" => "BUILD FAILURE",
                    _ => continue,
                };
                if !events.iter().any(|s| s == event) {
                    events.push(event.to_owned());
                }
            }
            Ok(OutputSummary {
                bytes,
                truncated: bytes > STREAM_LIMIT,
                events,
                assertions,
            })
        })();
        let _ = tx.send(result);
    });
    rx
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
    let maven = task == envlume_application::tasks::Template::MavenTest;
    let stdout = drain(out, maven);
    let stderr = drain(err, maven);
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
