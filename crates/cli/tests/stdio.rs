//! harness=false keeps the Rust test runner's own status text out of child stdout.
#[path = "../../../tests/contracts/fixture_cases.rs"]
mod fixture_cases;

use envlume_application::{ErrorCode, Session};
use envlume_domain::{Actor, BindingId, ClientKind};
use envlume_mcp::{ProjectTool, Resolution};
use envlume_storage::{Store, StorePaths};
use std::{
    env,
    ffi::OsString,
    path::Path,
    process::{Command, ExitCode},
};

const BINDING: &str = "44444444-4444-4444-8444-444444444444";

fn resolve(paths: StorePaths, binding: BindingId) -> Resolution {
    let store = Store::open(paths).map_err(|_| ErrorCode::InternalError)?;
    let session = Session::from_registration(store, binding, Actor::Mcp(ClientKind::Codex))?;
    Ok(Box::new(ProjectTool {
        session,
        use_cases: fixture_cases::FixtureCases::default,
    }))
}

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "mcp") {
        let paths = StorePaths::at(env::current_dir().unwrap().join("state")).unwrap();
        return envlume::run(&args, || Ok(paths.clone()), |id| resolve(paths.clone(), id));
    }
    let work = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(work.path().join("state")).unwrap();
    Store::open(paths.clone()).unwrap();
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE;")
        .unwrap();
    db.execute_batch(
        include_str!("../../../crates/storage/tests/fixtures/previous.sql")
            .replacen(
                "projects VALUES",
                "projects(id,name,created_at,updated_at,deleted_at) VALUES",
                1,
            )
            .as_str(),
    )
    .unwrap();
    let id = BindingId(BINDING.parse().unwrap());
    db.execute(
        "INSERT INTO workspace_bindings(id,project_id,alias,canonical_path,trust_state,created_at)
        SELECT ?1,id,'orders-main',?2,'TRUSTED',1700000000000 FROM projects",
        rusqlite::params![id.0, work.path().to_str().unwrap()],
    )
    .unwrap();
    db.execute_batch("INSERT INTO capability_grants(id,project_id,binding_id,capability)
        SELECT randomblob(16),project_id,id,'project.inspect' FROM workspace_bindings;
        INSERT INTO capability_grants(id,project_id,binding_id,target_id,resource_id,capability)
        SELECT randomblob(16),b.project_id,b.id,r.target_id,r.id,'resource.connect'
        FROM workspace_bindings b JOIN environment_targets t ON t.project_id=b.project_id JOIN resource_bindings r ON r.target_id=t.id;
        COMMIT;").unwrap();
    drop(db);
    let client =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contracts/stdio/client.mjs");
    let status = Command::new("node")
        .arg(client)
        .arg(env!("CARGO_BIN_EXE_envlume"))
        .arg(env::current_exe().unwrap())
        .arg(BINDING)
        .current_dir(work.path())
        .status()
        .unwrap();
    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
