//! Inspect Codex's effective configuration without starting a project thread.
use crate::native;
use envlume_application::integrations::{Fault, Launch};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// Refuse aliases of the user configuration before any preview, backup or publish.
pub(super) fn ensure_project_scope(root: &Path) -> Result<(), Fault> {
    let default_profile = dirs::home_dir().ok_or(Fault::UnsafePath)?.join(".codex");
    let selected = std::env::var_os("CODEX_HOME").map(PathBuf::from);
    for profile in std::iter::once(default_profile).chain(selected) {
        if !profile.is_absolute() {
            return Err(Fault::UnsafePath);
        }
        let resolved = match envlume_workspace::local_canonical(&profile) {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let parent = profile.parent().ok_or(Fault::UnsafePath)?;
                envlume_workspace::local_canonical(parent)
                    .map_err(|_| Fault::UnsafePath)?
                    .join(profile.file_name().ok_or(Fault::UnsafePath)?)
            }
            Err(_) => return Err(Fault::UnsafePath),
        };
        if resolved.starts_with(root) {
            return Err(Fault::ScopeConflict);
        }
    }
    Ok(())
}

pub(crate) fn executable(root: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("windows", "x86_64") => "win32-x64",
        _ => return None,
    };
    let vendor = PathBuf::from(format!("@openai/codex-{platform}/vendor"))
        .join(envlume_storage::cli_install::native_target())
        .join("bin")
        .join(name);
    for dir in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
        if let Some(exe) = native::native_file(&dir.join(name), root) {
            return Some(exe);
        }
        let modules = if cfg!(windows) {
            dir.join("node_modules")
        } else {
            let Some(prefix) = dir.parent() else { continue };
            prefix.join("lib/node_modules")
        };
        for base in [modules.clone(), modules.join("@openai/codex/node_modules")] {
            if let Some(exe) = native::native_file(&base.join(&vendor), root) {
                return Some(exe);
            }
        }
    }
    None
}

pub(crate) fn version(root: &Path) -> Result<String, Fault> {
    let exe = executable(root).ok_or(Fault::ClientUnavailable)?;
    let mut cmd = native::command(&exe, root)?;
    cmd.arg("--version");
    let text = native::output(cmd)?;
    text.trim()
        .strip_prefix("codex-cli ")
        .and_then(|v| semver::Version::parse(v).ok())
        .map(|v| v.to_string())
        .ok_or(Fault::ClientUnsupported)
}

fn project_layer(name: &Value, root: &Path) -> bool {
    name["type"] == "project"
        && name["dotCodexFolder"].as_str().is_some_and(|path| {
            Path::new(path).is_absolute()
                && envlume_workspace::local_canonical(Path::new(path))
                    .ok()
                    .is_some_and(|actual| actual == root.join(".codex"))
        })
}

fn check(config: &Value, root: &Path, launch: &Launch) -> Result<(), Fault> {
    let layers = config["layers"]
        .as_array()
        .ok_or(Fault::ClientUnsupported)?;
    let layer = layers
        .iter()
        .find(|l| project_layer(&l["name"], root))
        .ok_or(Fault::CodexProjectNotLoaded)?;
    if !layer["disabledReason"].is_null() {
        return Err(Fault::CodexProjectNotLoaded);
    }
    let registered = &layer["config"]["mcp_servers"]["envlume"];
    if serde_json::from_value::<Launch>(registered.clone())
        .as_ref()
        .ok()
        != Some(launch)
    {
        return Err(Fault::CodexProjectNotLoaded);
    }
    let effective = &config["config"]["mcp_servers"]["envlume"];
    let mut descriptor = effective
        .as_object()
        .cloned()
        .ok_or(Fault::CodexProjectNotLoaded)?;
    if descriptor.remove("enabled") != Some(json!(true))
        || descriptor
            .remove("environment_id")
            .is_some_and(|v| v != "local")
        || descriptor
            .remove("tool_timeout_sec")
            .is_some_and(|v| !v.is_null())
        || serde_json::from_value::<Launch>(Value::Object(descriptor))
            .as_ref()
            .ok()
            != Some(launch)
    {
        return Err(Fault::CodexConfigConflict);
    }
    let origins = config["origins"]
        .as_object()
        .ok_or(Fault::ClientUnsupported)?;
    let mut fields = vec!["mcp_servers.envlume.command".to_owned()];
    fields.extend((0..launch.args.len()).map(|i| format!("mcp_servers.envlume.args.{i}")));
    if !fields
        .iter()
        .all(|k| origins.get(k).is_some_and(|o| o["name"] == layer["name"]))
        || origins
            .iter()
            .any(|(k, o)| k.starts_with("mcp_servers.envlume.") && o["name"] != layer["name"])
    {
        return Err(Fault::CodexConfigConflict);
    }
    Ok(())
}

