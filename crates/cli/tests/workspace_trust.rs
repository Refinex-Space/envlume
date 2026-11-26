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
    let action = args.get(1).and_then(|v| v.to_str());
    let word = match (args.first().and_then(|v| v.to_str()), action) {
        (Some("task"), Some("enable")) => "enable",
        (Some("task"), Some("run")) => "run",
        (_, Some("delete")) => "delete",
        (_, Some("configure")) => "configure",
        _ => "trust",
    };
    let mut answers = vec![word];
    if word == "run" && env::var_os("T07_EXPECT_DOWNLOAD").is_some() {
        answers.push("download");
    }
    for answer in answers {
        let mut output = Vec::new();
        let mut answered_through = 0;
        let prompt = format!("Type {answer} to confirm:");
        while !output.windows(prompt.len()).any(|w| w == prompt.as_bytes()) {
            match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(bytes) => {
                    output.extend(bytes);
                    let query = b"\x1b[6n";
                    while let Some(pos) = output[answered_through..]
                        .windows(query.len())
                        .position(|b| b == query)
                    {
                        input.write_all(b"\x1b[1;1R").unwrap();
                        input.flush().unwrap();
                        answered_through += pos + query.len();
                    }
                }
                Err(e) => {
                    let _ = child.kill();
                    panic!(
                        "missing {answer} confirmation ({e}): {}",
                        String::from_utf8_lossy(&output)
                    );
                }
            }
        }
        if answer == "download" {
            let url = env::var("T07_EXPECT_DOWNLOAD").unwrap();
            assert!(
                String::from_utf8_lossy(&output).contains(&url),
                "distribution URL was not displayed"
            );
        }
        input.write_all(format!("{answer}\r").as_bytes()).unwrap();
        input.flush().unwrap();
    }
    let deadline = std::time::Instant::now()
        + if word == "run" {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(30)
        };
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if !status.success() {
                let tail: Vec<u8> = rx.try_iter().flatten().take(8192).collect();
                eprintln!(
                    "Confirmed CLI exited {status:?}: {}",
                    String::from_utf8_lossy(&tail)
                );
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
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
    // Native stand-ins live beside a fixture-version file in a synthetic JDK.
    // The installed CLI has no fixture switch or test environment override.
    let invoked = env::current_exe().unwrap();
    if matches!(
        invoked.file_stem().and_then(|n| n.to_str()),
        Some("java" | "javac")
    ) {
        assert_eq!(args, ["-version"]);
        if invoked.with_extension("fixture-observe").exists() {
            std::fs::write(invoked.with_extension("fixture-probed"), "probed").unwrap();
        }
        if invoked.with_extension("fixture-delay").exists() {
            thread::sleep(Duration::from_millis(2400));
        }
        assert!(env::var_os("T06_TEST_PASSWORD").is_none());
        assert!(env::var_os("JAVA_TOOL_OPTIONS").is_none());
        assert!(!Path::new("pom.xml").exists());
        let v = std::fs::read_to_string(invoked.with_extension("fixture-version")).unwrap();
        if invoked.file_stem().unwrap() == "java" {
            eprintln!("openjdk version \"{v}\"");
        } else {
            println!("javac {v}");
        }
        return ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|s| s == "approve") {
        return approve(Path::new(&args[1]), &args[2..]);
    }
    if args.first().is_some_and(|s| {
        s == "report"
            || s == "project"
            || s == "mcp"
            || s == "doctor"
            || s == "toolchain"
            || s == "task"
    }) {
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
    for script in [
        "../../tests/workspace/trust.mjs",
        "../../tests/toolchain/doctor.mjs",
        "../../tests/execution/tasks.mjs",
        "../../tests/execution/evidence.mjs",
    ] {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
        let status = Command::new("node")
            .arg(script)
            .arg(&exe)
            .arg(&helper)
            .env("T05_FIXTURE_STORE", tmp.path().join("state"))
            .current_dir(tmp.path())
            .status()
            .unwrap();
        if !status.success() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
