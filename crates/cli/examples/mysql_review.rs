//! Test harness for tests/mysql/review.py. Credentials arrive only on the private input pipe.
use envlume_application::{
    ErrorCode, Session,
    credentials::{self},
    environments::{Edit, Endpoint, RemoteAction, manage},
    ports::AuthorizedResource,
    remote::DatabaseBackend,
    workspaces::add_project,
};
use envlume_connector_mysql::{Logins, MySqlConnector};
use envlume_connectors::Registry;
use envlume_domain::*;
#[cfg(any(windows, target_os = "macos"))]
use envlume_secrets::{Login, NativeVault};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct ReviewLogins {
    #[cfg(any(windows, target_os = "macos"))]
    native: NativeVault<Store>,
    #[cfg(not(any(windows, target_os = "macos")))]
    fixture: std::collections::HashMap<ResourceId, (String, String)>,
    reads: Arc<AtomicUsize>,
}
impl Logins for ReviewLogins {
    fn with_login<T>(
        &self,
        r: &AuthorizedResource<'_>,
        op: impl FnOnce(&str, &str) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        #[cfg(any(windows, target_os = "macos"))]
        {
            self.native.with_login(r, op)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let item = self
                .fixture
                .get(&r.binding()?.id)
                .ok_or(ErrorCode::CredentialUnavailable)?;
            op(&item.0, &item.1)
        }
    }
}
fn alias(s: &str) -> Alias {
    s.to_owned().try_into().unwrap()
}
struct Fixture {
    store: Store,
    root: PathBuf,
    binding: BindingId,
    keys: Vec<(Alias, Alias)>,
}
impl Fixture {
    fn repo(&self) -> LocalRegistration {
        LocalRegistration {
            store: self.store.clone(),
            origin: self.root.clone(),
        }
    }
    fn edit(&self, edit: Edit) {
        manage(
            &self.repo(),
            &self.store,
            self.binding,
            &self.root,
            edit,
            |_| true,
        )
        .unwrap();
    }
    fn grant(&self, target: &str, name: &str, action: RemoteAction) {
        self.edit(Edit::Grant {
            target: alias(target),
            alias: alias(name),
            action,
            expires_at: None,
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(any(windows, target_os = "macos"))]
        if let Ok(v) = NativeVault::for_catalogue(self.store.clone()) {
            for (t, r) in &self.keys {
                if let Ok(s) = credentials::select(&self.repo(), self.binding, t, r) {
                    let _ = v.change(&s, credentials::Change::Clear, None);
                }
            }
        }
    }
}
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args.is_empty() {
        let paths = || {
            StorePaths::at(PathBuf::from(
                std::env::var_os("T05_FIXTURE_STORE").expect("isolated fixture store"),
            ))
        };
        return envlume::run(&args, paths, |id| envlume::resolve(paths(), id));
    }
    review();
    std::process::ExitCode::SUCCESS
}
fn review() {
    let input = std::io::stdin();
    let mut lines = input.lock().lines();
    let mut first = lines.next().unwrap().unwrap();
    let cfg: Value = serde_json::from_str(&first).expect("fixture configuration");
    first.clear();
    let base = Path::new(cfg["base"].as_str().unwrap());
    let root = base.join("workspace");
    std::fs::create_dir(&root).unwrap();
    let paths = StorePaths::at(base.join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let binding = add_project(&store, &root, None, |_, _| true)
        .unwrap()
        .binding_id;
    let mut fixture = Fixture {
        store: store.clone(),
        root,
        binding,
        keys: vec![],
    };
    let ca = envlume_application::mysql_policy::public_ca(
        &std::fs::read(cfg["ca"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    #[cfg(any(windows, target_os = "macos"))]
    let vault = NativeVault::for_catalogue(store.clone()).unwrap();
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut entries = std::collections::HashMap::new();
    for target in ["dev", "test"] {
        fixture.edit(Edit::preset(alias(target)).unwrap());
        let marker = format!("{target}_marker");
        let database = format!("orders_{target}");
        for resource in [
            "primary-db",
            "unapproved",
            "bad-cert",
            "wrong-name",
            "offline",
            "hung",
            "bad-login",
            "dns-fail",
        ] {
            if target == "test" && resource != "primary-db" {
                continue;
            }
            let host = match resource {
                "wrong-name" => "127.0.0.1",
                "dns-fail" => "absent.invalid",
                _ => "localhost",
            };
            let port = cfg[match resource {
                "offline" => "offline",
                "hung" => "hung",
                _ => "port",
            }]
            .as_u64()
            .unwrap() as u16;
            fixture.edit(Edit::Resource {
                target: alias(target),
                alias: alias(resource),
                endpoint: Endpoint {
                    host: host.into(),
                    port,
                    database: Some(database.clone()),
                    tls: TlsMode::VerifyIdentity,
                },
            });
            let ips = if resource == "unapproved" {
                vec!["192.0.2.10"]
            } else {
                vec!["127.0.0.1", "::1"]
            };
            let route = MySqlRoute {
                ips: ips.into_iter().map(|s| s.parse().unwrap()).collect(),
                ca_pem: (resource != "bad-cert").then(|| ca.clone()),
            };
            let tables = BTreeMap::from([(
                marker.clone(),
                MySqlTable {
                    columns: ["id", "name"].into_iter().map(str::to_owned).collect(),
                    indexes: ["PRIMARY", "name_idx", "mixed_idx"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                },
            )]);
            fixture.edit(Edit::MySqlPolicy {
                target: alias(target),
                alias: alias(resource),
                route,
                tables,
            });
            let selection =
                credentials::select(&fixture.repo(), binding, &alias(target), &alias(resource))
                    .unwrap();
            let user = cfg["accounts"][target]["user"].as_str().unwrap();
            let password = if resource == "bad-login" {
                "unusable-fixture-password"
            } else {
                cfg["accounts"][target]["password"].as_str().unwrap()
            };
            fixture.keys.push((alias(target), alias(resource)));
            #[cfg(any(windows, target_os = "macos"))]
            vault
                .change(
                    &selection,
                    credentials::Change::Set,
                    Some(&Login::new(user, password).unwrap()),
                )
                .unwrap();
            #[cfg(not(any(windows, target_os = "macos")))]
            entries.insert(
                selection.resource().id,
                (user.to_owned(), password.to_owned()),
            );
            fixture.grant(target, resource, RemoteAction::Connect);
            if resource == "primary-db" {
                fixture.grant(target, resource, RemoteAction::Schema);
            }
        }
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let logins = ReviewLogins {
        #[cfg(any(windows, target_os = "macos"))]
        native: vault,
        #[cfg(not(any(windows, target_os = "macos")))]
        fixture: entries,
        reads: reads.clone(),
    };
    let backend = Registry::new(MySqlConnector::new(logins).unwrap());
    let session = Session::from_registration(fixture.repo(), binding, Actor::Cli).unwrap();
    let other = base.join("other");
    std::fs::create_dir(&other).unwrap();
    let foreign = add_project(&store, &other, None, |_, _| true)
        .unwrap()
        .binding_id;
    let different = Session::from_registration(
        LocalRegistration {
            store,
            origin: other,
        },
        foreign,
        Actor::Cli,
    )
    .unwrap();
    println!(
        "{}",
        json!({"binding":binding,"workspace":fixture.root,"store":paths.root()})
    );
    for line in lines {
        let q: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let op = q["op"].as_str().unwrap();
        let args = &q["args"];
        let result = match op {
            "foreign" => {
                json!(different.remote_operation("connection_test", args.clone(), &backend))
            }
            "parallel" => std::thread::scope(|scope| {
                let handles: Vec<_> = args
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| {
                        scope.spawn(|| {
                            session.remote_operation("database_schema", a.clone(), &backend)
                        })
                    })
                    .collect();
                json!(
                    handles
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .collect::<Vec<_>>()
                )
            }),
            "health" => {
                session.refresh_remote(Some(&backend)).unwrap();
                json!(session.environment_health(json!({}), &|r| backend.health(binding, r)))
            }
            "regrant" => {
                fixture.grant("dev", "primary-db", RemoteAction::Connect);
                json!(true)
            }
            "revoke" => {
                fixture.edit(Edit::Revoke {
                    target: alias("dev"),
                    alias: alias("primary-db"),
                    action: RemoteAction::Connect,
                });
                json!(true)
            }
            "close" => {
                backend.close().unwrap();
                json!(true)
            }
            "connection_test" | "database_schema" => {
                json!(session.remote_operation(op, args.clone(), &backend))
            }
            _ => panic!("unknown fixture operation"),
        };
        println!(
            "{}",
            json!({"result":result,"reads":reads.load(Ordering::SeqCst)})
        );
        std::io::stdout().flush().unwrap();
    }
    backend.close().unwrap();
}
