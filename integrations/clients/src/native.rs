use envlume_application::integrations::Fault;
use std::time::Duration;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::io::AsyncReadExt;

pub(super) fn native_file(candidate: &Path, root: &Path) -> Option<PathBuf> {
    if candidate.starts_with(root) {
        return None;
    }
    let parent = envlume_workspace::local_canonical(candidate.parent()?).ok()?;
    if parent.starts_with(root) {
        return None;
    }
    let path = envlume_workspace::local_canonical(candidate).ok()?;
    if path.starts_with(root) {
        return None;
    }
    let m = path.metadata().ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    if !m.is_file() {
        return None;
    }
    // The native distribution is Mach-O (including universal binaries) or PE.
    let mut magic = [0; 4];
    std::fs::File::open(&path)
        .ok()?
        .read_exact(&mut magic)
        .ok()?;
    #[cfg(target_os = "macos")]
    let native = matches!(
        u32::from_be_bytes(magic),
        0xcffaedfe | 0xfeedfacf | 0xcafebabe | 0xbebafeca | 0xcafebabf | 0xbfbafeca
    );
    #[cfg(windows)]
    let native = magic.starts_with(b"MZ");
    #[cfg(not(any(target_os = "macos", windows)))]
    let native = magic == *b"\x7fELF";
    native.then_some(path)
}

pub(super) fn command(exe: &Path, root: &Path) -> Result<tokio::process::Command, Fault> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.current_dir(root)
        .env_clear()
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in [
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "SystemRoot",
        "TEMP",
        "TMP",
        "TMPDIR",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "SystemDrive",
    ] {
        if let Some(v) = std::env::var_os(key) {
            cmd.env(key, v);
        }
    }
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
        let win = PathBuf::from(std::env::var_os("SystemRoot").ok_or(Fault::ClientUnavailable)?);
        let system = win.join("System32");
        cmd.env("WINDIR", &win)
            .env("ComSpec", system.join("cmd.exe"))
            .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
            .env("PATH", {
                let mut path = vec![system];
                if let Some(dir) = std::env::var_os("ProgramFiles") {
                    path.push(PathBuf::from(dir).join("Git/bin"));
                }
                std::env::join_paths(path).map_err(|_| Fault::UnsafePath)?
            });
    }
    #[cfg(unix)]
    cmd.env("PATH", "/usr/bin:/bin");
    Ok(cmd)
}

pub(super) fn output(mut cmd: tokio::process::Command) -> Result<String, Fault> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Fault::Storage)?;
    rt.block_on(async {
        let mut child = cmd.spawn().map_err(|_| Fault::ClientUnavailable)?;
        let pipe = child.stdout.take().expect("piped stdout");
        let attempt = tokio::time::timeout(Duration::from_secs(15), async {
            let mut data = Vec::new();
            pipe.take(32769)
                .read_to_end(&mut data)
                .await
                .map_err(|_| Fault::ClientRejected)?;
            if data.len() > 32768 {
                return Err(Fault::ClientRejected);
            }
            if !child.wait().await.map_err(|_| Fault::Storage)?.success() {
                return Err(Fault::ClientRejected);
            }
            String::from_utf8(data).map_err(|_| Fault::ClientRejected)
        })
        .await
        .unwrap_or(Err(Fault::Timeout));
        if child.try_wait().map_err(|_| Fault::Storage)?.is_none() {
            child.kill().await.map_err(|_| Fault::Storage)?;
        }
        attempt
    })
}
