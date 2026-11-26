use envlume_application::{ErrorCode, projects::StorageError, result::Report};
use envlume_domain::{BindingId, EvidenceId, RequestId};
use envlume_storage::{RecordKey, Store, StorePaths};
use serde_json::{Value, json};
use std::{env, ffi::OsString, process::ExitCode};

pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, machine) = match args.split_last() {
        Some((v, a)) if v == "--json" => (a, true),
        _ => (args, false),
    };
    let result = (|| -> Result<Value, ErrorCode> {
        let (op, selector, value, binding) = match args {
            [op, selector, value, flag, id] if op == "show" && flag == "--binding" => {
                (op, selector.to_str(), value.to_str(), id)
            }
            [op, flag, id] if op == "clear" && flag == "--binding" => (op, None, None, id),
            _ => return Err(ErrorCode::InvalidInput),
        };
        let native = |e: envlume_application::workspaces::ManagementError| match e {
            envlume_application::workspaces::ManagementError::Storage(
                StorageError::StorageBusy,
            ) => ErrorCode::Timeout,
            envlume_application::workspaces::ManagementError::Storage(_) => {
                ErrorCode::InternalError
            }
            _ => ErrorCode::PolicyDenied,
        };
        let binding = BindingId(
            binding
                .to_str()
                .and_then(|s| s.parse().ok())
                .ok_or(ErrorCode::InvalidInput)?,
        );
        let store = Store::open(locate().map_err(|_| ErrorCode::InternalError)?)
            .map_err(|_| ErrorCode::InternalError)?;
        let origin = env::current_dir().map_err(|_| ErrorCode::ProjectNotTrusted)?;
        if op == "clear" {
            let n = store.clear_records(binding, &origin).map_err(native)?;
            return Ok(json!({"schemaVersion":1,"deleted":n}));
        }
        let raw = value
            .and_then(|s| s.parse().ok())
            .ok_or(ErrorCode::InvalidInput)?;
        let key = match selector {
            Some("--evidence") => RecordKey::Evidence(EvidenceId(raw)),
            Some("--request") => RecordKey::Request(RequestId(raw)),
            _ => return Err(ErrorCode::InvalidInput),
        };
        store
            .lookup_record(binding, &origin, key)
            .map_err(native)?
            .ok_or(ErrorCode::PolicyDenied)
    })();
    match result {
        Ok(v) => {
            if machine {
                println!("{v}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).expect("JSON value"));
            }
            ExitCode::SUCCESS
        }
        Err(code) => {
            let mut r = Report::new("unknown");
            r.fail(code, code.summary());
            if machine {
                println!("{}", json!(r));
            }
            eprintln!("{}", r.human_text());
            ExitCode::from(code.exit_code())
        }
    }
}
