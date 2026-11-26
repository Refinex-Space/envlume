use super::registration::confirm;
use envlume_application::{
    ErrorCode, Session,
    projects::StorageError,
    result::{Payload, Report, TaskResult},
    tasks::{AuthorizedTask, Template},
    workspaces::ManagementError,
};
use envlume_domain::{Actor, BindingId, Capability};
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

struct Failure(ErrorCode, &'static str);
impl From<ErrorCode> for Failure {
    fn from(code: ErrorCode) -> Self {
        Self(code, code.summary())
    }
}
impl From<StorageError> for Failure {
    fn from(e: StorageError) -> Self {
        Self(
            if e == StorageError::StorageBusy {
                ErrorCode::Timeout
            } else {
                ErrorCode::InternalError
            },
            e.message(),
        )
    }
}
impl From<ManagementError> for Failure {
    fn from(e: ManagementError) -> Self {
        use ManagementError::*;
        let code = match e {
            Storage(e) => return e.into(),
            InvalidPath | NotFound => ErrorCode::InvalidInput,
            WorkspaceChanged => ErrorCode::ProjectNotTrusted,
            ConfirmationRequired | ProjectMismatch | SharedProfile => ErrorCode::PolicyDenied,
        };
        Self(code, e.diagnostic().1)
    }
}
impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock => {
                Self(ErrorCode::PolicyDenied, "All local task slots are in use.")
            }
            io::ErrorKind::Interrupted => Self(
                ErrorCode::PolicyDenied,
                "The caller disconnected or cancelled.",
            ),
            _ => Self(ErrorCode::InternalError, "Local task control failed."),
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
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
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
    // Detached workers cannot write to a Windows caller's console handles.
    // Relay only the worker's report and diagnostics through dedicated pipes.
    let mut stdout = worker.stdout.take().expect("worker stdout");
    let mut stderr = worker.stderr.take().expect("worker stderr");
    let out = thread::spawn(move || io::copy(&mut stdout, &mut io::stdout().lock()));
    let err = thread::spawn(move || io::copy(&mut stderr, &mut io::stderr().lock()));
    let status = loop {
        if let Some(status) = worker.try_wait()? {
            break status;
        }
        if caller.disconnected() {
            drop(lifetime);
            break worker.wait()?;
        }
        thread::sleep(Duration::from_millis(20));
    };
    out.join().map_err(|_| ErrorCode::InternalError)??;
    err.join().map_err(|_| ErrorCode::InternalError)??;
    Ok(status.code().and_then(|v| v.try_into().ok()).unwrap_or(7))
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
    let mut report = Report::new("task_run");
    let request = report.request_id;
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
    report.duration_ms = outcome.duration_ms;
    report.binding_alias = Some(task.binding_alias);
    report.capability = Some(Capability::TaskRun(task.task).wire_name());
    report.evidence_id = Some(outcome.evidence_id);
    match (outcome.status.as_str(), outcome.exit_code) {
        ("timeout", _) => report.fail(ErrorCode::Timeout, "Task exceeded its time limit."),
        ("cancelled", _) => report.fail(ErrorCode::PolicyDenied, "Task cancelled."),
        (_, Some(code)) => {
            report.result = Some(Payload::Task(TaskResult {
                exit_code: code,
                stdout_summary: outcome.stdout_summary.to_string(),
                stderr_summary: outcome.stderr_summary.to_string(),
            }));
            let verdict = if code == 0 {
                "Build passed"
            } else {
                "Build failed"
            };
            let versions = outcome
                .versions
                .iter()
                .map(|(key, version)| format!("{key}={version}"))
                .collect::<Vec<_>>()
                .join(", ");
            report.summary = format!("{verdict} ({versions})");
        }
        _ => report.fail(
            ErrorCode::InternalError,
            "Task terminated without an exit code.",
        ),
    }
    print_report(&report, machine)
}

fn print_report(report: &Report, machine: bool) -> Result<u8> {
    if machine {
        println!(
            "{}",
            serde_json::to_string(report).map_err(|_| ErrorCode::InternalError)?
        );
    } else {
        println!("{}", report.human_text());
        if let Some(Payload::Tasks { tasks }) = &report.result {
            for task in tasks {
                println!("{}\t{} s", task.task_id.as_str(), task.timeout_seconds);
            }
        }
    }
    Ok(report.exit_code())
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
            return print_report(&session(&db, id)?.task_list(), machine);
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
        Err(Failure(code, msg)) => {
            if json {
                let operation = match args.first().and_then(|op| op.to_str()) {
                    Some("run" | "__worker") => "task_run",
                    Some("list") => "task_list",
                    _ => "unknown",
                };
                let mut report = Report::new(operation);
                report.fail(code, msg);
                println!("{}", json!(report));
            }
            eprintln!("{msg}");
            ExitCode::from(code.exit_code())
        }
    }
}
