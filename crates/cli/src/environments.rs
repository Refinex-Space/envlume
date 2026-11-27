use super::registration::confirm;
use envlume_application::{
    ErrorCode, Session,
    environments::{self, Edit, Endpoint, RemoteAction},
    projects::StorageError,
    result::{Payload, Report},
    workspaces::inspect_project,
};
use envlume_domain::*;
use envlume_storage::{LocalRegistration, Store, StorePaths};
use std::{env, ffi::OsString, process::ExitCode};

fn alias(s: &str) -> Result<Alias, ErrorCode> {
    s.to_owned().try_into().map_err(|_| ErrorCode::InvalidInput)
}
fn file(path: &str, max: u64) -> Result<Vec<u8>, ErrorCode> {
    use std::io::Read;
    let path = envlume_workspace::local_canonical(std::path::Path::new(path))
        .map_err(|_| ErrorCode::InvalidInput)?;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        open.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let handle = open.open(path).map_err(|_| ErrorCode::InvalidInput)?;
    let m = handle.metadata().map_err(|_| ErrorCode::InvalidInput)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(ErrorCode::InvalidInput);
        }
    }
    if !m.is_file() || m.len() > max {
        return Err(ErrorCode::InvalidInput);
    }
    let mut data = Vec::new();
    handle
        .take(max + 1)
        .read_to_end(&mut data)
        .map_err(|_| ErrorCode::InvalidInput)?;
    if data.len() as u64 > max {
        return Err(ErrorCode::InvalidInput);
    }
    Ok(data)
}
fn parse_edit(args: &[&str]) -> Result<Edit, ErrorCode> {
    let invalid = ErrorCode::InvalidInput;
    Ok(match args {
        ["mysql-query-policy", t, r, path] => Edit::QueryPolicy {
            target: alias(t)?,
            alias: alias(r)?,
            policy: serde_json::from_slice(&file(path, 65536)?).map_err(|_| invalid)?,
        },
        ["mysql-policy", t, r, path, rest @ ..] => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Policy {
                ips: std::collections::BTreeSet<std::net::IpAddr>,
                tables: TableScope,
            }
            let policy: Policy =
                serde_json::from_slice(&file(path, 65536)?).map_err(|_| invalid)?;
            let ca_pem = match rest {
                [] => None,
                ["--ca", p] => Some(envlume_application::mysql_policy::public_ca(&file(
                    p, 16384,
                )?)?),
                _ => return Err(invalid),
            };
            Edit::MySqlPolicy {
                target: alias(t)?,
                alias: alias(r)?,
                route: MySqlRoute {
                    ips: policy.ips,
                    ca_pem,
                },
                tables: policy.tables,
            }
        }
        ["set", name] => Edit::preset(alias(name)?)?,
        ["set", name, kind, risk] => Edit::Target {
            alias: alias(name)?,
            kind: match *kind {
                "DEV" => EnvironmentKind::Dev,
                "TEST" => EnvironmentKind::Test,
                "STAGING" => EnvironmentKind::Staging,
                "PROD" => EnvironmentKind::Prod,
                _ => return Err(invalid),
            },
            risk: match *risk {
                "LOW" => RiskLevel::Low,
                "MEDIUM" => RiskLevel::Medium,
                "HIGH" => RiskLevel::High,
                "CRITICAL" => RiskLevel::Critical,
                _ => return Err(invalid),
            },
        },
        ["rename", from, to] => Edit::Rename {
            alias: alias(from)?,
            to: alias(to)?,
        },
        ["resource", target, name, host, port, tls, db @ ..] => Edit::Resource {
            target: alias(target)?,
            alias: alias(name)?,
            endpoint: Endpoint {
                host: host.to_ascii_lowercase(),
                port: port.parse().map_err(|_| invalid)?,
                database: match db {
                    [] => None,
                    [name] => Some((*name).into()),
                    _ => return Err(invalid),
                },
                tls: match *tls {
                    "VERIFY_IDENTITY" => TlsMode::VerifyIdentity,
                    "DISABLED" => TlsMode::Disabled,
                    _ => return Err(invalid),
                },
            },
        },
        [verb @ ("grant" | "revoke"), target, name, action, rest @ ..] => {
            let target = alias(target)?;
            let name = alias(name)?;
            let action = RemoteAction::parse(action).ok_or(invalid)?;
            match (*verb, rest) {
                ("revoke", []) => Edit::Revoke {
                    target,
                    alias: name,
                    action,
                },
                ("grant", _) => {
                    let expires_at = match rest {
                        [] => None,
                        ["--ttl", s] => {
                            let seconds =
                                s.parse::<u32>().ok().filter(|n| *n > 0).ok_or(invalid)?;
                            Some(
                                UtcDateTime::now() + std::time::Duration::from_secs(seconds.into()),
                            )
                        }
                        _ => return Err(invalid),
                    };
                    Edit::Grant {
                        target,
                        alias: name,
                        action,
                        expires_at,
                    }
                }
                _ => return Err(invalid),
            }
        }
        _ => return Err(invalid),
    })
}
fn execute(
    args: &[&str],
    json: bool,
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> Result<u8, ErrorCode> {
    let [operation @ .., flag, raw] = args else {
        return Err(ErrorCode::InvalidInput);
    };
    let store = Store::open(locate().map_err(|_| ErrorCode::InternalError)?).map_err(|e| {
        if e == StorageError::StorageBusy {
            ErrorCode::Timeout
        } else {
            ErrorCode::InternalError
        }
    })?;
    let origin = env::current_dir().map_err(|_| ErrorCode::ProjectNotTrusted)?;
    let id = raw.parse().map_err(|_| ErrorCode::InvalidInput)?;
    let binding = match *flag {
        "--binding" => BindingId(id),
        "--project" if operation == ["list"] => {
            let row = inspect_project(&store, &origin)
                .map_err(|_| ErrorCode::ProjectNotTrusted)?
                .binding;
            if row.project_id != ProjectId(id) {
                return Err(ErrorCode::ProjectNotTrusted);
            }
            row.binding_id
        }
        _ => return Err(ErrorCode::InvalidInput),
    };
    let registry = LocalRegistration {
        store: store.clone(),
        origin: origin.clone(),
    };
    if operation == ["list"] {
        let report = super::tasks::Tasks::open(store, binding, Actor::Cli)?
            .environments(serde_json::json!({}));
        if json {
            println!("{}", serde_json::json!(report));
        } else if let Some(Payload::Environments { environments }) = &report.result {
            for e in environments {
                for r in &e.resources {
                    println!("{}\t{}", e.target.as_str(), r.alias.as_str());
                }
            }
        } else {
            eprintln!("{}", report.human_text());
        }
        return Ok(report.exit_code());
    }
    if matches!(
        operation.first(),
        Some(&"mysql-policy" | &"mysql-query-policy")
    ) {
        // Parsing this edit reads local files. Authenticate the origin before opening them.
        Session::from_registration(&registry, binding, Actor::Cli)?;
    }
    environments::manage(
        &registry,
        &store,
        binding,
        &origin,
        parse_edit(operation)?,
        |preview| confirm(preview, "confirm"),
    )?;
    if json {
        println!("{}", serde_json::json!({"schemaVersion":1,"ok":true}));
    } else {
        println!("Environment configuration saved.");
    }
    Ok(0)
}
pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, json) = match args.split_last() {
        Some((f, rest)) if f == "--json" => (rest, true),
        _ => (args, false),
    };
    let result = args
        .iter()
        .map(|s| s.to_str().ok_or(ErrorCode::InvalidInput))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|v| execute(&v, json, locate));
    match result {
        Ok(n) => n.into(),
        Err(code) => {
            let mut report = Report::new(if args.first().is_some_and(|s| s == "list") {
                "environment_list"
            } else {
                "unknown"
            });
            report.fail(code, code.summary());
            if json {
                println!("{}", serde_json::json!(report));
            }
            eprintln!("{}", report.human_text());
            code.exit_code().into()
        }
    }
}
