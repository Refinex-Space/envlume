use crate::versions::version;
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn native(path: &Path) -> bool {
    let mut magic = [0; 4];
    if !fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
    {
        return false;
    }
    #[cfg(windows)]
    {
        magic.starts_with(b"MZ")
    }
    #[cfg(target_os = "macos")]
    {
        matches!(
            u32::from_be_bytes(magic),
            0xcafebabe | 0xcafebabf | 0xcefaedfe | 0xcffaedfe | 0xfeedface | 0xfeedfacf
        )
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        &magic == b"\x7fELF"
    }
}
fn capture(pipe: impl Read + Send + 'static) -> mpsc::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut out = Vec::new();
        let result = pipe.take(16385).read_to_end(&mut out);
        let _ = tx.send((result.is_ok() && out.len() <= 16384).then_some(out));
    });
    rx
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    Unavailable,
    TimedOut,
}

fn captured(rx: mpsc::Receiver<Option<Vec<u8>>>, end: Instant) -> Result<Vec<u8>, ProbeFailure> {
    rx.recv_timeout(end.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => ProbeFailure::TimedOut,
            mpsc::RecvTimeoutError::Disconnected => ProbeFailure::Unavailable,
        })?
        .ok_or(ProbeFailure::Unavailable)
}
/// Fixed native version commands only. No project cwd, shell, inherited flags,
/// credentials, init files, installer shims or unbounded diagnostic output.
pub(crate) fn installed_version(
    path: &Path,
    tool: &str,
    cwd: &Path,
    deadline: Instant,
) -> Result<String, ProbeFailure> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(ProbeFailure::Unavailable)?
        .to_ascii_lowercase();
    let expected = match tool {
        "jdk" => "java",
        "python" => "python",
        t => t,
    };
    if !(name == expected
        || (tool == "python"
            && name.strip_prefix("python").is_some_and(|s| {
                !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.')
            })))
    {
        return Err(ProbeFailure::Unavailable);
    }
    if !native(path)
        || path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some("shims" | "WindowsApps")))
    {
        return Err(ProbeFailure::Unavailable);
    }
    #[cfg(target_os = "macos")]
    if path.parent() == Some(Path::new("/usr/bin"))
        && matches!(tool, "jdk" | "javac" | "git" | "python" | "rustc" | "cargo")
    {
        return Err(ProbeFailure::Unavailable);
    }
    if matches!(tool, "rustc" | "cargo")
        && path.parent().and_then(Path::parent).is_some_and(|p| {
            p.join("bin")
                .join(if cfg!(windows) {
                    "rustup.exe"
                } else {
                    "rustup"
                })
                .is_file()
        })
    {
        return Err(ProbeFailure::Unavailable);
    }
    let args: &[&str] = match tool {
        "jdk" | "javac" => &["-version"],
        "python" => &["-I", "-S", "--version"],
        "git" | "node" | "uv" | "cargo" | "rustc" => &["--version"],
        _ => return Err(ProbeFailure::Unavailable),
    };
    let mut cmd = Command::new(path);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        if let Some(w) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", w);
        }
    }
    let end = deadline;
    if Instant::now() >= end {
        return Err(ProbeFailure::TimedOut);
    }
    let mut child = cmd.spawn().map_err(|_| ProbeFailure::Unavailable)?;
    let stdout = capture(child.stdout.take().ok_or(ProbeFailure::Unavailable)?);
    let stderr = capture(child.stderr.take().ok_or(ProbeFailure::Unavailable)?);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(ProbeFailure::Unavailable);
                }
                break;
            }
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(5)),
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if outcome.is_ok() {
                    ProbeFailure::TimedOut
                } else {
                    ProbeFailure::Unavailable
                });
            }
        }
    }
    let mut data = captured(stdout, end)?;
    data.push(b'\n');
    data.extend(captured(stderr, end)?);
    // Localized stderr may have a different encoding. Only a validated ASCII
    // version token is retained from either stream.
    let text = String::from_utf8_lossy(&data);
    for line in text.lines().map(str::trim) {
        let token = match tool {
            "jdk" => line
                .strip_prefix("openjdk version \"")
                .or_else(|| line.strip_prefix("java version \""))
                .and_then(|s| s.split_once('"').map(|(v, _)| v)),
            "javac" => line.strip_prefix("javac "),
            "node" => line.strip_prefix('v'),
            "python" => line.strip_prefix("Python "),
            "git" => line.strip_prefix("git version "),
            "uv" => line.strip_prefix("uv "),
            "cargo" => line.strip_prefix("cargo "),
            "rustc" => line.strip_prefix("rustc "),
            _ => None,
        };
        if let Some(raw) = token.and_then(|s| s.split_ascii_whitespace().next()) {
            let v = if tool == "git" {
                raw.split_once(".windows.").map_or(raw, |(v, _)| v)
            } else {
                raw
            };
            if let Some(ver) = version(v) {
                return Ok(ver.to_owned());
            }
        }
    }
    Err(ProbeFailure::Unavailable)
}
