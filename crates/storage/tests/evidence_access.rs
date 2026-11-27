use envlume_application::{
    result::Report,
    workspaces::{ManagementError, add_project},
};
use envlume_audit::Entry;
use envlume_domain::{Actor, EvidenceId, GrantId};
use envlume_storage::{RecordKey, Store, StorePaths};
use rusqlite::params;

#[test]
fn history_requires_this_bindings_live_project_inspect_grant() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let paths = StorePaths::at(tmp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let project = add_project(&store, &root, None, |_, _| true).unwrap();
    let binding = project.binding_id;
    let sibling = tmp.path().join("sibling");
    std::fs::create_dir(&sibling).unwrap();
    add_project(&store, &sibling, Some(project.project_id), |_, _| true).unwrap();
    let evidence = EvidenceId::generate();
    let mut report = Report::new("database_query");
    report.evidence_id = Some(evidence);
    store
        .record(
            &store.evidence_scope(binding, &root).unwrap(),
            Actor::Cli,
            Entry {
                report: &report,
                template: None,
                outcome: None,
            },
        )
        .unwrap();
    let lookup = || store.lookup_record(binding, &root, RecordKey::Request(report.request_id));
    assert!(lookup().unwrap().is_some());
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    // Another local capability and the sibling's project.inspect stay live.
    let insert_grant = "INSERT INTO capability_grants(id, project_id, binding_id, capability)
                        VALUES(?1, ?2, ?3, ?4)";
    db.execute(
        insert_grant,
        params![
            GrantId::generate().0,
            project.project_id.0,
            binding.0,
            "task.inspect"
        ],
    )
    .unwrap();
    for (expires, revoked) in [(Some(0), None), (None, Some(1))] {
        db.execute(
            "UPDATE capability_grants SET expires_at=?2, revoked_at=?3
             WHERE binding_id=?1 AND capability='project.inspect'",
            params![binding.0, expires, revoked],
        )
        .unwrap();
        assert_eq!(lookup().unwrap_err(), ManagementError::ConfirmationRequired);
        assert_eq!(
            store
                .lookup_record(binding, &root, RecordKey::Evidence(evidence))
                .unwrap_err(),
            ManagementError::ConfirmationRequired
        );
        assert_eq!(
            store.clear_records(binding, &root).unwrap_err(),
            ManagementError::ConfirmationRequired
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM execution_records", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    db.execute(
        insert_grant,
        params![
            GrantId::generate().0,
            project.project_id.0,
            binding.0,
            "project.inspect"
        ],
    )
    .unwrap();
    assert!(lookup().unwrap().is_some());
    assert_eq!(store.clear_records(binding, &root).unwrap(), 1);
}
