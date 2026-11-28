use super::*;
use envlume_application::projects::StorageError;
use serde_json::json;
use std::{
    ffi::OsString,
    io::{self, IsTerminal, Write},
    process::ExitCode,
};

pub fn command(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, json_mode) = match args.split_last() {
        Some((flag, args)) if flag == "--json" => (args, true),
        _ => (args, false),
    };
    let result = (|| {
        let words = args
            .iter()
            .map(|a| a.to_str().ok_or(Fault::InvalidInput))
            .collect::<Result<Vec<_>, _>>()?;
        let [action, client, options @ .., "--binding", id] = words.as_slice() else {
            return Err(Fault::InvalidInput);
        };
        let action = match *action {
            "detect" => Action::Detect,
            "preview" => Action::Preview,
            "install" => Action::Install,
            "verify" => Action::Verify,
            "uninstall" => Action::Uninstall,
            _ => return Err(Fault::InvalidInput),
        };
        let client = match *client {
            "claude-code" => ClientKind::ClaudeCode,
            "codex" => ClientKind::Codex,
            "cursor" => ClientKind::Cursor,
            _ => return Err(Fault::InvalidInput),
        };
        let shared_project = options.contains(&"--shared-project");
        let preview_remove = options.contains(&"--uninstall");
        if shared_project && client != ClientKind::ClaudeCode
            || preview_remove && action != Action::Preview
            || options.len() != usize::from(shared_project) + usize::from(preview_remove)
        {
            return Err(Fault::InvalidInput);
        }
        let action = if preview_remove {
            Action::PreviewUninstall
        } else {
            action
        };
        let binding = BindingId(id.parse().map_err(|_| Fault::InvalidInput)?);
        let paths = locate().map_err(|_| Fault::Storage)?;
        let store = Store::open(paths.clone()).map_err(|e| {
            if e == StorageError::StorageBusy {
                Fault::Busy
            } else {
                Fault::Storage
            }
        })?;
        let origin = std::env::current_dir().map_err(|_| Fault::UntrustedBinding)?;
        Service {
            store: &store,
            paths: &paths,
            origin: &origin,
            binding,
            client,
            adapter: &LocalAdapter,
            shared_project,
        }
        .execute(action, |view| {
            if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
                return false;
            }
            let location = if view.scope == "local" {
                "Private Claude Code profile, entry for this project"
            } else {
                "Repository configuration (may be committed to Git)"
            };
            eprintln!("{location}: {}", json!(view));
            eprint!("Type confirm to apply: ");
            let mut line = String::new();
            io::stderr().flush().is_ok()
                && io::stdin().read_line(&mut line).is_ok()
                && line.trim_end() == "confirm"
        })
    })();
    match result {
        Ok(r) => {
            if json_mode {
                println!("{}", json!({"schemaVersion":1,"ok":true,"result":r}));
            } else {
                println!("{} {}", r.state, json!(r.preview));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json_mode {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"ok":false,"errorCode":e,"summary":e.message()})
                );
            }
            eprintln!("{}", e.message());
            e.exit().into()
        }
    }
}
