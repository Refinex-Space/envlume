//! Native IPC smoke test. The optional CLI comparison uses a fresh CI user's
//! standard application directory; the default uses only a temporary directory.
#[path = "../src/commands.rs"]
mod commands;

use envlume_application::{
    environments::{Edit, Endpoint, RemoteAction, manage},
    workspaces::{WorkspaceRepository, add_project},
};
use envlume_domain::{ProjectId, TlsMode};
use envlume_storage::{LocalRegistration, Store, StorePaths};

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::Mutex,
    thread,
    time::Duration,
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, webview::PageLoadEvent};

// Keep the same STDIO server alive while the main window observes a grant change.
struct Peers {
    binary: PathBuf,
    origin: PathBuf,
    paths: StorePaths,
    binding: String,
    server: Child,
    lines: BufReader<ChildStdout>,
    sequence: u32,
}
impl Peers {
    fn new(binary: PathBuf, origin: &Path, paths: StorePaths, binding: String) -> Self {
        let mut server = Command::new(&binary)
            .args(["mcp", "--binding", &binding])
            .current_dir(origin)
            .env("T05_FIXTURE_STORE", paths.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = BufReader::new(server.stdout.take().unwrap());
        let mut peer = Self {
            binary,
            origin: origin.into(),
            paths,
            binding,
            server,
            lines,
            sequence: 0,
        };
        peer.request("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"native-smoke","version":"1"}}));
        writeln!(
            peer.server.stdin.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        peer
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.sequence += 1;
        let input = self.server.stdin.as_mut().unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.sequence,"method":method,"params":params})
        )
        .unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        self.lines.read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(msg["id"], self.sequence);
        assert!(msg.get("error").is_none(), "{msg}");
        msg["result"].clone()
    }
    fn compare(&mut self, expected: &Value) {
        let mcp = self.request(
            "tools/call",
            json!({"name":"environment_list","arguments":{}}),
        );

        let out = Command::new(&self.binary)
            .args(["environment", "list", "--binding", &self.binding, "--json"])
            .current_dir(&self.origin)
            .env("T05_FIXTURE_STORE", self.paths.root())
            .output()
            .unwrap();
        assert_eq!(out.status.success(), expected["ok"] == true);
        let cli: Value = serde_json::from_slice(&out.stdout).unwrap();
        for key in ["ok", "status", "result", "errorCode"] {
            assert_eq!(cli[key], expected[key]);
            assert_eq!(mcp["structuredContent"][key], expected[key]);
        }
    }
}
impl Drop for Peers {
    fn drop(&mut self) {
        drop(self.server.stdin.take());
        let _ = self.server.wait();
    }
}
fn main() {
    let temp = tempfile::tempdir().unwrap();
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let isolated = first.as_deref() == Some(std::ffi::OsStr::new("--fixture-cli"));
    let cli = if isolated {
        Some(args.next().expect("fixture binary required"))
    } else {
        first
    }
    .map(|p| PathBuf::from(p).canonicalize().unwrap());
    let paths = if cli.is_some() && !isolated {
        StorePaths::local().unwrap()
    } else {
        StorePaths::at(temp.path().join("Envlume")).unwrap()
    };
    assert!(
        !paths.root().exists(),
        "native smoke requires an unused application directory"
    );
    let fixture = tempfile::NamedTempFile::new_in(temp.path()).unwrap();
    {
        let db = rusqlite::Connection::open(fixture.path()).unwrap();
        db.execute_batch(include_str!(
            "../../../../crates/storage/migrations/001_baseline.sql"
        ))
        .unwrap();
        db.execute_batch("PRAGMA application_id=0x454e564c; PRAGMA user_version=1; INSERT INTO schema_migrations VALUES(1,1700000000000);").unwrap();
        db.execute_batch(include_str!(
            "../../../../crates/storage/tests/fixtures/previous.sql"
        ))
        .unwrap();
    }
    let store =
        Store::restore_into(&fixture.path().canonicalize().unwrap(), paths.clone()).unwrap();
    let expected = json!({"schemaVersion":1,"projects":[{"id":"11111111-1111-4111-8111-111111111111","name":"orders"}]});
    if let Some(binary) = &cli {
        let out = Command::new(binary)
            .env("T05_FIXTURE_STORE", paths.root())
            .args(["project", "list", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "CLI failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&out.stdout).unwrap(),
            expected
        );
        println!("Independent CLI: {expected}");
    }
    let origin = temp.path().join("bound-workspace");
    std::fs::create_dir(&origin).unwrap();
    let project = ProjectId("11111111-1111-4111-8111-111111111111".parse().unwrap());
    let binding = add_project(&store, &origin, Some(project), |_, _| true)
        .unwrap()
        .binding_id;
    let registry = LocalRegistration {
        store: store.clone(),
        origin: origin.clone(),
    };
    let cleanup = store.clone();
    let edit = move |change| {
        manage(&registry, &store, binding, &registry.origin, change, |_| {
            true
        })
        .unwrap()
    };
    let alias = |s: &str| envlume_domain::Alias::try_from(s.to_owned()).unwrap();
    edit(Edit::Resource {
        target: alias("dev"),
        alias: alias("primary-db"),
        endpoint: Endpoint {
            host: "fixture.invalid".into(),
            port: 3306,
            tls: TlsMode::VerifyIdentity,
            database: None,
        },
    });
    edit(Edit::Grant {
        target: alias("dev"),
        alias: alias("primary-db"),
        action: RemoteAction::Schema,
        expires_at: None,
    });
    let peers = cli.map(|bin| {
        Mutex::new(Peers::new(
            bin,
            &origin,
            paths.clone(),
            binding.0.to_string(),
        ))
    });
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app=commands::with_commands(tauri::Builder::default())
        .manage(paths)
        .manage(commands::DesktopBinding(Some((binding,origin))))
        .setup(move |app|{
            WebviewWindowBuilder::new(app,"main",WebviewUrl::App("index.html".into()))
                .visible(false)
                .on_page_load(|window,payload|{
                    if payload.event()==PageLoadEvent::Finished {
                        window.eval("window.__TAURI_INTERNALS__.invoke('list_projects').then(x => document.title = 'T03:' + JSON.stringify(x)).catch(() => document.title = 'T03:null')")
                            .expect("dispatch catalogue command");
                    }
                })
                .on_document_title_changed(move |window,title|{
                    if let Some(payload)=title.strip_prefix("T03:"){
                        let actual=serde_json::from_str::<Value>(payload).ok();
                        if actual.as_ref()==Some(&expected) {
                            println!("Native desktop IPC: {expected}");
                            window.eval("window.__TAURI_INTERNALS__.invoke('list_environments').then(x => document.title = 'T09:before:' + JSON.stringify(x))").unwrap();
                        } else {eprintln!("Unexpected desktop catalogue: {actual:?}");window.app_handle().exit(1);}
                    }
                    if let Some(payload)=title.strip_prefix("T09:before:") {
                        let actual: Value=serde_json::from_str(payload).unwrap();
                        assert_eq!(actual["result"],json!({"environments":[{"target":"dev","resources":[{"alias":"primary-db","health":"unknown"}]}]}));
                        if let Some(peer)=&peers {peer.lock().unwrap().compare(&actual);}
                        edit(Edit::Revoke{target:alias("dev"),alias:alias("primary-db"),action:RemoteAction::Schema});
                        window.eval("window.__TAURI_INTERNALS__.invoke('list_environments').then(x => document.title = 'T09:after:' + JSON.stringify(x))").unwrap();
                    }
                    if let Some(payload)=title.strip_prefix("T09:after:") {
                        let actual:Value=serde_json::from_str(payload).unwrap();
                        assert_eq!(actual["result"],json!({"environments":[]}));
                        if let Some(peer)=&peers {peer.lock().unwrap().compare(&actual);println!("T09: CLI, live MCP and native IPC agreed before and after revocation on the same binding");}
                        cleanup.delete_project(project).unwrap();
                        window.eval("window.__TAURI_INTERNALS__.invoke('list_environments').then(x => document.title = 'T09:deleted:' + JSON.stringify(x)).catch(() => document.title = 'T09:deleted:null')").unwrap();
                    }
                    if let Some(payload)=title.strip_prefix("T09:deleted:"){
                        let actual:Value=serde_json::from_str(payload).unwrap();
                        assert_eq!(actual["errorCode"],"PROJECT_NOT_TRUSTED");
                        if let Some(peer)=&peers {peer.lock().unwrap().compare(&actual);}
                        println!("T09: native main-window IPC reloaded revocation and denied the deleted project");
                        window.app_handle().exit(0);
                    }
                }).build()?;
            let handle=app.handle().clone();
            thread::spawn(move ||{thread::sleep(Duration::from_secs(45));eprintln!("Timed out waiting for native IPC");handle.exit(1);});
            Ok(())
        }).build(context).unwrap();
    let code = app.run_return(|_, _| {});
    drop(temp);
    std::process::exit(code);
}
