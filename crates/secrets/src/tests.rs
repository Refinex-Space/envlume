use super::*;
use envlume_application::{
    Session,
    credentials::{self, Catalogue, Change, Selection},
    environments::{Edit, Endpoint, RemoteAction, manage},
    ports::{AuthorizedResource, ResourceConnector},
    result::ConnectionResult,
    workspaces::add_project,
};
use envlume_domain::*;
use envlume_storage::{LocalRegistration, Store, StorePaths};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    rc::Rc,
};

#[path = "../../../tests/contracts/fixture_cases.rs"]
mod fixture_cases;

const USER: &str = "test-only-user-T10";
const FIRST: &str = "SYNTHETIC_T10_first_测试";
const SECOND: &str = "SYNTHETIC_T10_rotated_测试";
fn alias(s: &str) -> Alias {
    s.to_owned().try_into().unwrap()
}
struct Fixture {
    dir: tempfile::TempDir,
    paths: StorePaths,
    store: Store,
    id: BindingId,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::at(dir.path().join("catalogue")).unwrap();
        let store = Store::open(paths.clone()).unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let id = add_project(&store, &root, None, |_, _| true)
            .unwrap()
            .binding_id;
        let f = Self {
            dir,
            paths,
            store,
            id,
        };
        for target in ["dev", "test"] {
            f.edit(Edit::preset(alias(target)).unwrap());
            for resource in ["db", "shared"] {
                f.edit(Edit::Resource {
                    target: alias(target),
                    alias: alias(resource),
                    endpoint: Endpoint {
                        host: "fixture.invalid".into(),
                        port: 3306,
                        database: Some("fixture".into()),
                        tls: TlsMode::VerifyIdentity,
                    },
                });
            }
        }
        f
    }
    fn registry(&self) -> LocalRegistration {
        LocalRegistration {
            store: self.store.clone(),
            origin: self.dir.path().join("workspace"),
        }
    }
    fn edit(&self, e: Edit) {
        let reg = self.registry();
        manage(&reg, &self.store, self.id, &reg.origin, e, |_| true).unwrap();
    }
    fn selection(&self, target: &str, resource: &str) -> Selection {
        credentials::select(&self.registry(), self.id, &alias(target), &alias(resource)).unwrap()
    }
    fn db(&self) -> Connection {
        Connection::open(self.paths.database()).unwrap()
    }
    fn scan(&self, report: &str) {
        let mut files = vec![
            self.paths.database(),
            self.paths.database().with_extension("db-wal"),
        ];
        files.extend(
            fs::read_dir(self.paths.backups())
                .unwrap()
                .map(|e| e.unwrap().path()),
        );
        for marker in [USER, FIRST, SECOND] {
            assert!(!report.contains(marker), "secret in report");
            for f in files.iter().filter(|p| p.is_file()) {
                assert!(
                    !fs::read(f)
                        .unwrap()
                        .windows(marker.len())
                        .any(|b| b == marker.as_bytes()),
                    "secret in SQLite artifact"
                );
            }
        }
    }
}
#[derive(Default)]
struct Fake {
    values: RefCell<BTreeMap<String, Vec<u8>>>,
    write_fault: Cell<Option<Fault>>,
    delete_fault: Cell<Option<Fault>>,
    read_fault: Cell<Option<Fault>>,
    after_write: RefCell<Option<Box<dyn FnOnce()>>>,
}
impl CredentialWriter for Fake {
    type Input = Login;
    fn write_new(&self, k: &ItemKey, v: &Login) -> Result<(), Fault> {
        self.values.borrow_mut().insert(k.reference(), v.0.to_vec());
        if let Some(hook) = self.after_write.borrow_mut().take() {
            hook();
        }
        self.write_fault.get().map_or(Ok(()), Err)
    }
    fn available(&self, k: &ItemKey) -> Result<(), Fault> {
        if let Some(e) = self.read_fault.get() {
            return Err(e);
        }
        self.values
            .borrow()
            .get(&k.reference())
            .map(|_| ())
            .ok_or(Fault::Missing)
    }
    fn delete(&self, k: &ItemKey) -> Result<(), Fault> {
        if let Some(e) = self.delete_fault.get() {
            return Err(e);
        }
        self.values
            .borrow_mut()
            .remove(&k.reference())
            .map(|_| ())
            .ok_or(Fault::Missing)
    }
}
#[test]
fn interrupted_publication_shared_refs_and_cleanup_retry() {
    let f = Fixture::new();
    let os = Fake::default();
    let input = Login::new(USER, FIRST).unwrap();
    let change = |s: &Selection, a, i| credentials::change(&f.store, &os, s, a, i);
    let fresh = f.selection("dev", "db");
    change(&fresh, Change::Set, Some(&input)).unwrap();
    let s = f.selection("dev", "db");
    let original = ItemKey::for_resource(s.resource()).unwrap();
    let replaced = Login::new(USER, SECOND).unwrap();
    let guard = f.store.lock_credentials().unwrap();
    assert!(matches!(
        change(&s, Change::Replace, Some(&replaced)),
        Err(Fault::Busy)
    ));
    drop(guard);
    // OS may have saved the item even when it reports an error. The old pointer is authoritative.
    os.write_fault.set(Some(Fault::Unavailable));
    assert!(matches!(
        change(&s, Change::Replace, Some(&replaced)),
        Err(Fault::Unavailable)
    ));
    os.write_fault.set(None);
    assert!(ItemKey::for_resource(f.selection("dev", "db").resource()).unwrap() == original);
    assert!(os.values.borrow()[&original.reference()] == input.0.as_slice());
    credentials::cleanup(
        &Store::open(f.paths.clone()).unwrap(),
        &os,
        f.id,
        &f.registry().origin,
    )
    .unwrap();
    assert_eq!(os.values.borrow().len(), 1);

    let db = f.db();
    db.execute_batch(
        "CREATE TRIGGER injected_failure BEFORE UPDATE OF credential_ref ON resource_bindings
        BEGIN SELECT RAISE(ABORT,'synthetic driver failure'); END;",
    )
    .unwrap();
    assert!(matches!(
        change(&s, Change::Replace, Some(&replaced)),
        Err(Fault::Storage)
    ));
    db.execute_batch("DROP TRIGGER injected_failure").unwrap();
    assert!(ItemKey::for_resource(f.selection("dev", "db").resource()).unwrap() == original);
    // A shared reference is retained even when a different resource rotates or is removed.
    let shared = f.selection("dev", "shared");
    db.execute(
        "UPDATE resource_bindings SET credential_ref=?1 WHERE id=?2",
        params![original.reference(), shared.resource().id.0],
    )
    .unwrap();
    change(&s, Change::Replace, Some(&replaced)).unwrap();
    assert!(matches!(
        change(&s, Change::Replace, Some(&input)),
        Err(Fault::Changed)
    ));
    let current = f.selection("dev", "db");
    assert!(ItemKey::for_resource(current.resource()).unwrap() != original);
    assert!(os.available(&original).is_ok());
    change(&current, Change::RemoveResource, None).unwrap();
    assert_eq!(os.values.borrow().len(), 1);
    assert!(credentials::status(&f.store, &os, &f.selection("dev", "shared")).is_ok());
    os.delete_fault.set(Some(Fault::AccessDenied));
    assert!(
        change(&f.selection("dev", "shared"), Change::Clear, None)
            .unwrap()
            .cleanup_pending
    );
    assert!(
        f.selection("dev", "shared")
            .resource()
            .credential_ref
            .is_none()
    );
    os.delete_fault.set(None);
    credentials::cleanup(&f.store, &os, f.id, &f.registry().origin).unwrap();
    assert!(os.values.borrow().is_empty());
}
#[test]
fn target_isolation_and_interrupted_writer_recovery() {
    let f = Fixture::new();
    let os = Fake::default();
    let s = f.selection("dev", "db");
    let input = Login::new(USER, FIRST).unwrap();
    credentials::change(&f.store, &os, &s, Change::Set, Some(&input)).unwrap();
    let dev = f.selection("dev", "db");
    let devkey = ItemKey::for_resource(dev.resource()).unwrap();
    let test = f.selection("test", "db");
    credentials::change(&f.store, &os, &test, Change::Set, Some(&input)).unwrap();
    let testkey = ItemKey::for_resource(f.selection("test", "db").resource()).unwrap();
    assert!(testkey != devkey);
    let db = f.db();
    db.execute(
        "UPDATE resource_bindings SET credential_ref=?1 WHERE id=?2",
        params![devkey.reference(), test.resource().id.0],
    )
    .unwrap();
    assert_eq!(
        credentials::status(&f.store, &os, &f.selection("test", "db")),
        Err(Fault::Damaged)
    );
    // T09 endpoint change detaches its reference and retires grants atomically.
    f.edit(Edit::Resource {
        target: alias("test"),
        alias: alias("db"),
        endpoint: Endpoint {
            host: "changed.invalid".into(),
            port: 3306,
            database: None,
            tls: TlsMode::VerifyIdentity,
        },
    });
    credentials::cleanup(&f.store, &os, f.id, &f.registry().origin).unwrap();
    assert!(os.available(&devkey).is_ok());
    assert!(os.available(&testkey).is_err());
}

