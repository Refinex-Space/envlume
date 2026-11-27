#[path = "credentials.rs"]
mod credentials;
#[path = "database.rs"]
mod database;
#[path = "doctor.rs"]
mod doctor;
#[path = "environments.rs"]
mod environments;
#[path = "registration.rs"]
pub mod registration;
#[path = "reports.rs"]
mod reports;
#[path = "tasks.rs"]
mod tasks;
use envlume_application::projects::{StorageError, list_projects};
use envlume_domain::BindingId;
use envlume_mcp::Resolution;
use envlume_storage::{Store, StorePaths};
use std::{ffi::OsString, process::ExitCode};

const USAGE: &str = "Usage: envlume connection test <target> <resource> --binding <id> [--json] | envlume database schema <target> <resource> [--table <name>] --binding <id> [--json] | envlume credential set|replace|repair|clear|status|remove-resource <target> <resource> --binding <id> [--json] | envlume credential rebind <target> <destination> <source> --binding <id> [--json] | envlume credential cleanup --binding <id> [--json] | envlume environment list --binding <id> [--json] | envlume environment set|rename|resource|grant|revoke ... --binding <id> | envlume report show --evidence|--request <id> --binding <id> [--json] | envlume report clear --binding <id> [--json] | envlume task list --binding <id> [--json] | envlume task enable <name> --binding <id> [--timeout <seconds>] [--json] | envlume task disable|run|cancel <name> --binding <id> [--json] | envlume --version | envlume doctor [--binding <id> [--profile <name>]] [--json] | envlume toolchain configure --binding <id> --profile <name> [--tool <name> <constraint> [--path <absolute>]]... [--mise] [--json] | envlume project list [--json] | envlume project add <path> [--project <id>] [--json] | envlume project inspect --path <path> [--json] | envlume project revoke --binding <id> [--json] | envlume project delete --project <id> [--json] | envlume mcp --binding <uuid>";

pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
    resolve: impl FnOnce(BindingId) -> Resolution,
) -> ExitCode {
    if args
        .first()
        .is_some_and(|op| op == "connection" || op == "database")
    {
        return database::run(args, locate);
    }
    if args.first().is_some_and(|op| op == "credential") {
        return credentials::run(&args[1..], locate);
    }
    if args.first().is_some_and(|op| op == "environment") {
        return environments::run(&args[1..], locate);
    }
    if args.first().is_some_and(|op| op == "report") {
        return reports::run(&args[1..], locate);
    }
    if args.first().is_some_and(|op| op == "task") {
        return tasks::run(&args[1..], locate);
    }
    if args
        .first()
        .is_some_and(|a| a == "doctor" || a == "toolchain")
    {
        return doctor::run(args, locate);
    }
    if let [command, flag, raw_id] = args
        && command == "mcp"
        && flag == "--binding"
    {
        let Some(id) = raw_id.to_str().and_then(|s| s.parse().ok()).map(BindingId) else {
            eprintln!("Envlume MCP: invalid binding UUID");
            return ExitCode::from(2);
        };
        return match envlume_mcp::run_stdio(resolve(id)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => {
                eprintln!("{}", code.summary());
                ExitCode::from(code.exit_code())
            }
        };
    }

    if args == ["--version"] || args == ["-V"] {
        println!("envlume {}", envlume_application::VERSION);
    } else if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
    } else if args == ["project", "list"] || args == ["project", "list", "--json"] {
        let json = args.len() == 3;
        match locate()
            .and_then(Store::open)
            .and_then(|s| list_projects(&s))
        {
            Ok(result) if json => println!(
                "{}",
                serde_json::to_string(&result).expect("serializable project list")
            ),
            Ok(result) if result.projects.is_empty() => println!("No projects."),
            Ok(result) => {
                for p in result.projects {
                    println!("{}\t{}", p.id.0, p.name.replace(char::is_control, " "));
                }
            }
            Err(error) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"schemaVersion":1,"errorCode":error,"summary":error.message()})
                    );
                }
                eprintln!("{}", error.message());
                return ExitCode::from(if error == StorageError::StorageBusy {
                    6
                } else {
                    7
                });
            }
        }
    } else if args.first().is_some_and(|s| s == "project") {
        return registration::run(&args[1..], locate);
    } else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