pub(super) fn verify(root: &Path, launch: &Launch) -> Result<(), Fault> {
    let exe = executable(root).ok_or(Fault::ClientUnavailable)?;
    let mut cmd = native::command(&exe, root)?;
    if let Some(profile) = std::env::var_os("CODEX_HOME") {
        cmd.env("CODEX_HOME", profile);
    }
    cmd.args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Fault::Storage)?;
    rt.block_on(async {
        let mut child = cmd.spawn().map_err(|_| Fault::ClientUnavailable)?;
        let mut input = child.stdin.take().ok_or(Fault::Storage)?;
        // The entire stream is bounded, including notifications and a missing newline.
        let mut lines = BufReader::new(
            child
                .stdout
                .take()
                .ok_or(Fault::Storage)?
                .take(4 * 1024 * 1024),
        )
        .lines();
        let attempt = tokio::time::timeout(Duration::from_secs(15), async {
            for (id, method, params) in [
                (
                    1,
                    "initialize",
                    json!({"clientInfo":{"name":"envlume","version":envlume_application::VERSION}}),
                ),
                (2, "config/read", json!({"cwd":root,"includeLayers":true})),
            ] {
                let message = json!({"id":id,"method":method,"params":params}).to_string() + "\n";
                input
                    .write_all(message.as_bytes())
                    .await
                    .map_err(|_| Fault::ClientRejected)?;
                loop {
                    let line = lines
                        .next_line()
                        .await
                        .map_err(|_| Fault::ClientRejected)?
                        .ok_or(Fault::ClientRejected)?;
                    let msg: Value =
                        serde_json::from_str(&line).map_err(|_| Fault::ClientRejected)?;
                    if msg["id"] != id {
                        continue;
                    }
                    let result = msg.get("result").ok_or(Fault::ClientRejected)?;
                    if id == 2 {
                        return check(result, root, launch);
                    }
                    break;
                }
            }
            Err(Fault::ClientRejected)
        })
        .await
        .unwrap_or(Err(Fault::Timeout));
        drop(input);
        let exit = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
            Ok(status) => status.map_err(|_| Fault::Storage).and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(Fault::ClientRejected)
                }
            }),
            Err(_) => {
                child.kill().await.map_err(|_| Fault::Storage)?;
                Err(Fault::Timeout)
            }
        };
        attempt.and(exit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_options_and_user_sourced_arguments_cannot_attest_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let folder = root.join(".codex");
        std::fs::create_dir(&folder).unwrap();
        let project = json!({"type":"project","dotCodexFolder":folder});
        let launch = Launch {
            command: r"C:\Program Files\Envlume\envlume.exe".into(),
            args: vec!["mcp".into(), "--binding".into(), "fixture".into()],
        };
        let mut cfg = json!({
            "layers":[{"name":project,"disabledReason":null,"config":{"mcp_servers":{"envlume":launch}}}],
            "config":{"mcp_servers":{"envlume":{"command":launch.command,"args":launch.args,"enabled":true,"environment_id":"local","tool_timeout_sec":null}}},
            "origins":{}
        });
        for suffix in ["command", "args.0", "args.1", "args.2"] {
            cfg["origins"][format!("mcp_servers.envlume.{suffix}")] = json!({"name":project});
        }
        assert_eq!(check(&cfg, &root, &launch), Ok(()));
        let mut inherited = cfg.clone();
        inherited["config"]["mcp_servers"]["envlume"]["env"] = json!({"SYNTHETIC":"fixture"});
        assert_eq!(
            check(&inherited, &root, &launch),
            Err(Fault::CodexConfigConflict)
        );
        cfg["origins"]["mcp_servers.envlume.args.2"]["name"] = json!({"type":"user"});
        assert_eq!(check(&cfg, &root, &launch), Err(Fault::CodexConfigConflict));
    }
}
