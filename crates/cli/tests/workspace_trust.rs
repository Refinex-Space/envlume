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

fn show_integration_phases(trace_tail: &[u8]) {
    for phase in [
        "checking",
        "check_binding",
        "check_cli",
        "probe",
        "runtime_ready",
        "child_started",
        "initialized",
        "tool_replied",
        "transport_closed",
        "child_exited",
        "runtime_closed",
        "verified",
        "registered",
    ] {
        let needle = format!("envlume_clients: {phase}");
        if trace_tail
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
        {
            eprintln!("T15 observed phase {phase}");
        }
    }
}
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
    #[cfg(windows)]
    if args
        .first()
        .is_some_and(|a| a == "integration" || a == "--integration")
    {
        // portable-pty overlays registry PATH on the parent environment. The preview
        // and its confirmation must see the same isolated client discovery directory.
        command.env("PATH", env::var_os("PATH").unwrap_or_default());
    }
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
        (Some("credential"), Some("cleanup")) => "cleanup",
        (Some("credential"), _) | (Some("environment"), _) => "confirm",
        (Some("integration" | "--integration"), _) => "confirm",
        (Some("task"), Some("enable")) => "enable",
        (Some("task"), Some("run")) => "run",
        (_, Some("delete")) => "delete",
        (_, Some("configure")) => "configure",
        _ => "trust",
    };
    let secret_input = args.first().is_some_and(|s| s == "credential")
        && matches!(action, Some("set" | "repair" | "replace"));
    let mut transcript = Vec::new();
    let prompt = if args
        .first()
        .is_some_and(|a| a == "integration" || a == "--integration")
    {
        "Type confirm to apply:".into()
    } else {
        format!("Type {word} to confirm:")
    };
    let mut answers = vec![(prompt, word)];
    if word == "run" && env::var_os("T07_EXPECT_DOWNLOAD").is_some() {
        answers.push(("Type download to confirm:".into(), "download"));
    }
    const USER: &str = "CLI_T10_fixture_user";
    const PASSWORD: &str = "CLI_T10_fixture_password";
    if secret_input {
        answers.push(("Database username (hidden):".into(), USER));
        answers.push(("Database password (hidden):".into(), PASSWORD));
    }
    for (prompt, answer) in answers {
        let mut output = Vec::new();
        let mut answered_through = 0;

        while !output.windows(prompt.len()).any(|w| w == prompt.as_bytes()) {
            match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(bytes) => {
                    if secret_input {
                        transcript.extend_from_slice(&bytes);
                    }
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
                    if secret_input {
                        panic!("credential terminal prompt unavailable");
                    }
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
    let integration = args
        .first()
        .is_some_and(|a| a == "integration" || a == "--integration");
    let mut pending = Vec::new();
    let mut trace_tail = Vec::new();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if secret_input {
                drop(input);
                drop(pair.master);
                while let Ok(bytes) = rx.recv_timeout(Duration::from_secs(1)) {
                    transcript.extend(bytes);
                }
                for marker in [USER, PASSWORD] {
                    assert!(
                        !transcript
                            .windows(marker.len())
                            .any(|b| b == marker.as_bytes()),
                        "secret echoed into terminal"
                    );
                }
                return if status.success() {
                    ExitCode::SUCCESS
                } else {
                    eprintln!("Credential command returned {status:?}");
                    ExitCode::FAILURE
                };
            }
            if !status.success() {
                let tail: Vec<u8> = rx.try_iter().flatten().take(8192).collect();
                if integration {
                    trace_tail.extend_from_slice(&tail);
                    show_integration_phases(&trace_tail);
                }
                eprintln!(
                    "Confirmed CLI exited {status:?}: {}",
                    String::from_utf8_lossy(&tail)
                );
                return ExitCode::FAILURE;
            }
            return ExitCode::SUCCESS;
        }
        if integration {
            // Behave like a terminal until the command exits, including cursor
            // queries issued after the user answered the confirmation prompt.
            while let Ok(bytes) = rx.try_recv() {
                trace_tail.extend_from_slice(&bytes);
                if trace_tail.len() > 16384 {
                    trace_tail.drain(..trace_tail.len() - 16384);
                }
                pending.extend(bytes);
            }
            let query = b"\x1b[6n";
            while let Some(pos) = pending.windows(query.len()).position(|w| w == query) {
                input.write_all(b"\x1b[1;1R").unwrap();
                input.flush().unwrap();
                pending.drain(..pos + query.len());
            }
            if pending.len() > 8192 {
                pending.drain(..pending.len() - 8192);
            }
        }
        if std::time::Instant::now() > deadline {
            if integration {
                show_integration_phases(&trace_tail);
                #[cfg(windows)]
                if let Some(pid) = child.process_id() {
                    let ps = PathBuf::from(env::var_os("SystemRoot").unwrap())
                        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
                    let script = r#"$procs=Get-CimInstance Win32_Process; $ids=@([int]$env:T15_PID); while ($ids.Count) { $children=@($procs | Where-Object {$ids -contains $_.ParentProcessId}); foreach ($c in $children) { Write-Output ('T15 active child: '+$c.Name) }; $ids=@($children | ForEach-Object {$_.ProcessId}) }"#;
                    let output = Command::new(ps)
                        .args(["-NoProfile", "-NonInteractive", "-Command", script])
                        .env("T15_PID", pid.to_string())
                        .output()
                        .unwrap();
                    eprint!("{}", String::from_utf8_lossy(&output.stdout));
                }
            }
            child.kill().unwrap();
            child.wait().unwrap();
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
        s == "database"
            || s == "connection"
            || s == "credential"
            || s == "environment"
            || s == "report"
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
        "../../tests/policy/environments.mjs",
        "../../tests/credentials/lifecycle.mjs",
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
