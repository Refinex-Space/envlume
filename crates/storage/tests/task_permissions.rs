use envlume_application::{
    ErrorCode, Session,
    tasks::Template,
    workspaces::{WorkspaceRepository, add_project},
};
use envlume_domain::{Actor, Alias, BindingId};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use std::fs;

#[test]
fn task_permission_and_first_use_are_distinct_and_scoped_to_one_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(tmp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let first = add_project(&store, &a, None, |_, _| true).unwrap();
    let other = add_project(&store, &b, Some(first.project_id), |_, _| true).unwrap();
    let profile = Alias::try_from("default".to_owned()).unwrap();
    store
        .configure_toolchain(first.binding_id, &a, &profile, &[])
        .unwrap();
    store
        .configure_toolchain(other.binding_id, &b, &profile, &[])
        .unwrap();
    let registry = |origin| LocalRegistration {
        store: store.clone(),
        origin,
    };
    assert!(matches!(
        Session::from_registration(registry(a.clone()), BindingId::generate(), Actor::Cli),
        Err(ErrorCode::ProjectNotTrusted)
    ));
    let s = Session::from_registration(registry(a.clone()), first.binding_id, Actor::Cli).unwrap();
    let t = Template::NodeBuild;
    assert!(matches!(s.authorize_task(t), Err(ErrorCode::PolicyDenied)));
    assert!(s.list_tasks().is_err());
    store.enable_task(first.binding_id, &a, t, 30).unwrap();
    let grant = s.authorize_task(t).unwrap();
    let fingerprint = "1".repeat(64);
    assert!(!store.task_approved(&grant, &fingerprint, None).unwrap());
    store.approve_task(&grant, &fingerprint, None).unwrap();
    store.enable_task(first.binding_id, &a, t, 30).unwrap();
    assert!(store.task_approved(&grant, &fingerprint, None).unwrap());
    let sibling =
        Session::from_registration(registry(b.clone()), other.binding_id, Actor::Cli).unwrap();
    assert!(matches!(
        sibling.authorize_task(t),
        Err(ErrorCode::PolicyDenied)
    ));
    store.enable_task(other.binding_id, &b, t, 20).unwrap();
    let separate = sibling.authorize_task(t).unwrap();
    assert_eq!(grant.task, separate.task);
    assert_ne!(grant.grant, separate.grant);
    assert_eq!(separate.timeout.as_secs(), 20);
    assert!(!store.task_approved(&separate, &fingerprint, None).unwrap());
    store.enable_task(first.binding_id, &a, t, 31).unwrap();
    assert!(!store.task_approved(&grant, &fingerprint, None).unwrap());
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    db.execute(
        "UPDATE task_definitions SET workspace_relative_dir='../other' WHERE id=?1",
        [grant.task.0],
    )
    .unwrap();
    assert!(matches!(s.authorize_task(t), Err(ErrorCode::PolicyDenied)));
    db.execute("UPDATE task_definitions SET workspace_relative_dir='.',arguments='[\"run\",\"other\"]' WHERE id=?1",[grant.task.0]).unwrap();
    assert!(matches!(s.authorize_task(t), Err(ErrorCode::PolicyDenied)));
    assert!(s.list_tasks().unwrap().is_empty());
    store.revoke(first.binding_id).unwrap();
    assert!(matches!(
        s.authorize_task(t),
        Err(ErrorCode::BindingRevoked)
    ));
}
