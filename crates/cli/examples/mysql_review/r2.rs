//! Provision two native-vault-backed projects in one disposable catalogue.
use super::*;

pub(super) fn provision(cfg: &Value, lines: impl Iterator<Item = std::io::Result<String>>) {
    let base = Path::new(cfg["base"].as_str().unwrap());
    let paths = StorePaths::at(base.join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let ca = envlume_application::mysql_policy::public_ca(
        &std::fs::read(cfg["ca"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let port = cfg["port"].as_u64().unwrap().try_into().unwrap();
    let vault = NativeVault::for_catalogue(store.clone()).unwrap();
    let mut fixtures = Vec::new();
    let mut projects = Vec::new();
    for (project, allowed) in [("a", "dev"), ("b", "test")] {
        let root = base.join(project);
        std::fs::create_dir(&root).unwrap();
        let registration = add_project(&store, &root, None, |_, _| true).unwrap();
        let mut f = Fixture {
            store: store.clone(),
            root,
            binding: registration.binding_id,
            keys: vec![],
        };
        for target in ["dev", "test", "prod"] {
            f.edit(Edit::preset(alias(target)).unwrap());
            let account = if target == "prod" { "test" } else { target };
            let names: &[&str] = if project == "b" && target == "test" {
                &["primary-db", "ledger"]
            } else {
                &["primary-db"]
            };
            for &name in names {
                f.edit(Edit::Resource {
                    target: alias(target),
                    alias: alias(name),
                    endpoint: Endpoint {
                        host: "localhost".into(),
                        port,
                        tls: TlsMode::VerifyIdentity,
                        database: Some(format!("orders_{account}")),
                    },
                });
                let marker = format!("{account}_marker");
                f.edit(Edit::MySqlPolicy {
                    target: alias(target),
                    alias: alias(name),
                    route: MySqlRoute {
                        ips: ["127.0.0.1", "::1"].map(|ip| ip.parse().unwrap()).into(),
                        ca_pem: Some(ca.clone()),
                    },
                    tables: BTreeMap::from([(
                        marker,
                        MySqlTable {
                            columns: ["id", "name", "payload"].map(str::to_owned).into(),
                            indexes: ["PRIMARY".into()].into(),
                        },
                    )]),
                });
                let mut policy = query_policy(account);
                // Use the production ceiling for the three simultaneous lock waiters.
                policy.timeout_ms = 5000;
                f.edit(Edit::QueryPolicy {
                    target: alias(target),
                    alias: alias(name),
                    policy,
                });
                let s = credentials::select(&f.repo(), f.binding, &alias(target), &alias(name))
                    .unwrap();
                let login = &cfg["accounts"][account];
                f.keys.push((alias(target), alias(name)));
                vault
                    .change(
                        &s,
                        credentials::Change::Set,
                        Some(
                            &Login::new(
                                login["user"].as_str().unwrap(),
                                login["password"].as_str().unwrap(),
                            )
                            .unwrap(),
                        ),
                    )
                    .unwrap();
                if target == allowed {
                    for action in [
                        RemoteAction::Connect,
                        RemoteAction::Schema,
                        RemoteAction::Read,
                    ] {
                        f.grant(target, name, action);
                    }
                }
            }
        }
        projects.push(
            json!({"workspace": f.root, "binding": f.binding, "project":registration.project_id}),
        );
        fixtures.push(f);
    }
    println!("{}", json!({"projects":projects,"store":paths.root()}));
    std::io::stdout().flush().unwrap();
    assert!(
        lines.into_iter().next().is_none(),
        "provisioner expects EOF"
    );
    for f in &mut fixtures {
        for (target, resource) in f.keys.clone() {
            let selection = credentials::select(&f.repo(), f.binding, &target, &resource).unwrap();
            let cleaned = vault
                .change(&selection, credentials::Change::Clear, None)
                .unwrap();
            assert!(!cleaned.cleanup_pending);
        }
        f.keys.clear();
    }
}
