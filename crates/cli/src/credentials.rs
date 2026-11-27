use super::registration::confirm;
use envlume_application::{
    credentials::{self, Change, Fault},
    projects::StorageError,
};
use envlume_domain::{Alias, BindingId};
use envlume_secrets::{Login, NativeVault};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use serde_json::{Value, json};
use std::{env, ffi::OsString, process::ExitCode};
use zeroize::Zeroizing;

fn execute(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> Result<Value, Fault> {
    let text = args
        .iter()
        .map(|s| s.to_str().ok_or(Fault::InvalidInput))
        .collect::<Result<Vec<_>, _>>()?;
    let [operation @ .., "--binding", raw] = text.as_slice() else {
        return Err(Fault::InvalidInput);
    };
    let binding = BindingId(raw.parse().map_err(|_| Fault::InvalidInput)?);
    let paths = locate().map_err(|_| Fault::Storage)?;
    let store = Store::open(paths.clone()).map_err(|e| {
        if e == StorageError::StorageBusy {
            Fault::Busy
        } else {
            Fault::Storage
        }
    })?;
    let os = NativeVault::for_catalogue(paths.root())?;
    let origin = env::current_dir().map_err(|_| Fault::Untrusted)?;
    if operation == ["cleanup"] {
        if !confirm(
            "Retry deletion of detached Envlume credentials for this binding's project?",
            "cleanup",
        ) {
            return Err(Fault::AccessDenied);
        }
        os.cleanup(&store, binding, &origin)?;
        return Ok(json!({"schemaVersion":1,"ok":true,"state":"CLEANED"}));
    }
    let [name, target, resource] = operation else {
        return Err(Fault::InvalidInput);
    };
    let alias = |s: &str| Alias::try_from(s.to_owned()).map_err(|_| Fault::InvalidInput);
    let selection = credentials::select(
        &LocalRegistration {
            store: store.clone(),
            origin,
        },
        binding,
        &alias(target)?,
        &alias(resource)?,
    )?;
    if *name == "status" {
        let state = match os.status(&store, &selection) {
            Ok(()) => "AVAILABLE",
            Err(Fault::NotSet) => "NOT_SET",
            Err(e) => return Err(e),
        };
        return Ok(json!({"schemaVersion":1,"ok":true,"state":state}));
    }
    let action = match *name {
        "set" => Change::Set,
        "replace" => Change::Replace,
        "repair" => Change::Repair,
        "clear" => Change::Clear,
        "remove-resource" => Change::RemoveResource,
        _ => return Err(Fault::InvalidInput),
    };
    let r = selection.resource();
    let prompt = format!(
        "Confirm {name} for project {}, binding {}, target {} ({:?}, {:?}), MySQL resource {} at {}:{} ({:?}, database {:?}).",
        selection.project().0,
        binding.0,
        target,
        selection.target().kind,
        selection.target().risk,
        resource,
        r.host,
        r.port,
        r.tls,
        r.database
    );
    if !confirm(&prompt, "confirm") {
        return Err(Fault::AccessDenied);
    }
    let input = if matches!(action, Change::Set | Change::Replace | Change::Repair) {
        let user = Zeroizing::new(
            rpassword::prompt_password("Database username (hidden): ")
                .map_err(|_| Fault::AccessDenied)?,
        );
        let password = Zeroizing::new(
            rpassword::prompt_password("Database password (hidden): ")
                .map_err(|_| Fault::AccessDenied)?,
        );
        Some(Login::new(&user, &password)?)
    } else {
        None
    };
    let saved = os.change(&store, &selection, action, input.as_ref())?;
    let state = match action {
        Change::RemoveResource => "RESOURCE_DISABLED",
        Change::Clear => "NOT_SET",
        _ => "AVAILABLE",
    };
    Ok(json!({"schemaVersion":1,"ok":true,"state":state,"cleanupPending":saved.cleanup_pending}))
}
pub fn run(
    args: &[OsString],
    locate: impl FnOnce() -> Result<StorePaths, StorageError>,
) -> ExitCode {
    let (args, machine) = match args.split_last() {
        Some((f, rest)) if f == "--json" => (rest, true),
        _ => (args, false),
    };
    match execute(args, locate) {
        Ok(v) => {
            if machine {
                println!("{v}");
            } else {
                println!("{}", v["state"].as_str().unwrap_or_default());
            }
            if v["cleanupPending"] == true {
                eprintln!("Change saved. Retry detached item cleanup with credential cleanup.");
            }
            ExitCode::SUCCESS
        }
        Err(fault) => {
            let code = fault.code();
            if machine {
                println!(
                    "{}",
                    json!({"schemaVersion":1,"ok":false,"errorCode":code,"state":fault,"summary":fault.hint()})
                );
            }
            eprintln!("{}", fault.hint());
            code.exit_code().into()
        }
    }
}
