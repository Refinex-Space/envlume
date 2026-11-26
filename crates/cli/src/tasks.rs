use super::registration::confirm;
use envlume_application::{
    ErrorCode, Session,
    projects::StorageError,
    tasks::{AuthorizedTask, Template},
    workspaces::ManagementError,
};
use envlume_domain::{Actor, BindingId, RequestId};
use envlume_execution::{Caller, Review, Slot};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use serde_json::json;
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
    process::{Command, ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Failure(&'static str, &'static str, u8);
impl From<ErrorCode> for Failure {
    fn from(e: ErrorCode) -> Self {
        Self(
            match e {
                ErrorCode::InvalidInput => "INVALID_INPUT",
                ErrorCode::BindingRevoked => "BINDING_REVOKED",
                ErrorCode::ProjectNotTrusted => "PROJECT_NOT_TRUSTED",
                ErrorCode::PolicyDenied => "POLICY_DENIED",
                ErrorCode::Timeout => "TIMEOUT",
                ErrorCode::ToolchainMissing => "TOOLCHAIN_MISSING",
                _ => "INTERNAL_ERROR",
            },
            e.summary(),
            e.exit_code(),
        )
    }
}
impl From<StorageError> for Failure {
    fn from(e: StorageError) -> Self {
        Self(
            "STORAGE_ERROR",
            e.message(),
            if e == StorageError::StorageBusy { 6 } else { 7 },
        )
    }
}
impl From<ManagementError> for Failure {
    fn from(e: ManagementError) -> Self {
        let (c, m, x) = e.diagnostic();
        Self(c, m, x)
    }
}
impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock => Self("TASK_BUSY", "All local task slots are in use.", 3),
            io::ErrorKind::Interrupted => {
                Self("CANCELLED", "The caller disconnected or cancelled.", 3)
            }
            _ => Self("INTERNAL_ERROR", "Local task control failed.", 7),
        }
    }
}
type Result<T> = std::result::Result<T, Failure>;
fn binding(arg: &OsString) -> Result<BindingId> {
    arg.to_str()
        .and_then(|v| v.parse().ok())
        .map(BindingId)
        .ok_or(ErrorCode::InvalidInput.into())
}
fn key(arg: &OsString) -> Result<Template> {
    Ok(Template::parse(arg.to_str().unwrap_or(""))?)
}
fn session(store: &Store, id: BindingId) -> Result<Session<LocalRegistration>> {
    Ok(Session::from_registration(
        LocalRegistration {
            store: store.clone(),
            origin: env::current_dir()?,
        },
        id,
        Actor::Cli,
    )?)
}
fn approved(store: &Store, task: &AuthorizedTask, review: &Review) -> Result<bool> {
    Ok(store.task_approved(task, &review.digest, review.distribution_digest.as_deref())?)
}
fn first_use(
    store: &Store,
    s: &Session<LocalRegistration>,
    task: &AuthorizedTask,
    review: &Review,
) -> Result<()> {
    if approved(store, task, review)? {
        return Ok(());
    }
    let prompt = format!(
        "First build: {} in {:?}. Repository scripts and plugins run with your OS user privileges. Allow this task?",
        task.template.key(),
        task.root
    );
    if !confirm(&prompt, "run") {
        return Err(ErrorCode::PolicyDenied.into());
    }
    if !review.urls.is_empty() {
        let prompt = format!(
            "This workspace wrapper may download from the following approved URL(s):\n{}",
            review.urls.join("\n")
        );
        if !confirm(&prompt, "download") {
            return Err(ErrorCode::PolicyDenied.into());
        }
    }
    let checked = s.authorize_task(task.template)?;
    if envlume_execution::review(&checked)?.digest != review.digest {
        return Err(ErrorCode::PolicyDenied.into());
    }
    Ok(store.approve_task(
        &checked,
        &review.digest,
        review.distribution_digest.as_deref(),
    )?)
}
fn supervise(template: Template, id: BindingId, machine: bool) -> Result<u8> {
    let caller = Caller::capture()?;
    if caller.disconnected() {
        return Err(io::Error::from(io::ErrorKind::Interrupted).into());
    }
    let mut cmd = Command::new(env::current_exe()?);
    cmd.args([
        "task",
        "__worker",
        template.key(),
        "--binding",
        &id.0.to_string(),
    ])
    .stdin(Stdio::piped());
    if machine {
        cmd.arg("--json");
    }
    // The worker owns the tree and slot. Losing the CLI (including SIGKILL)
    // closes its private pipe. It must not receive the foreground terminal signal.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000200);
    }
    let mut worker = cmd.spawn()?;
    let mut lifetime = worker.stdin.take().expect("worker pipe");
    if let Err(e) = lifetime.write_all(b"R") {
        drop(lifetime);
        worker.wait()?;
        return Err(e.into());
    }
    loop {
        if let Some(status) = worker.try_wait()? {
            return Ok(status.code().and_then(|v| v.try_into().ok()).unwrap_or(7));
        }
        if caller.disconnected() {
            drop(lifetime);
            return Ok(worker
                .wait()?
                .code()
                .and_then(|v| v.try_into().ok())
                .unwrap_or(7));
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn worker(
    paths: &StorePaths,
    store: &Store,
    s: &Session<LocalRegistration>,
    task: AuthorizedTask,
    review: Review,
    machine: bool,
) -> Result<u8> {
    if !approved(store, &task, &review)? || io::stdin().is_terminal() {
        return Err(ErrorCode::PolicyDenied.into());
    }
    let mut start = [0];
    io::stdin().read_exact(&mut start)?;
    if start != *b"R" {
        return Err(ErrorCode::PolicyDenied.into());
    }
    let disconnected = Arc::new(AtomicBool::new(false));
    let flag = disconnected.clone();
    thread::spawn(move || {
        let _ = io::stdin().read(&mut [0]);
        flag.store(true, Ordering::Release);
    });
    let request = RequestId::generate();
    let directory = paths.root().join("tmp");
    let slot = Slot::acquire(&directory, task.binding, task.template, request)?;
    let temp = tempfile::tempdir_in(&directory)?;
    let home = directory.join(format!("build-home-{}", task.binding.0));
    private_home(&home)?;
    let stopped =
        || -> io::Result<bool> { Ok(disconnected.load(Ordering::Acquire) || slot.cancelled()?) };
    if stopped()? {
        return Err(io::Error::from(io::ErrorKind::Interrupted).into());
    }
    let launch = envlume_execution::prepare(&task, &review, &home, temp.path())?;
    let latest = s.authorize_task(task.template)?;
    if envlume_execution::review(&latest)?.digest != review.digest
        || !approved(store, &latest, &review)?
    {
        return Err(ErrorCode::PolicyDenied.into());
    }
    let mut last = Instant::now();
    let outcome = envlume_execution::execute(launch, task.template, request, task.timeout, || {
        if stopped()? {
            return Ok(true);
        }
        if last.elapsed() >= Duration::from_millis(250) {
            last = Instant::now();
            return Ok(!s.authorize_task(task.template).is_ok_and(|t| {
                t.grant == task.grant
                    && t.profile == task.profile
                    && approved(store, &t, &review).unwrap_or(false)
            }));
        }
        Ok(false)
    })?;
    let code = if outcome.status == "timeout" { 6 } else { 0 };
    if machine {
        println!(
            "{}",
            serde_json::to_string(&outcome).map_err(|_| ErrorCode::InternalError)?
        );
    } else {
        println!(
            "{} {} (exit {:?}, {} ms)\nrequestId: {}\nevidenceId: {}\nstdout: {} bytes{}; stderr: {} bytes{}",
            task.template.key(),
            outcome.status,
            outcome.exit_code,
            outcome.duration_ms,
            request.0,
            outcome.evidence_id.0,
            outcome.stdout_summary.bytes,
            if outcome.stdout_summary.truncated {
                " (truncated)"
            } else {
                ""
            },
            outcome.stderr_summary.bytes,
            if outcome.stderr_summary.truncated {
                " (truncated)"
            } else {
                ""
            }
        );
    }
    Ok(code)
}
fn private_home(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    };
    #[cfg(not(unix))]
    let created = fs::create_dir(path);
    match created {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        r => r?,
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.is_symlink() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}
fn dispatch(
    args: &[OsString],
    machine: bool,
    locate: impl FnOnce() -> std::result::Result<StorePaths, StorageError>,
) -> Result<u8> {
    let open = || -> Result<_> {
        let paths = locate()?;
        let db = Store::open(paths.clone())?;
        Ok((paths, db))
    };
    match args {
        [op, flag, id] if op == "list" && flag == "--binding" => {
            let id = binding(id)?;
            let (_, db) = open()?;
            let tasks = session(&db, id)?.list_tasks()?;
            if machine {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"requestId":RequestId::generate(),"tasks":tasks})
                );
            } else {
                for t in tasks {
                    println!("{}\t{} s", t.task_id.as_str(), t.timeout_seconds);
                }
            }
        }
        [op, name, flag, id, rest @ ..] if flag == "--binding" => {
            let template = key(name)?;
            let id = binding(id)?;
            if op == "enable" {
                let timeout = match rest {
                    [] => 600,
                    [flag, sec] if flag == "--timeout" => sec
                        .to_str()
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|n| (1..=3600).contains(n))
                        .ok_or(ErrorCode::InvalidInput)?,
                    _ => return Err(ErrorCode::InvalidInput.into()),
                };
                let (_, db) = open()?;
                session(&db, id)?;
                if !confirm(
                    &format!(
                        "Enable {} for binding {} with fixed arguments {:?} and timeout {}s?",
                        template.key(),
                        id.0,
                        template.arguments(),
                        timeout
                    ),
                    "enable",
                ) {
                    return Err(ErrorCode::PolicyDenied.into());
                }
                db.enable_task(id, &env::current_dir()?, template, timeout)?;
            } else {
                if !rest.is_empty() {
                    return Err(ErrorCode::InvalidInput.into());
                }
                let (paths, db) = open()?;
                let s = session(&db, id)?;
                match op.to_str() {
                    Some("disable") => db.disable_task(id, &env::current_dir()?, template)?,
                    Some("cancel") => {
                        let n = envlume_execution::cancel(&paths.root().join("tmp"), id, template)?;
                        if n == 0 {
                            return Err(ErrorCode::InvalidInput.into());
                        }
                    }
                    Some("run" | "__worker") => {
                        let task = s.authorize_task(template)?;
                        let review = envlume_execution::review(&task)?;
                        if op == "__worker" {
                            return worker(&paths, &db, &s, task, review, machine);
                        }
                        first_use(&db, &s, &task, &review)?;
                        return supervise(template, id, machine);
                    }
                    _ => return Err(ErrorCode::InvalidInput.into()),
                }
            }
            if machine {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"task":template.key(),"operation":op.to_str(),"ok":true})
                );
            } else {
                println!("{}: {}", op.to_string_lossy(), template.key());
            }
        }
        _ => return Err(ErrorCode::InvalidInput.into()),
    }
    Ok(0)
}
pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> std::result::Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, json) = match args.split_last() {
        Some((flag, rest)) if flag == "--json" => (rest, true),
        _ => (args, false),
    };
    match dispatch(args, json, locate) {
        Ok(code) => ExitCode::from(code),
        Err(Failure(code, msg, exit)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"schemaVersion":1,"requestId":RequestId::generate(),"errorCode":code,"summary":msg})
                );
            }
            eprintln!("{msg}");
            ExitCode::from(exit)
        }
    }
}
