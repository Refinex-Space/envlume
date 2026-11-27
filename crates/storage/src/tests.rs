use super::*;
use envlume_application::ports::Repository;
use envlume_domain::BindingId;
use std::{
    fs,
    process::{Child, Command},
    thread,
    time::Instant,
};
use tempfile::{TempDir, tempdir};

const V1: &str = include_str!("../migrations/001_baseline.sql");
const FIXTURE: &str = include_str!("../tests/fixtures/previous.sql");
const CANARY: &str = "TEST_ONLY_PASSWORD_73ABA087";
const BINDING: &str = "44444444-4444-4444-8444-444444444444";

fn isolated() -> (TempDir, StorePaths) {
    let dir = tempdir().unwrap();
    let paths = StorePaths::at(dir.path().join("Envlume")).unwrap();
    (dir, paths)
}
fn previous(paths: &StorePaths) -> Connection {
    paths.prepare().unwrap();
    paths::create_file(&paths.database()).unwrap();
    let conn = connect(&paths.database(), true).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; BEGIN IMMEDIATE;")
        .unwrap();
    conn.execute_batch(V1).unwrap();
    conn.execute_batch("PRAGMA application_id=0x454e564c; PRAGMA user_version=1; INSERT INTO schema_migrations VALUES (1,1700000000000);").unwrap();
    conn.execute_batch(FIXTURE).unwrap();
    conn.execute_batch("COMMIT;").unwrap();
    conn
}
fn backups(paths: &StorePaths) -> Vec<std::path::PathBuf> {
    fs::read_dir(paths.backups())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn fresh_schema_enforces_relationships_business_keys_and_sizes() {
    let (_dir, paths) = isolated();
    let store = Store::open(paths.clone()).unwrap();
    assert!(store.active_projects().unwrap().is_empty());
    let conn = connect(&paths.database(), true).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        11
    );
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5000
    );
    assert_eq!(
        conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    conn.execute_batch(&FIXTURE.replace(
        "INSERT INTO projects VALUES",
        "INSERT INTO projects(id,name,created_at,updated_at,deleted_at) VALUES",
    ))
    .unwrap();
    for sql in [
        "UPDATE resource_bindings SET target_id=randomblob(16)",
        "INSERT INTO resource_bindings(id,target_id,alias,kind,host,port,tls_mode,credential_ref,policy_labels,status) SELECT randomblob(16),target_id,alias,kind,host,port,tls_mode,credential_ref,policy_labels,status FROM resource_bindings",
        "UPDATE resource_bindings SET alias='../../outside'",
        "UPDATE projects SET id=X'01'",
        "UPDATE projects SET name=hex(zeroblob(100))",
        "UPDATE projects SET updated_at=-1",
    ] {
        let error = conn.execute_batch(sql).unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ffi::ErrorCode::ConstraintViolation),
            "{sql}"
        );
    }
    assert_eq!(store.active_projects().unwrap()[0].name, "orders");
    conn.execute_batch("UPDATE projects SET deleted_at=updated_at")
        .unwrap();
    assert!(store.active_projects().unwrap().is_empty());
    assert!(backups(&paths).is_empty());
}

