use envlume_application::projects::{StorageError, list_projects};
use envlume_storage::{Store, StorePaths};
use std::{env, ffi::OsString, process::ExitCode};

const USAGE: &str = "Usage: envlume --version | envlume project list [--json]";

fn run(args: &[OsString], locate: impl FnOnce() -> Result<StorePaths, StorageError>) -> ExitCode {
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
    } else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    run(
        &env::args_os().skip(1).collect::<Vec<_>>(),
        StorePaths::local,
    )
}
