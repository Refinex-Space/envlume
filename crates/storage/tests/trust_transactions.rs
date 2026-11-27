use envlume_application::{
    ErrorCode,
    ports::Repository,
    workspaces::{ManagementError, WorkspaceRepository, add_project, inspect_project},
};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use rusqlite::Connection;
use std::{fs, path::PathBuf};
fn fixture() -> (tempfile::TempDir, StorePaths, Store, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(tmp.path().join("store")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let path = tmp.path().join("orders");
    fs::create_dir(&path).unwrap();
    (tmp, paths, store, path)
}

#[test]
fn changes_during_confirmation_and_cancel_leave_no_authority() {
    let (tmp, paths, store, path) = fixture();
    assert_eq!(
        add_project(&store, &path, None, |_, _| false).unwrap_err(),
        ManagementError::ConfirmationRequired
    );
    let changed = add_project(&store, &path, None, |_, _| {
        fs::rename(&path, tmp.path().join("saved")).unwrap();
        fs::create_dir(&path).unwrap();
        true
    });
    assert_eq!(changed.unwrap_err(), ManagementError::WorkspaceChanged);
    let db = Connection::open(paths.database()).unwrap();
    for table in ["projects", "workspace_bindings", "capability_grants"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn replacing_directory_at_same_name_invalidates_id_and_all_grants() {
    let (tmp, paths, store, path) = fixture();
    let old = add_project(&store, &path, None, |_, _| true).unwrap();
    fs::rename(&path, tmp.path().join("original")).unwrap();
    fs::create_dir(&path).unwrap();
    let repo = LocalRegistration {
        store: Store::open(paths.clone()).unwrap(),
        origin: path.clone(),
    };
    assert!(matches!(
        repo.load_binding(old.binding_id),
        Err(ErrorCode::ProjectNotTrusted)
    ));
    assert_eq!(
        inspect_project(&store, &path).unwrap().binding.state,
        "STALE"
    );
    let new = add_project(&store, &path, Some(old.project_id), |_, _| true).unwrap();
    assert_ne!(new.binding_id, old.binding_id);
    let db = Connection::open(paths.database()).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM capability_grants WHERE binding_id=?1 AND revoked_at IS NULL",
            [old.binding_id.0],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM workspace_bindings WHERE confirmation_source='LOCAL_CLI_TTY' AND confirmed_at_ms IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),2);
}

#[test]
fn failed_revocation_and_soft_delete_roll_back_as_a_unit() {
    let (_tmp, paths, store, path) = fixture();
    let old = add_project(&store, &path, None, |_, _| true).unwrap();
    let db = Connection::open(paths.database()).unwrap();
    db.execute_batch("CREATE TRIGGER fault BEFORE UPDATE OF revoked_at ON capability_grants BEGIN SELECT raise(ABORT,'test rollback'); END;").unwrap();
    assert!(store.revoke(old.binding_id).is_err());
    assert!(store.delete_project(old.project_id).is_err());
    assert_eq!(
        inspect_project(&store, &path).unwrap().binding.state,
        "TRUSTED"
    );
    assert!(
        db.query_row(
            "SELECT deleted_at IS NULL FROM projects WHERE id=?1",
            [old.project_id.0],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    db.execute_batch("DROP TRIGGER fault").unwrap();
    store.delete_project(old.project_id).unwrap();
    store.delete_project(old.project_id).unwrap();
    assert_eq!(
        inspect_project(&store, &path).unwrap().binding.state,
        "REVOKED"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM capability_grants WHERE revoked_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn unverified_legacy_rows_and_untrusted_marker_files_cannot_grant_access() {
    let (_tmp, paths, store, path) = fixture();
    let old = add_project(&store, &path, None, |_, _| true).unwrap();
    let db = Connection::open(paths.database()).unwrap();
    db.execute(
        "UPDATE workspace_bindings SET confirmation_source=NULL, confirmed_at_ms=NULL WHERE id=?1",
        [old.binding_id.0],
    )
    .unwrap();
    fs::write(
        path.join("package.json"),
        r#"{"bindingId":"ignored","trust":true}"#,
    )
    .unwrap();
    let repo = LocalRegistration {
        store: Store::open(paths).unwrap(),
        origin: path.clone(),
    };
    assert!(matches!(
        repo.load_binding(old.binding_id),
        Err(ErrorCode::ProjectNotTrusted)
    ));
    assert_eq!(
        inspect_project(&store, &path).unwrap().binding.state,
        "STALE"
    );
}

#[test]
fn real_context_filters_the_current_grants_and_private_resource_metadata() {
    use envlume_application::Session;
    use envlume_domain::Actor;
    use serde_json::json;
    let (_tmp, paths, store, path) = fixture();
    let bound = add_project(&store, &path, None, |_, _| true).unwrap();
    let db = Connection::open(paths.database()).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for (index, alias, kind, status) in [
        (1, "dev", "DEV", "ACTIVE"),
        (2, "test", "TEST", "ACTIVE"),
        (3, "live", "PROD", "ACTIVE"),
        (4, "off", "DEV", "DISABLED"),
    ] {
        let target: [u8; 16] = [index; 16];
        let resource = [index + 10; 16];
        db.execute(
            "INSERT INTO environment_targets VALUES(?1,?2,?3,?4,'MEDIUM',0,?5)",
            rusqlite::params![target, bound.project_id.0, alias, kind, status],
        )
        .unwrap();
        db.execute("INSERT INTO resource_bindings(id,target_id,alias,kind,host,port,tls_mode,credential_ref,policy_labels,status,confirmed_at_ms) VALUES(?1,?2,'primary-db','MYSQL','internal.invalid',3306,
            'VERIFY_IDENTITY','private-credential-ref','[]','ACTIVE',1)",rusqlite::params![resource,target]).unwrap();
        if alias != "test" {
            db.execute("INSERT INTO capability_grants(id,project_id,binding_id,target_id,resource_id,capability)
                VALUES(randomblob(16),?1,?2,?3,?4,'database.schema')",
                rusqlite::params![bound.project_id.0,bound.binding_id.0,target,resource]).unwrap();
        }
    }
    let session = Session::from_registration(
        LocalRegistration {
            store,
            origin: path,
        },
        bound.binding_id,
        Actor::LocalStdio,
    )
    .unwrap();
    let report = serde_json::to_value(session.project_context(json!({}))).unwrap();
    assert_eq!(report["result"]["targets"], json!(["dev"]));
    assert_eq!(
        report["result"]["capabilities"],
        json!([
            "database.schema:0b0b0b0b-0b0b-0b0b-0b0b-0b0b0b0b0b0b",
            "project.inspect"
        ])
    );
    assert!(!report.to_string().contains("internal.invalid"));
    assert!(!report.to_string().contains("private-credential-ref"));
    db.execute(
        "UPDATE capability_grants SET expires_at=0 WHERE capability='database.schema'",
        [],
    )
    .unwrap();
    let after = serde_json::to_value(session.project_context(json!({}))).unwrap();
    assert_eq!(after["result"]["targets"], json!([]));
    assert_eq!(after["result"]["capabilities"], json!(["project.inspect"]));
}

#[test]
fn repeated_import_of_unchanged_binding_does_not_prompt_or_regrant_access() {
    let (_tmp, paths, store, path) = fixture();
    let registered = add_project(&store, &path, None, |_, _| true).unwrap();
    let db = Connection::open(paths.database()).unwrap();
    db.execute(
        "UPDATE capability_grants SET revoked_at=0 WHERE binding_id=?1",
        [registered.binding_id.0],
    )
    .unwrap();
    let again = add_project(&store, &path, Some(registered.project_id), |_, _| {
        panic!("unchanged registration prompted again")
    })
    .unwrap();
    assert_eq!(again.binding_id, registered.binding_id);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM capability_grants WHERE revoked_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