#[test]
fn backup_includes_committed_wal_and_restores_into_fresh_destination() {
    let (_dir, paths) = isolated();
    let old = previous(&paths);
    old.execute(
        "UPDATE resource_bindings SET credential_ref='fixture-ref-in-wal'",
        [],
    )
    .unwrap();
    assert!(
        fs::metadata(paths.database().with_extension("db-wal"))
            .unwrap()
            .len()
            > 0
    );
    let store = Store::open(paths.clone()).unwrap();
    let rows = store.active_projects().unwrap();
    let files = backups(&paths);
    assert_eq!(files.len(), 1);
    let backup = connect(&files[0], false).unwrap();
    assert_eq!(schema::version(&backup).unwrap(), 1);
    assert_eq!(
        backup
            .query_row("SELECT credential_ref FROM resource_bindings", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "fixture-ref-in-wal"
    );
    drop(backup);
    let (_new, dest) = isolated();
    let restored = Store::restore_into(&files[0], dest.clone()).unwrap();
    assert_eq!(restored.active_projects().unwrap(), rows);
    assert_eq!(
        connect(&dest.database(), false)
            .unwrap()
            .query_row("SELECT credential_ref FROM resource_bindings", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "fixture-ref-in-wal"
    );
    assert!(matches!(
        Store::restore_into(&files[0], dest),
        Err(Error::DestinationExists)
    ));
    assert_eq!(backups(&paths).len(), 1);
}

#[test]
fn newer_or_unrecognized_database_refuses_old_entry_without_modification() {
    let (_dir, paths) = isolated();
    let handle = Store::open(paths.clone()).unwrap();
    let db = connect(&paths.database(), true).unwrap();
    db.pragma_update(None, "user_version", schema::CURRENT + 1)
        .unwrap();
    let wal = paths.database().with_extension("db-wal");
    let image = fs::read(paths.database()).unwrap();
    let journal = fs::read(&wal).unwrap();
    assert!(matches!(
        Store::open(paths.clone()),
        Err(Error::UpgradeRequired)
    ));
    assert_eq!(
        handle.active_projects().unwrap_err(),
        Error::UpgradeRequired
    );
    let id = BindingId::generate();
    assert!(matches!(
        handle.binding_snapshot(id),
        Err(Error::UpgradeRequired)
    ));
    assert_eq!(fs::read(paths.database()).unwrap(), image);
    assert_eq!(fs::read(wal).unwrap(), journal);
    assert!(backups(&paths).is_empty());
    drop(db);
    let (_dir, unknown) = isolated();
    unknown.prepare().unwrap();
    paths::create_file(&unknown.database()).unwrap();
    connect(&unknown.database(), true)
        .unwrap()
        .execute_batch("CREATE TABLE sqliteXcatalog(x)")
        .unwrap();
    let before = fs::read(unknown.database()).unwrap();
    assert!(matches!(
        Store::open(unknown.clone()),
        Err(Error::InvalidDatabase)
    ));
    assert_eq!(fs::read(unknown.database()).unwrap(), before);
}

#[test]
fn sqlite_repository_loads_only_the_registered_binding_in_one_snapshot() {
    let (_dir, paths) = isolated();
    let db = previous(&paths);
    db.execute_batch("INSERT INTO toolchain_profiles VALUES(X'55555555555545558555555555555555',X'11111111111141118111111111111111','default','[]',1700000000000);
        INSERT INTO workspace_bindings VALUES(X'44444444444444448444444444444444',X'11111111111141118111111111111111','main','/fixture/orders',NULL,X'55555555555545558555555555555555','TRUSTED',1700000000000,NULL);
        INSERT INTO capability_grants VALUES(randomblob(16),X'11111111111141118111111111111111',X'44444444444444448444444444444444',X'22222222222242228222222222222222',X'33333333333343338333333333333333',NULL,'database.schema',NULL,NULL);
        INSERT INTO task_definitions VALUES(randomblob(16),X'11111111111141118111111111111111',NULL,'maven-test','mvn','[\"test\"]','.',600,'CLOSED',1);
        INSERT INTO projects VALUES(randomblob(16),'other',1700000000000,1700000000000,NULL);").unwrap();
    let store = Store::open(paths).unwrap();
    let id = BindingId(BINDING.parse().unwrap());
    let state = store.load_binding(id).unwrap().unwrap();
    assert_eq!(state.project.name, "orders");
    assert_eq!(state.binding.id, id);
    assert_eq!(state.targets.len(), 1);
    assert_eq!(
        state.resources[0].credential_ref.as_ref().unwrap().0,
        "fixture-ref-1"
    );
    assert_eq!(state.tasks[0].arguments, ["test"]);
    assert_eq!(state.profiles[0].name.as_str(), "default");
    assert!(
        state.grants[0]
            .capability
            .wire_name()
            .starts_with("database.schema:33333333")
    );
    assert!(store.load_binding(BindingId::generate()).unwrap().is_none());
}

#[test]
fn migration_failure_rolls_back_and_outputs_no_secret() {
    let (_dir, paths) = isolated();
    let mut db = previous(&paths);
    // A memory-only trigger emits the canary in a genuine SQLite migration error.
    db.execute_batch(&format!("CREATE TEMP TRIGGER fail_schema_version BEFORE INSERT ON main.schema_migrations WHEN NEW.version=2 BEGIN SELECT raise(ABORT, '{CANARY}'); END")).unwrap();
    let raw = db
        .execute("INSERT INTO schema_migrations VALUES(2,0)", [])
        .unwrap_err();
    assert!(raw.to_string().contains(CANARY));
    let rejected = db
        .execute(
            "UPDATE resource_bindings SET host=?1",
            [format!("mysql://user:{CANARY}@fixture.invalid")],
        )
        .unwrap_err();
    assert_eq!(
        rejected.sqlite_error_code(),
        Some(rusqlite::ffi::ErrorCode::ConstraintViolation)
    );
    let error = schema::migrate_connection(&paths, &mut db).unwrap_err();
    assert_eq!(error, Error::MigrationFailed);
    assert_eq!(schema::version(&db).unwrap(), 1);
    assert!(db.prepare("SELECT description FROM projects").is_err());
    assert!(!error.message().contains(CANARY));
    let mut files = backups(&paths);
    assert_eq!(files.len(), 1);
    files.extend([paths.database(), paths.database().with_extension("db-wal")]);
    for file in files {
        let bytes = fs::read(file).unwrap();
        assert!(!bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()));
    }
}

#[test]
fn native_user_permissions_cover_directories_database_wal_and_backup() {
    let (_dir, paths) = isolated();
    let _writer = previous(&paths);
    Store::open(paths.clone()).unwrap();
    let mut files = backups(&paths);
    assert_eq!(files.len(), 1);
    files.extend([
        paths.database(),
        paths.database().with_extension("db-wal"),
        paths.database().with_extension("db-shm"),
        paths.root().join("data/migration.lock"),
    ]);
    let dirs = ["", "data", "tmp", "logs", "backups"].map(|p| paths.root().join(p));
    for file in &files {
        assert!(file.is_file());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in files {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        for path in dirs {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
    #[cfg(windows)]
    {
        let status=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",r#"
            $ErrorActionPreference='Stop'
            $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User
            $entries=@(Get-Item -LiteralPath $env:TEST_ACL_ROOT) + @(Get-ChildItem -LiteralPath $env:TEST_ACL_ROOT -Recurse -Force)
            foreach ($entry in $entries) {
                $acl=Get-Acl -LiteralPath $entry.FullName
                $rules=$acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])
                if ($rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid.Value -or
                    $rules[0].AccessControlType -ne 'Allow' -or $rules[0].FileSystemRights -ne 'FullControl') { exit 1 }
            }
            Write-Output 'Current-user-only DACL verified on directories, DB, WAL, SHM and backup.'
        "#]).env("TEST_ACL_ROOT",paths.root()).status().unwrap();
        assert!(status.success());
        assert!(dirs.iter().all(|p| p.is_dir()));
    }
}

pub(super) fn pause_uncommitted_migration(paths: &StorePaths) {
    if std::env::var_os("T03_KILL_BEFORE_COMMIT").as_deref() == Some(paths.root().as_os_str()) {
        fs::write(paths.root().join("uncommitted"), b"ready").unwrap();
        loop {
            thread::park();
        }
    }
}

#[test]
fn storage_process_worker() {
    let Some(root) = std::env::var_os("T03_WORKER_ROOT") else {
        return;
    };
    let result = Store::open(StorePaths::at(root.into()).unwrap());
    if std::env::var_os("T03_EXPECT_BUSY").is_some() {
        assert!(matches!(result, Err(Error::StorageBusy)));
    } else {
        assert_eq!(result.unwrap().active_projects().unwrap()[0].name, "orders");
    }
}
fn worker(paths: &StorePaths, extra: Option<(&str, &std::ffi::OsStr)>) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "tests::storage_process_worker"])
        .env("T03_WORKER_ROOT", paths.root());
    if let Some((key, value)) = extra {
        cmd.env(key, value);
    }
    cmd.spawn().unwrap()
}

