use envlume_application::{
    ErrorCode, Session,
    projects::StorageError,
    workspaces::{ManagementError, WorkspaceRepository, add_project, inspect_project},
};
use envlume_domain::{Actor, BindingId, ProjectId};
use envlume_mcp::Resolution;
use envlume_storage::{LocalRegistration, Store, StorePaths};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{self, IsTerminal, Write},
    path::Path,
    process::ExitCode,
};

pub(super) fn session(
    paths: Result<StorePaths, StorageError>,
    id: BindingId,
) -> Result<Session<LocalRegistration>, ErrorCode> {
    let paths = paths.map_err(|_| ErrorCode::InternalError)?;
    if !paths
        .database()
        .try_exists()
        .map_err(|_| ErrorCode::InternalError)?
    {
        return Err(ErrorCode::ProjectNotTrusted);
    }
    let store = Store::open(paths).map_err(|e| {
        if e == StorageError::StorageBusy {
            ErrorCode::Timeout
        } else {
            ErrorCode::InternalError
        }
    })?;
    let origin = std::env::current_dir().map_err(|_| ErrorCode::ProjectNotTrusted)?;
    Session::from_registration(LocalRegistration { store, origin }, id, Actor::LocalStdio)
}
pub fn resolve(paths: Result<StorePaths, StorageError>, id: BindingId) -> Resolution {
    Ok(Box::new(envlume_mcp::DiagnosticProject {
        session: session(paths, id)?,
        inspector: envlume_toolchain::LocalToolchain,
    }))
}

pub(super) fn confirm(prompt: &str, word: &str) -> bool {
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        return false;
    }
    eprint!("{prompt}\nType {word} to confirm: ");
    if io::stderr().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok() && answer.trim_end_matches(['\r', '\n']) == word
}
fn project_id(s: &OsString) -> Result<ProjectId, ManagementError> {
    s.to_str()
        .and_then(|s| s.parse().ok())
        .map(ProjectId)
        .ok_or(ManagementError::InvalidPath)
}

fn execute(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> Result<Value, ManagementError> {
    let open = || Ok::<_, ManagementError>(Store::open(locate()?)?);
    match args {
        [cmd, path, ..] if cmd == "add" => {
            let project = match &args[2..] {
                [] => None,
                [flag, id] if flag == "--project" => Some(project_id(id)?),
                _ => return Err(ManagementError::InvalidPath),
            };
            let result = add_project(&open()?, Path::new(path), project, |ws, p| {
                let project = p.map_or_else(|| "new project".into(), |id| id.0.to_string());
                confirm(
                    &format!(
                        "Trust workspace {:?} -> {:?} for {project}?\nGit: {}; stack hints: {:?}\nGrant: project.inspect only.",
                        ws.entry_path, ws.canonical_path, ws.git_kind, ws.hints
                    ),
                    "trust",
                )
            })?;
            Ok(json!({"schemaVersion":1,"binding":result}))
        }
        [cmd, flag, path] if cmd == "inspect" && flag == "--path" => {
            let inspected = inspect_project(&open()?, Path::new(path))?;
            Ok(
                json!({"schemaVersion":1,"binding":inspected.binding,"workspace":inspected.workspace}),
            )
        }
        [cmd, flag, id] if cmd == "revoke" && flag == "--binding" => {
            let id = id
                .to_str()
                .and_then(|s| s.parse().ok())
                .map(BindingId)
                .ok_or(ManagementError::InvalidPath)?;
            open()?.revoke(id)?;
            Ok(json!({"schemaVersion":1,"bindingId":id,"state":"REVOKED"}))
        }
        [cmd, flag, id] if cmd == "delete" && flag == "--project" => {
            let id = project_id(id)?;
            if !confirm(
                &format!(
                    "Soft-delete project {} and revoke all of its workspace grants?",
                    id.0
                ),
                "delete",
            ) {
                return Err(ManagementError::ConfirmationRequired);
            }
            open()?.delete_project(id)?;
            Ok(json!({"schemaVersion":1,"projectId":id,"state":"DELETED"}))
        }
        _ => Err(ManagementError::InvalidPath),
    }
}
pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, json_mode) = match args.split_last() {
        Some((last, rest)) if last == "--json" => (rest, true),
        _ => (args, false),
    };
    match execute(args, locate) {
        Ok(value) => {
            if json_mode {
                println!("{value}");
            } else if let Some(binding) = value.get("binding") {
                println!(
                    "Project {}  Binding {}  {}",
                    binding["projectId"], binding["bindingId"], binding["state"]
                );
                println!(
                    "Workspace {}  Path {}",
                    binding["alias"], binding["canonicalPath"]
                );
                if let Some(observed) = value.get("workspace").filter(|w| !w.is_null()) {
                    println!(
                        "Git {}  Stack hints {}",
                        observed["gitKind"], observed["hints"]
                    );
                }
            } else {
                println!("{}", value["state"]);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let (code, msg, exit) = e.diagnostic();
            if json_mode {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"errorCode":code,"summary":msg})
                );
            }
            eprintln!("{msg}");
            ExitCode::from(exit)
        }
    }
}