struct CheckLogin {
    vault: NativeVault,
    expected: &'static str,
    calls: Rc<Cell<u32>>,
}
impl ResourceConnector for CheckLogin {
    fn kind(&self) -> ResourceKind {
        ResourceKind::MySql
    }
    fn test_connection(&self, r: &AuthorizedResource<'_>) -> Result<ConnectionResult, ErrorCode> {
        self.vault.with_login(r, |user, pw| {
            self.calls.set(self.calls.get() + 1);
            assert!(
                user == USER && pw == self.expected,
                "unexpected synthetic login"
            );
            Ok(ConnectionResult {
                latency_ms: 0,
                version_summary: "test consumer".into(),
            })
        })
    }
}
#[cfg(any(target_os = "macos", windows))]
#[test]
fn native_lifecycle_authorized_connector_repair_and_redaction() {
    let f = Fixture::new();
    let vault = || NativeVault::for_catalogue(f.paths.root()).unwrap();
    let native = vault();
    let os = Writer(&native);
    let session = Session::from_registration(f.registry(), f.id, Actor::Cli).unwrap();
    let run = |resource: &str, expected, allowed| {
        let args = json!({"target":"dev","resource":resource});
        let calls = Rc::new(Cell::new(0));
        let cases = fixture_cases::FixtureCases {
            connector: Some(Box::new(CheckLogin {
                vault: vault(),
                expected,
                calls: calls.clone(),
            })),
            ..Default::default()
        };
        let report = session.invoke(&cases, "connection_test", args.clone());
        assert_eq!(report.ok, allowed);
        assert_eq!(calls.get(), u32::from(allowed));
        serde_json::to_string(&report).unwrap()
    };
    let s = f.selection("dev", "db");
    assert_eq!(credentials::status(&f.store, &os, &s), Err(Fault::NotSet));
    credentials::change(
        &f.store,
        &os,
        &s,
        Change::Set,
        Some(&Login::new(USER, FIRST).unwrap()),
    )
    .unwrap();
    let denied = run("db", FIRST, false);
    let s = f.selection("dev", "db");
    assert!(credentials::status(&f.store, &os, &s).is_ok());
    f.edit(Edit::Grant {
        target: alias("dev"),
        alias: alias("db"),
        action: RemoteAction::Connect,
        expires_at: None,
    });
    let good = run("db", FIRST, true);
    let old = ItemKey::for_resource(s.resource()).unwrap();
    credentials::change(
        &f.store,
        &os,
        &s,
        Change::Replace,
        Some(&Login::new(USER, SECOND).unwrap()),
    )
    .unwrap();
    assert_eq!(os.available(&old), Err(Fault::Missing));
    let rotated = run("db", SECOND, true);
    let s = f.selection("dev", "db");
    let key = ItemKey::for_resource(s.resource()).unwrap();
    platform::write(&native.namespace, &key.reference(), b"invalid fixture blob").unwrap();
    assert_eq!(credentials::status(&f.store, &os, &s), Err(Fault::Damaged));
    let broken = run("db", SECOND, false);
    os.delete(&key).unwrap();
    assert_eq!(credentials::status(&f.store, &os, &s), Err(Fault::Missing));
    credentials::change(
        &f.store,
        &os,
        &s,
        Change::Repair,
        Some(&Login::new(USER, FIRST).unwrap()),
    )
    .unwrap();
    let repaired = run("db", FIRST, true);
    let s = f.selection("dev", "db");
    let key = ItemKey::for_resource(s.resource()).unwrap();
    let shared = f.selection("dev", "shared");
    f.db()
        .execute(
            "UPDATE resource_bindings SET credential_ref=?1 WHERE id=?2",
            params![key.reference(), shared.resource().id.0],
        )
        .unwrap();
    f.edit(Edit::Grant {
        target: alias("dev"),
        alias: alias("shared"),
        action: RemoteAction::Connect,
        expires_at: None,
    });
    credentials::change(&f.store, &os, &s, Change::RemoveResource, None).unwrap();
    assert!(os.available(&key).is_ok());
    let shared_report = run("shared", FIRST, true);
    let s = f.selection("dev", "shared");
    // Take a real SQLite snapshot while the configured credential exists.
    let backup = f.paths.backups().join("t10-snapshot.db");
    f.db()
        .backup(
            rusqlite::MAIN_DB,
            &backup,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .unwrap();
    let reader = f.db();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM resource_bindings;")
        .unwrap();
    credentials::change(&f.store, &os, &s, Change::Clear, None).unwrap();
    assert!(f.paths.database().with_extension("db-wal").exists());
    f.scan(&[denied, good, rotated, broken, repaired, shared_report].join("\n"));
    assert_eq!(os.available(&key), Err(Fault::Missing));
    println!(
        "T10 native: save, authorized use, rotate, damaged/missing, repair, clear; sanitized reports, SQLite/WAL and backup passed."
    );
}

#[test]
fn provider_failure_and_endpoint_changes_during_submission_are_recoverable() {
    let f = Fixture::new();
    let os = Fake::default();
    let s = f.selection("dev", "db");
    let input = Login::new(USER, FIRST).unwrap();
    os.write_fault.set(Some(Fault::AccessDenied));
    assert!(matches!(
        credentials::change(&f.store, &os, &s, Change::Set, Some(&input)),
        Err(Fault::AccessDenied)
    ));
    assert!(f.selection("dev", "db").resource().credential_ref.is_none());
    os.write_fault.set(None);
    credentials::change(&f.store, &os, &s, Change::Set, Some(&input)).unwrap();
    assert_eq!(os.values.borrow().len(), 1);
    let s = f.selection("dev", "db");
    for fault in [Fault::AccessDenied, Fault::Unavailable] {
        os.read_fault.set(Some(fault));
        assert_eq!(credentials::status(&f.store, &os, &s), Err(fault));
    }
    os.read_fault.set(None);
    let db = f.db();
    let id = s.resource().id;
    *os.after_write.borrow_mut() = Some(Box::new(move || {
        db.execute(
            "UPDATE resource_bindings SET host='different.invalid' WHERE id=?1",
            [id.0],
        )
        .unwrap();
    }));
    assert!(matches!(
        credentials::change(
            &f.store,
            &os,
            &s,
            Change::Replace,
            Some(&Login::new(USER, SECOND).unwrap())
        ),
        Err(Fault::Changed)
    ));
    let db = f.db();
    assert!(db.query_row("SELECT credential_ref IS NULL AND confirmed_at_ms IS NULL FROM resource_bindings WHERE id=?1",[id.0],|r|r.get::<_,bool>(0)).unwrap());
    credentials::cleanup(
        &Store::open(f.paths.clone()).unwrap(),
        &os,
        f.id,
        &f.registry().origin,
    )
    .unwrap();
    assert!(os.values.borrow().is_empty());
}
