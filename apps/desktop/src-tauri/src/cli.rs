//! Trusted native entry points; never accept a source image or path over WebView IPC.
use envlume_storage::{
    StorePaths,
    cli_install::{self, InstallError, InstalledCli},
};
use std::{ffi::OsString, process::ExitCode};

#[cfg(feature = "bundled-cli")]
pub fn install(paths: &StorePaths, automatic: bool) -> Result<InstalledCli, InstallError> {
    let release: cli_install::Release =
        serde_json::from_str(include_str!(env!("ENVLUME_BUNDLE_RELEASE")))
            .map_err(|_| InstallError::InvalidRelease)?;
    if release.max_schema != envlume_storage::SCHEMA_VERSION {
        return Err(InstallError::InvalidRelease);
    }
    cli_install::install(
        paths,
        &release,
        &include_bytes!(env!("ENVLUME_BUNDLE_CLI"))[..],
        automatic,
    )
}

#[cfg(not(feature = "bundled-cli"))]
pub fn install(_: &StorePaths, _: bool) -> Result<InstalledCli, InstallError> {
    Err(InstallError::InvalidRelease)
}

pub fn maintenance(args: &[OsString]) -> ExitCode {
    let op = match args {
        [op] => op.to_str(),
        _ => None,
    };
    if !matches!(op, Some("install" | "locate" | "restore")) {
        println!(
            "{}",
            serde_json::json!({"ok":false,"errorCode":"INVALID_INPUT","summary":"Use --cli install, locate or restore."})
        );
        return ExitCode::from(2);
    }
    let result = StorePaths::local()
        .map_err(InstallError::from)
        .and_then(|p| match op {
            Some("install") => install(&p, false),
            Some("locate") => cli_install::locate(&p),
            Some("restore") => cli_install::restore(&p),
            _ => unreachable!("validated maintenance operation"),
        });
    match result {
        Ok(cli) => {
            println!("{}", serde_json::json!({"ok":true,"cli":cli}));
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({"ok":false,"errorCode":e,"summary":e.message()})
            );
            ExitCode::FAILURE
        }
    }
}