#[test]
fn independent_processes_contend_then_upgrade_once() {
    let (_dir, paths) = isolated();
    let db = previous(&paths);
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(
        worker(&paths, Some(("T03_EXPECT_BUSY", "1".as_ref())))
            .wait()
            .unwrap()
            .success()
    );
    db.execute_batch("COMMIT").unwrap();
    let mut one = worker(&paths, None);
    let mut two = worker(&paths, None);
    assert!(one.wait().unwrap().success());
    assert!(two.wait().unwrap().success());
    assert_eq!(backups(&paths).len(), 1);
}

#[test]
fn killed_migrator_leaves_committed_version_and_retry_succeeds() {
    let (_dir, paths) = isolated();
    drop(previous(&paths));
    let mut child = worker(
        &paths,
        Some(("T03_KILL_BEFORE_COMMIT", paths.root().as_os_str())),
    );
    let start = Instant::now();
    while !paths.root().join("uncommitted").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "worker exited before transaction barrier"
        );
        if start.elapsed() > Duration::from_secs(40) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("migration barrier timed out");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let conn = connect(&paths.database(), false).unwrap();
    assert_eq!(schema::version(&conn).unwrap(), 1);
    assert!(conn.prepare("SELECT description FROM projects").is_err());
    drop(conn);
    assert!(worker(&paths, None).wait().unwrap().success());
    assert_eq!(
        schema::version(&connect(&paths.database(), false).unwrap()).unwrap(),
        schema::CURRENT
    );
}

