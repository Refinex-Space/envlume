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
/// Fixed native version commands only. No project cwd, shell, inherited flags,
/// credentials, init files, installer shims or unbounded diagnostic output.
pub(crate) fn installed_version(
    path: &Path,
    tool: &str,
    cwd: &Path,
    deadline: Instant,
) -> Option<String> {
    let name = path.file_stem()?.to_str()?.to_ascii_lowercase();
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
        return None;
    }
    if !native(path)
        || path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some("shims" | "WindowsApps")))
    {
        return None;
    }
    #[cfg(target_os = "macos")]
    if path.parent() == Some(Path::new("/usr/bin"))
        && matches!(tool, "jdk" | "javac" | "git" | "python" | "rustc" | "cargo")
    {
        return None;
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
        return None;
    }
    let args: &[&str] = match tool {
        "jdk" | "javac" => &["-version"],
        "python" => &["-I", "-S", "--version"],
        "git" | "node" | "uv" | "cargo" | "rustc" => &["--version"],
        _ => return None,
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
    let end = deadline.min(Instant::now() + Duration::from_secs(2));
    if Instant::now() >= end {
        return None;
    }
    let mut child = cmd.spawn().ok()?;
    let stdout = capture(child.stdout.take()?);
    let stderr = capture(child.stderr.take()?);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut data = stdout
        .recv_timeout(end.saturating_duration_since(Instant::now()))
        .ok()??;
    data.push(b'\n');
    data.extend(
        stderr
            .recv_timeout(end.saturating_duration_since(Instant::now()))
            .ok()??,
    );
    let text = std::str::from_utf8(&data).ok()?;
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
                raw.split(".windows.").next()?
            } else {
                raw
            };
            if let Some(ver) = version(v) {
                return Some(ver.to_owned());
            }
        }
    }
    None
}
