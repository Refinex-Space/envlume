use envlume_application::{
    ErrorCode, Session,
    environments::{Edit, Endpoint, RemoteAction, manage},
    workspaces::add_project,
};
use envlume_domain::*;
use envlume_storage::{LocalRegistration, Store, StorePaths};
use serde_json::{Value, json};
use std::path::Path;
fn alias(s: &str) -> Alias {
    s.to_owned().try_into().unwrap()
}
fn set_target(kind: EnvironmentKind, risk: RiskLevel) -> Edit {
    Edit::Target {
        alias: alias("blue"),
        kind,
        risk,
    }
}
fn resource(host: &str) -> Edit {
    Edit::Resource {
        target: alias("blue"),
        alias: alias("db"),
        endpoint: Endpoint {
            host: host.into(),
            port: 3306,
            tls: TlsMode::VerifyIdentity,
            database: Some("orders".into()),
        },
    }
}
fn grant() -> Edit {
    Edit::Grant {
        target: alias("blue"),
        alias: alias("db"),
        action: RemoteAction::Schema,
        expires_at: None,
    }
}
fn edit(store: &Store, id: BindingId, root: &Path, change: Edit) -> Result<(), ErrorCode> {
    manage(
        &LocalRegistration {
            store: store.clone(),
            origin: root.into(),
        },
        store,
        id,
        root,
        change,
        |_| true,
    )
}
#[test]
fn endpoint_and_risk_edits_retire_grants_on_every_binding_and_reject_stale_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    for root in [&a, &b] {
        std::fs::create_dir(root).unwrap();
    }
    let binding = add_project(&store, &a, None, |_, _| true).unwrap();
    let first = binding.binding_id;
    let second = add_project(&store, &b, Some(binding.project_id), |_, _| true)
        .unwrap()
        .binding_id;
    let view = |id, root: &Path| {
        let session = Session::from_registration(
            LocalRegistration {
                store: store.clone(),
                origin: root.into(),
            },
            id,
            Actor::Cli,
        )
        .unwrap();
        serde_json::to_value(session.environment_list(json!({}))).unwrap()["result"]["environments"]
            .clone()
    };
    let empty = json!([]);
    let visible = json!([{"target":"blue","resources":[{"alias":"db","health":"unknown"}]}]);
    edit(
        &store,
        first,
        &a,
        set_target(EnvironmentKind::Dev, RiskLevel::Low),
    )
    .unwrap();
    edit(&store, first, &a, resource("one.invalid")).unwrap();
    edit(&store, first, &a, grant()).unwrap();
    assert_eq!(view(second, &b), Value::Null);
    edit(&store, second, &b, grant()).unwrap();
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    db.execute(
        "UPDATE resource_bindings SET credential_ref='test-reference'",
        [],
    )
    .unwrap();
    edit(&store, first, &a, resource("one.invalid")).unwrap();
    assert_eq!(view(first, &a), visible);
    assert_eq!(
        db.query_row("SELECT credential_ref FROM resource_bindings", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "test-reference"
    );
    let repo = LocalRegistration {
        store: store.clone(),
        origin: a.clone(),
    };
    let stale = manage(&repo, &store, first, &a, grant(), |preview| {
        assert!(preview.contains("one.invalid"));
        edit(&store, second, &b, resource("two.invalid")).unwrap();
        true
    });
    assert_eq!(stale, Err(ErrorCode::PolicyDenied));
    assert_eq!(view(first, &a), empty);
    assert_eq!(view(second, &b), empty);
    assert!(
        db.query_row(
            "SELECT credential_ref IS NULL AND confirmed_at_ms IS NOT NULL FROM resource_bindings",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    for (id, root) in [(first, &a), (second, &b)] {
        edit(&store, id, root, grant()).unwrap();
    }
    edit(
        &store,
        first,
        &a,
        set_target(EnvironmentKind::Dev, RiskLevel::Medium),
    )
    .unwrap();
    assert_eq!(view(first, &a), empty);
    assert_eq!(view(second, &b), empty);
    edit(&store, first, &a, grant()).unwrap();
    assert_eq!(view(first, &a), visible);
    edit(
        &store,
        first,
        &a,
        set_target(EnvironmentKind::Prod, RiskLevel::Critical),
    )
    .unwrap();
    edit(
        &store,
        first,
        &a,
        Edit::Rename {
            alias: alias("blue"),
            to: alias("dev"),
        },
    )
    .unwrap();
    let restore = Edit::Target {
        alias: alias("dev"),
        kind: EnvironmentKind::Dev,
        risk: RiskLevel::Low,
    };
    assert_eq!(
        edit(&store, first, &a, restore),
        Err(ErrorCode::PolicyDenied)
    );
    for action in [
        RemoteAction::Schema,
        RemoteAction::Connect,
        RemoteAction::Read,
    ] {
        assert_eq!(
            edit(
                &store,
                first,
                &a,
                Edit::Grant {
                    target: alias("dev"),
                    alias: alias("db"),
                    action,
                    expires_at: None
                }
            ),
            Err(ErrorCode::TargetNotAllowed)
        );
    }
    assert_eq!(view(first, &a), empty);
    assert_eq!(
        db.query_row("SELECT is_default FROM environment_targets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn finite_grants_do_not_issue_unlimited_inspection_and_endpoint_components_invalidate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let paths = StorePaths::at(tmp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let id = add_project(&store, &root, None, |_, _| true)
        .unwrap()
        .binding_id;
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    edit(
        &store,
        id,
        &root,
        set_target(EnvironmentKind::Test, RiskLevel::Medium),
    )
    .unwrap();
    edit(&store, id, &root, resource("fixture.invalid")).unwrap();
    let expiration = UtcDateTime::now() + std::time::Duration::from_secs(3600);
    let allow = || Edit::Grant {
        target: alias("blue"),
        alias: alias("db"),
        action: RemoteAction::Schema,
        expires_at: Some(expiration),
    };
    for (port, db_name, tls) in [
        (3307, "orders", TlsMode::VerifyIdentity),
        (3307, "other", TlsMode::VerifyIdentity),
        (3307, "other", TlsMode::Disabled),
    ] {
        edit(&store, id, &root, allow()).unwrap();
        let deadlines: Vec<i64>=db.prepare("SELECT expires_at FROM capability_grants WHERE binding_id=?1 AND capability IN ('environment.inspect','database.schema') AND revoked_at IS NULL")
            .unwrap().query_map([id.0],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(
            deadlines,
            vec![(expiration.unix_timestamp_nanos() / 1_000_000) as i64; 2]
        );
        db.execute(
            "UPDATE resource_bindings SET credential_ref='fixture-ref'",
            [],
        )
        .unwrap();
        edit(
            &store,
            id,
            &root,
            Edit::Resource {
                target: alias("blue"),
                alias: alias("db"),
                endpoint: Endpoint {
                    host: "fixture.invalid".into(),
                    port,
                    database: Some(db_name.into()),
                    tls,
                },
            },
        )
        .unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM capability_grants WHERE resource_id IS NOT NULL AND revoked_at IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(
            db.query_row(
                "SELECT credential_ref IS NULL FROM resource_bindings",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
    }
    edit(&store, id, &root, grant()).unwrap();
    assert!(db.query_row("SELECT expires_at IS NULL FROM capability_grants WHERE capability='environment.inspect' AND revoked_at IS NULL",[],|r|r.get::<_,bool>(0)).unwrap());
}

#[test]
fn mysql_table_edits_revoke_grants_and_route_edits_detach_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let paths = StorePaths::at(tmp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let id = add_project(&store, &root, None, |_, _| true)
        .unwrap()
        .binding_id;
    edit(
        &store,
        id,
        &root,
        set_target(EnvironmentKind::Dev, RiskLevel::Low),
    )
    .unwrap();
    edit(&store, id, &root, resource("fixture.invalid")).unwrap();
    let policy = |ip: &str, col: &str| Edit::MySqlPolicy {
        target: alias("blue"),
        alias: alias("db"),
        route: MySqlRoute {
            ips: [ip.parse().unwrap()].into(),
            ca_pem: None,
        },
        tables: [(
            "orders".into(),
            MySqlTable {
                columns: [col.into()].into(),
                indexes: Default::default(),
            },
        )]
        .into(),
    };
    edit(&store, id, &root, policy("127.0.0.1", "id")).unwrap();
    edit(&store, id, &root, grant()).unwrap();
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    db.execute(
        "UPDATE resource_bindings SET credential_ref='fixture-ref'",
        [],
    )
    .unwrap();
    let live = || {
        db.query_row("SELECT count(*) FROM capability_grants WHERE resource_id IS NOT NULL AND revoked_at IS NULL",[],|r|r.get::<_,i64>(0)).unwrap()
    };
    edit(&store, id, &root, policy("127.0.0.1", "id")).unwrap();
    assert_eq!(live(), 1);
    edit(&store, id, &root, policy("127.0.0.1", "name")).unwrap();
    assert_eq!(live(), 0);
    assert_eq!(
        db.query_row("SELECT credential_ref FROM resource_bindings", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "fixture-ref"
    );
    edit(&store, id, &root, grant()).unwrap();
    edit(&store, id, &root, policy("127.0.0.2", "name")).unwrap();
    assert_eq!(live(), 0);
    assert!(
        db.query_row(
            "SELECT credential_ref IS NULL FROM resource_bindings",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    edit(&store, id, &root, resource("other.invalid")).unwrap();
    assert!(
        db.query_row(
            "SELECT mysql_route IS NULL AND mysql_tables='{}' FROM resource_bindings",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn query_policy_requires_fresh_read_grants_on_all_bindings() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(tmp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    for root in [&first, &second] {
        std::fs::create_dir(root).unwrap();
    }
    let a = add_project(&store, &first, None, |_, _| true).unwrap();
    let b = add_project(&store, &second, Some(a.project_id), |_, _| true).unwrap();
    let set = |change| edit(&store, a.binding_id, &first, change).unwrap();
    set(set_target(EnvironmentKind::Dev, RiskLevel::Low));
    set(resource("fixture.invalid"));
    set(Edit::MySqlPolicy {
        target: alias("blue"),
        alias: alias("db"),
        route: MySqlRoute {
            ips: ["127.0.0.1".parse().unwrap()].into(),
            ca_pem: None,
        },
        tables: [(
            "orders".into(),
            MySqlTable {
                columns: ["id".into()].into(),
                indexes: Default::default(),
            },
        )]
        .into(),
    });
    let read = || Edit::Grant {
        target: alias("blue"),
        alias: alias("db"),
        action: RemoteAction::Read,
        expires_at: None,
    };
    let approve = || {
        set(read());
        edit(&store, b.binding_id, &second, read()).unwrap();
    };
    let db = rusqlite::Connection::open(paths.database()).unwrap();
    let live = |name: &str| {
        db.query_row(
            "SELECT count(*) FROM capability_grants WHERE capability=?1 AND revoked_at IS NULL",
            [name],
            |r| r.get::<_, i32>(0),
        )
        .unwrap()
    };
    let policy = |max_rows| Edit::QueryPolicy {
        target: alias("blue"),
        alias: alias("db"),
        policy: QueryPolicy {
            tables: [(
                "orders".into(),
                QueryColumns {
                    readable: ["id".into()].into(),
                    masked: Default::default(),
                },
            )]
            .into(),
            max_rows,
            max_bytes: 8192,
            timeout_ms: 1000,
        },
    };
    set(grant());
    approve();
    assert_eq!(live("database.read"), 2);
    set(policy(10));
    assert_eq!(live("database.read"), 0);
    approve();
    set(policy(10));
    assert_eq!(live("database.read"), 2);
    set(policy(3));
    assert_eq!(live("database.read"), 0);
    assert_eq!(live("database.schema"), 1);
    set(resource("changed.invalid"));
    assert!(
        db.query_row(
            "SELECT mysql_query_policy IS NULL FROM resource_bindings",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}