#[test]
fn restoring_current_schema_enables_wal_on_the_recovered_database() {
    let (_dir, source) = isolated();
    Store::open(source.clone()).unwrap();
    let snapshot_path = source.backups().join("current.db");
    schema::snapshot(
        &connect(&source.database(), false).unwrap(),
        &snapshot_path,
        schema::CURRENT,
    )
    .unwrap();
    let (_dir, destination) = isolated();
    Store::restore_into(&snapshot_path, destination.clone()).unwrap();
    let mode = connect(&destination.database(), false)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn missing_database_with_orphan_journal_refuses_initialization() {
    let (_dir, paths) = isolated();
    paths.prepare().unwrap();
    let wal = paths.database().with_extension("db-wal");
    paths::create_file(&wal).unwrap();
    fs::write(&wal, b"orphan").unwrap();
    assert!(matches!(
        Store::open(paths.clone()),
        Err(Error::InvalidDatabase)
    ));
    assert_eq!(fs::read(wal).unwrap(), b"orphan");
    assert!(!paths.database().exists());
}

#[test]
fn migration_preserves_existing_evidence_and_its_binding() {
    let (_dir, paths) = isolated();
    let db = previous(&paths);
    let binding = BindingId(BINDING.parse().unwrap());
    let request = envlume_domain::RequestId::generate();
    let evidence = envlume_domain::EvidenceId::generate();
    db.execute(
        "INSERT INTO workspace_bindings(id,project_id,alias,canonical_path,trust_state,created_at)
         SELECT ?1,id,'old-binding','fixture','TRUSTED',1700000000000 FROM projects",
        [binding.0],
    )
    .unwrap();
    db.execute("INSERT INTO execution_records(id,request_id,evidence_id,project_id,binding_id,capability,
        actor,decision,sanitized_summary,exit_code,duration_ms,created_at)
        SELECT randomblob(16),?1,?2,project_id,id,'task.run','CLI','FAILED','Build failed',1,18,1700000000000
        FROM workspace_bindings WHERE id=?3", rusqlite::params![request.0,evidence.0,binding.0]).unwrap();
    drop(db);
    Store::open(paths.clone()).unwrap();
    let db = connect(&paths.database(), false).unwrap();
    let preserved:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM execution_records WHERE binding_id=?1
        AND request_id=?2 AND evidence_id=?3 AND exit_code=1 AND decision='FAILED' AND actor='CLI' AND details_json IS NULL)",
        rusqlite::params![binding.0,request.0,evidence.0],|r|r.get(0)).unwrap();
    assert!(preserved);
}

#[test]
fn legacy_resources_keep_metadata_and_references_but_require_new_approval() {
    let (_temp, paths) = isolated();
    let db = previous(&paths);
    let binding = BindingId(BINDING.parse().unwrap());
    db.execute(
        "INSERT INTO workspace_bindings(id,project_id,alias,canonical_path,trust_state,created_at)
                SELECT ?1,id,'fixture','fixture','TRUSTED',1 FROM projects",
        [binding.0],
    )
    .unwrap();
    db.execute(
        "INSERT INTO capability_grants(id,project_id,binding_id,target_id,resource_id,capability)
                SELECT randomblob(16),t.project_id,?1,t.id,r.id,'database.read'
                FROM resource_bindings r JOIN environment_targets t ON r.target_id=t.id",
        [binding.0],
    )
    .unwrap();
    drop(db);
    Store::open(paths.clone()).unwrap();
    let db = connect(&paths.database(), false).unwrap();
    let intact = db
        .query_row(
            "SELECT host,credential_ref,confirmed_at_ms IS NULL FROM resource_bindings",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        intact,
        ("fixture.invalid".into(), "fixture-ref-1".into(), true)
    );
    assert!(
        db.query_row(
            "SELECT revoked_at IS NOT NULL FROM capability_grants WHERE capability='database.read'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[cfg(windows)]
#[test]
fn existing_private_catalogue_needs_no_shell_and_rejects_an_additional_sid() {
    let (_tmp, paths) = isolated();
    drop(previous(&paths));
    Store::open(paths.clone()).unwrap();
    // A fresh process must apply the same DACL policy with no PowerShell available.
    let mut proc = worker(
        &paths,
        Some(("SystemRoot", std::ffi::OsStr::new(r"Z:\absent-fixture"))),
    );
    assert!(proc.wait().unwrap().success());
    let dir = paths.root().join("logs");
    let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let status = Command::new(cmd)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            r#"
$ErrorActionPreference='Stop'
$d=[IO.DirectoryInfo]::new($env:T15_TEST_ACL_DIR)
$a=$d.GetAccessControl()
$s=[Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$a.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($s,'Read','Allow'))
$d.SetAccessControl($a)
"#,
        ])
        .env("T15_TEST_ACL_DIR", dir)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(matches!(
        Store::open(paths),
        Err(Error::PrivateDirectoryRequired)
    ));
}
