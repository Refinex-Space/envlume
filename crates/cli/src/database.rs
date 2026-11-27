use super::registration::endpoint;
use envlume_application::{ErrorCode, projects::StorageError, result::Report};
use envlume_domain::{Actor, BindingId};
use envlume_storage::StorePaths;
use serde_json::json;
use std::{ffi::OsString, process::ExitCode};

pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, machine) = match args.split_last() {
        Some((f, rest)) if f == "--json" => (rest, true),
        _ => (args, false),
    };
    let name = if args.first().is_some_and(|s| s == "connection") {
        "connection_test"
    } else {
        "database_schema"
    };
    let result = (|| {
        let parts = args
            .iter()
            .map(|s| s.to_str().ok_or(ErrorCode::InvalidInput))
            .collect::<Result<Vec<_>, _>>()?;
        let [cmd, op, t, r, options @ .., "--binding", id] = parts.as_slice() else {
            return Err(ErrorCode::InvalidInput);
        };
        let mut params = json!({"target":t,"resource":r});
        match (*cmd, *op, options) {
            ("connection", "test", []) | ("database", "schema", []) => (),
            ("database", "schema", ["--table", table]) => params["table"] = json!(table),
            _ => return Err(ErrorCode::InvalidInput),
        }
        let access = endpoint(
            locate(),
            BindingId(id.parse().map_err(|_| ErrorCode::InvalidInput)?),
            Actor::Cli,
        )?;
        let report = access.database(name, params);
        Ok(report)
    })();
    let report = result.unwrap_or_else(|e| {
        let mut r = Report::new(name);
        r.fail(e, e.summary());
        r
    });
    if machine {
        println!("{}", json!(report));
    } else if report.ok {
        println!("{}", report.human_text());
        println!(
            "{}",
            serde_json::to_string_pretty(&report.result).expect("typed result")
        );
    } else {
        eprintln!("{}", report.human_text());
    }
    report.exit_code().into()
}
