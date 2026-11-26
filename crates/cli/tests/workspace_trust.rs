//! Default: run the real dispatcher and resolver with an isolated storage root.
//! CI may also pass the installed CLI, in its disposable runner profile.
use envlume_storage::StorePaths;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    env,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::mpsc,
    thread,
    time::Duration,
};

fn approve(exe: &Path, args: &[OsString]) -> ExitCode {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(exe);
    command.args(args);
    command.cwd(env::current_dir().unwrap());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut input = pair.master.take_writer().unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut output = Vec::new();
    while !output
        .windows(b"to confirm: ".len())
        .any(|w| w == b"to confirm: ")
    {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(bytes) => output.extend(bytes),
            Err(e) => {
                let _ = child.kill();
                panic!(
                    "missing confirmation prompt ({e}): {}",
                    String::from_utf8_lossy(&output)
                );
            }
        }
    }
    let answer = if args.iter().any(|a| a == "delete") {
        "delete\r"
    } else {
        "trust\r"
    };
    input.write_all(answer.as_bytes()).unwrap();
    input.flush().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            panic!("CLI did not exit after confirmation");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|s| s == "approve") {
        return approve(Path::new(&args[1]), &args[2..]);
    }
    if args.first().is_some_and(|s| s == "project" || s == "mcp") {
        let scope = || {
            StorePaths::at(PathBuf::from(
                env::var_os("T05_FIXTURE_STORE").expect("isolated test root"),
            ))
        };
        return envlume::run(&args, scope, |id| envlume::resolve(scope(), id));
    }
    let tmp = tempfile::tempdir().unwrap();
    let helper = env::current_exe().unwrap();
    let exe = env::var_os("T05_NATIVE_CLI")
        .map(PathBuf::from)
        .map(|p| p.canonicalize().unwrap())
        .unwrap_or_else(|| helper.clone());
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workspace/trust.mjs");
    let status = Command::new("node")
        .arg(script)
        .arg(exe)
        .arg(helper)
        .env("T05_FIXTURE_STORE", tmp.path().join("state"))
        .current_dir(tmp.path())
        .status()
        .unwrap();
    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
