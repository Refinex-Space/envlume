use super::registration;
use envlume_application::{
    ErrorCode,
    projects::StorageError,
    result::Payload,
    workspaces::{ManagementError, inspect_project},
};
use envlume_domain::{Alias, BindingId, ToolRule};
use envlume_storage::{Store, StorePaths};
use envlume_toolchain::{LocalToolchain, TOOLS, mise_rules, valid_requirement};
use serde_json::json;
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::ExitCode,
};

type Failure = (&'static str, &'static str, u8);
const INPUT: Failure = (
    "INVALID_INPUT",
    "Check doctor/profile arguments, tool names, absolute paths and numeric version constraints.",
    2,
);
fn text(s: &OsStr) -> Result<&str, Failure> {
    s.to_str().ok_or(INPUT)
}
fn id(s: &OsStr) -> Result<BindingId, Failure> {
    text(s)?.parse().map(BindingId).map_err(|_| INPUT)
}
fn app_error(err: ErrorCode) -> Failure {
    ("DOCTOR_UNAVAILABLE", err.summary(), err.exit_code())
}
fn storage(err: StorageError) -> Failure {
    ManagementError::Storage(err).diagnostic()
}
fn execute(
    args: &[OsString],
    json_mode: bool,
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> Result<u8, Failure> {
    match args {
        [cmd] if cmd == "doctor" => {
            let report = LocalToolchain::host_doctor().map_err(app_error)?;
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&report)
                        .map_err(|_| app_error(ErrorCode::InternalError))?
                );
            } else {
                for t in report.tools {
                    println!(
                        "{}\t{}\t{}\t{} {}",
                        t.tool,
                        t.status,
                        t.version.as_deref().unwrap_or("-"),
                        t.source,
                        t.details.join("; ")
                    );
                    if let Some(action) = t.repair {
                        println!("  {action}");
                    }
                }
            }
            Ok(0)
        }
        [cmd, flag, binding, rest @ ..] if cmd == "doctor" && flag == "--binding" => {
            let profile = match rest {
                [] => "default",
                [flag, name] if flag == "--profile" => text(name)?,
                _ => return Err(INPUT),
            };
            let session = registration::session(locate(), id(binding)?).map_err(app_error)?;
            let report = session.toolchain_doctor(json!({"profile":profile}), &LocalToolchain);
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&report)
                        .map_err(|_| app_error(ErrorCode::InternalError))?
                );
            } else if let Some(Payload::Doctor { tools }) = &report.result {
                for t in tools {
                    println!(
                        "{}\t{}\t{}",
                        t.tool.as_str(),
                        t.version.as_deref().unwrap_or("-"),
                        t.path_summary.as_deref().unwrap_or("-")
                    );
                    if let Some(repair) = &t.repair {
                        println!("  {repair}");
                    }
                }
            } else {
                eprintln!("{}", report.summary);
            }
            Ok(report.exit_code())
        }
        [cmd, sub, flag, binding, flag2, name, tail @ ..]
            if cmd == "toolchain"
                && sub == "configure"
                && flag == "--binding"
                && flag2 == "--profile" =>
        {
            let id = id(binding)?;
            let name = Alias::try_from(text(name)?.to_owned()).map_err(|_| INPUT)?;
            let store = Store::open(locate().map_err(storage)?).map_err(storage)?;
            let root = std::env::current_dir().map_err(|_| INPUT)?;
            let inspected = inspect_project(&store, &root).map_err(|e| e.diagnostic())?;
            if inspected.binding.binding_id != id || inspected.binding.state != "TRUSTED" {
                return Err(ManagementError::WorkspaceChanged.diagnostic());
            }
            let mut rules: Vec<ToolRule> = Vec::new();
            let mut import = false;
            let mut rest = tail;
            while !rest.is_empty() {
                match rest {
                    [flag, more @ ..] if flag == "--mise" && !import => {
                        import = true;
                        rest = more;
                    }
                    [flag, tool, req, more @ ..] if flag == "--tool" => {
                        let t = text(tool)?;
                        let req = text(req)?;
                        if !TOOLS.contains(&t)
                            || !valid_requirement(req)
                            || rules.iter().any(|r| r.tool.as_str() == t)
                        {
                            return Err(INPUT);
                        }
                        let mut selected_path = None;
                        rest = more;
                        if let [flag, path, next @ ..] = more
                            && flag == "--path"
                        {
                            let p = PathBuf::from(path);
                            if !p.is_absolute() || p.to_str().is_none() {
                                return Err(INPUT);
                            }
                            let p = envlume_workspace::entry_path(&p).map_err(|_| INPUT)?;
                            if p.starts_with(&inspected.binding.canonical_path)
                                || envlume_workspace::local_canonical(&p)
                                    .is_ok_and(|p| p.starts_with(&inspected.binding.canonical_path))
                            {
                                return Err(INPUT);
                            }
                            selected_path = Some(p);
                            rest = next;
                        }
                        rules.push(ToolRule {
                            tool: Alias::try_from(t.to_owned()).map_err(|_| INPUT)?,
                            version_requirement: req.to_owned(),
                            selected_path,
                            priority: 0,
                        });
                    }
                    _ => return Err(INPUT),
                }
            }
            if import {
                for rule in mise_rules(&inspected.binding.canonical_path)
                    .map_err(|msg| ("INVALID_INPUT", msg, 2))?
                {
                    if !rules.iter().any(|r| r.tool == rule.tool) {
                        rules.push(rule);
                    }
                }
            }
            rules.sort_by(|a, b| a.tool.as_str().cmp(b.tool.as_str()));
            let mut prompt = format!(
                "Replace profile {} for workspace {}? Grant toolchain.inspect to this binding.",
                name.as_str(),
                inspected.binding.alias
            );
            for r in &rules {
                prompt.push_str(&format!(
                    "\n  {} {} ({})",
                    r.tool.as_str(),
                    r.version_requirement,
                    if r.priority == envlume_toolchain::MISE_PRIORITY {
                        "approved mise pin"
                    } else {
                        "local rule"
                    }
                ));
                if let Some(p) = &r.selected_path {
                    prompt.push_str(&format!(" {p:?}"));
                }
            }
            if !registration::confirm(&prompt, "configure") {
                return Err(ManagementError::ConfirmationRequired.diagnostic());
            }
            let profile = store
                .configure_toolchain(id, &root, &name, &rules)
                .map_err(|e| e.diagnostic())?;
            if json_mode {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"profileId":profile,"profile":name,"bindingId":id})
                );
            } else {
                println!("Selected profile {} ({})", name.as_str(), profile.0);
            }
            Ok(0)
        }
        _ => Err(INPUT),
    }
}
pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, machine) = match args.split_last() {
        Some((last, rest)) if last == "--json" => (rest, true),
        _ => (args, false),
    };
    match execute(args, machine, locate) {
        Ok(code) => ExitCode::from(code),
        Err((code, summary, exit)) => {
            if machine {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"errorCode":code,"summary":summary})
                );
            }
            eprintln!("{summary}");
            ExitCode::from(exit)
        }
    }
}
