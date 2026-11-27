use envlume_application::{
    integrations::{Adapter, Fault, Launch, Verification},
    workspaces::BindingInfo,
};
use envlume_domain::ClientKind;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use std::{path::Path, process::Stdio, time::Duration};

pub struct LocalAdapter;
impl Adapter for LocalAdapter {
    fn detected(&self, client: ClientKind, root: &Path) -> bool {
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
    fn verify(
        &self,
        _: ClientKind,
        root: &Path,
        launch: &Launch,
        expected: &BindingInfo,
    ) -> Result<Verification, Fault> {
        let mut cmd = tokio::process::Command::new(&launch.command);
        cmd.args(&launch.args)
            .current_dir(root)
            .kill_on_drop(true)
            .env_clear();
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
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Fault::Storage)?;
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(15), async {
                let (transport, _) = TokioChildProcess::builder(cmd)
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|_| Fault::CliUnavailable)?;
                let service = ().serve(transport).await.map_err(|_| Fault::VerificationFailed)?;
                let reply = service
                    .call_tool(CallToolRequestParams::new("project_context"))
                    .await
                    .map_err(|_| Fault::VerificationFailed)?;
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
            .map_err(|_| Fault::Timeout)?
        })
    }
}
