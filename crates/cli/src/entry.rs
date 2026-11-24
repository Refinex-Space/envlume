use envlume_application::projects::{StorageError, list_projects};
use envlume_domain::BindingId;
use envlume_mcp::Resolution;
use envlume_storage::{Store, StorePaths};
use std::{ffi::OsString, process::ExitCode};

const USAGE: &str =
    "Usage: envlume --version | envlume project list [--json] | envlume mcp --binding <uuid>";

pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
    resolve: impl FnOnce(BindingId) -> Resolution,
) -> ExitCode {
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
    } else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
