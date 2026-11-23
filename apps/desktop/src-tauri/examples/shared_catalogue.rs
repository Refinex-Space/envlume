//! Native IPC smoke test. The optional CLI comparison uses a fresh CI user's
//! standard application directory; the default uses only a temporary directory.
#[path = "../src/commands.rs"]
mod commands;

use envlume_storage::{Store, StorePaths};
use serde_json::{Value, json};
use std::{process::Command, thread, time::Duration};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, webview::PageLoadEvent};

fn main() {
    let temp = tempfile::tempdir().unwrap();
    let cli = std::env::args_os().nth(1);
    let paths = if cli.is_some() {
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
    Store::restore_into(&fixture.path().canonicalize().unwrap(), paths.clone()).unwrap();
    let expected = json!({"schemaVersion":1,"projects":[{"id":"11111111-1111-4111-8111-111111111111","name":"orders"}]});
    if let Some(binary) = cli {
        let out = Command::new(binary)
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
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app=commands::with_commands(tauri::Builder::default())
        .manage(paths)
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
                            window.app_handle().exit(0);
                        } else {eprintln!("Unexpected desktop catalogue: {actual:?}");window.app_handle().exit(1);}
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
