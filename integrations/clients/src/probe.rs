use envlume_application::{
    integrations::{Adapter, Fault, Launch, Verification},
    workspaces::BindingInfo,
};
use envlume_domain::ClientKind;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use std::{path::Path, process::Stdio, time::Duration};

pub struct LocalAdapter;
impl Adapter for LocalAdapter {
    fn detected(&self, client: ClientKind, root: &Path) -> bool {
        if client == ClientKind::Codex {
            return crate::codex::executable(root).is_some();
        }
        let name = match client {
            ClientKind::ClaudeCode => "claude",
            ClientKind::Cursor => "cursor",
            ClientKind::Codex => "codex",
        };
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&paths)
            .filter(|d| d.is_absolute())
            .any(|d| {
                let exts: &[&str] = if cfg!(windows) {
                    &[".exe", ".cmd", ".bat"]
                } else {
                    &[""]
                };
                exts.iter().any(|ext| {
                    let Ok(path) = d.join(format!("{name}{ext}")).canonicalize() else {
                        return false;
                    };
                    if path.starts_with(root) {
                        return false;
                    }
                    let Ok(m) = path.metadata() else { return false };
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if m.permissions().mode() & 0o111 == 0 {
                            return false;
                        }
                    }
                    m.is_file()
                })
            })
    }
    fn version(&self, client: ClientKind, root: &Path) -> Result<Option<String>, Fault> {
        if client == ClientKind::Codex {
            return crate::codex::version(root).map(Some);
        }
        Ok(None)
    }
    fn verify(
        &self,
        client: ClientKind,
        root: &Path,
        launch: &Launch,
        expected: &BindingInfo,
    ) -> Result<Verification, Fault> {
        let mut cmd = tokio::process::Command::new(&launch.command);
        cmd.args(&launch.args)
            .current_dir(root)
            .kill_on_drop(true)
            .env_clear();
        // The probe communicates over its pipes. It must not attach to a caller's
        // console (particularly ConPTY) or open a console for the desktop host.
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for key in [
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "SystemRoot",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        #[cfg(windows)]
        {
            // The native catalogue checks ACLs using the inbox Windows PowerShell.
            // Supply the OS profile/CLR locations without inheriting client credentials,
            // profile scripts, a user module search path or workspace executables.
            for key in [
                "SystemDrive",
                "ProgramData",
                "ALLUSERSPROFILE",
                "PUBLIC",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "CommonProgramFiles",
                "CommonProgramFiles(x86)",
                "CommonProgramW6432",
                "HOMEDRIVE",
                "HOMEPATH",
                "USERNAME",
                "USERDOMAIN",
                "COMPUTERNAME",
                "PROCESSOR_ARCHITECTURE",
                "NUMBER_OF_PROCESSORS",
                "OS",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    cmd.env(key, value);
                }
            }
            let windows = std::path::PathBuf::from(
                std::env::var_os("SystemRoot").ok_or(Fault::VerificationFailed)?,
            );
            let system = windows.join("System32");
            let powershell = system.join("WindowsPowerShell").join("v1.0");
            cmd.env("WINDIR", &windows)
                .env("ComSpec", system.join("cmd.exe"))
                .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
                .env("PSModulePath", powershell.join("Modules"))
                .env(
                    "PATH",
                    std::env::join_paths([system, powershell])
                        .map_err(|_| Fault::VerificationFailed)?,
                );
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Fault::Storage)?;
        super::trace("runtime_ready");
        let result = rt.block_on(async {
            let mut child = cmd.spawn().map_err(|_| Fault::CliUnavailable)?;
            super::trace("child_started");
            let stdout = child.stdout.take().expect("piped stdout");
            let stdin = child.stdin.take().expect("piped stdin");
            let probe = tokio::time::timeout(Duration::from_secs(15), async move {
                let service =
                    ().serve((stdout, stdin))
                        .await
                        .map_err(|_| Fault::VerificationFailed)?;
                super::trace("initialized");
                let reply = service
                    .call_tool(CallToolRequestParams::new("project_context"))
                    .await
                    .map_err(|_| Fault::VerificationFailed)?;
                super::trace("tool_replied");
                let r = reply
                    .structured_content
                    .as_ref()
                    .ok_or(Fault::VerificationFailed)?;
                let valid = reply.is_error != Some(true)
                    && r["ok"] == true
                    && r["operation"] == "project_context"
                    && r["bindingAlias"] == expected.alias
                    && r["result"]["workspace"] == expected.alias
                    && r["result"]["project"] == expected.project;
                service
                    .cancel()
                    .await
                    .map_err(|_| Fault::VerificationFailed)?;
                if valid {
                    Ok(Verification::Gateway)
                } else {
                    Err(Fault::VerificationFailed)
                }
            })
            .await
            .map_err(|_| Fault::Timeout);
            // Keep ownership through shutdown, including a cancelled handshake. Exit
            // only after the probe has released the executable and workspace handles.
            super::trace("transport_closed");
            let status = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(status) => status.map_err(|_| Fault::Storage)?,
                Err(_) => {
                    child.kill().await.map_err(|_| Fault::Storage)?;
                    return Err(Fault::Timeout);
                }
            };
            super::trace("child_exited");
            let verdict = probe??;
            if !status.success() {
                return Err(Fault::VerificationFailed);
            }
            Ok(verdict)
        });
        drop(rt);
        super::trace("runtime_closed");
        let verdict = result?;
        if client == ClientKind::Codex {
            crate::codex::verify(root, launch)?;
            return Ok(Verification::ProjectConfig);
        }
        Ok(verdict)
    }
}
