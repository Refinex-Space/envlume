//! Only native Claude executables outside the registered workspace are launched.
//! Discovery uses a disposable profile, so the client's own project-key resolution
//! can be checked before offering to change a user's project-local configuration.
use envlume_application::integrations::{Fault, Launch};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Client {
    pub exe: PathBuf,
    pub version: String,
}

fn executable(root: &Path) -> Option<PathBuf> {
    let mut bins = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
            #[cfg(windows)]
            {
                bins.push(dir.join("claude.exe"));
                bins.push(dir.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
            }
            #[cfg(not(windows))]
            bins.push(dir.join("claude"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        bins.push(home.join(".local/bin").join(if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        }));
    }
    bins.into_iter()
        .find_map(|p| crate::native::native_file(&p, root))
}

pub(crate) fn profile_path(root: &Path) -> Result<PathBuf, Fault> {
    let parent = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .ok_or(Fault::UnsafePath)?;
    if !parent.is_absolute() {
        return Err(Fault::UnsafePath);
    }
    let parent = envlume_workspace::local_canonical(&parent).map_err(|_| Fault::UnsafePath)?;
    if parent.starts_with(root) || !parent.is_dir() {
        return Err(Fault::UnsafePath);
    }
    Ok(parent.join(".claude.json"))
}

fn output(exe: &Path, root: &Path, profile: Option<&Path>, args: &[&str]) -> Result<String, Fault> {
    let mut cmd = crate::native::command(exe, root)?;
    cmd.args(args);
    if let Some(dir) = profile {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }
    cmd.env("LC_ALL", "C")
        .env("NO_COLOR", "1")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("CLAUDE_CODE_DISABLE_CLAUDE_AI_MCP", "1");
    crate::native::output(cmd)
}
impl Client {
    pub fn available(root: &Path) -> bool {
        executable(root).is_some()
    }
    pub fn detect(root: &Path) -> Result<Self, Fault> {
        let exe = executable(root).ok_or(Fault::ClientUnavailable)?;
        let profile = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let text = output(&exe, root, profile.as_deref(), &["--version"])?;
        let version = text
            .trim()
            .strip_suffix(" (Claude Code)")
            .and_then(|v| semver::Version::parse(v).ok())
            .filter(|v| v.major == 2)
            .ok_or(Fault::ClientUnsupported)?;
        Ok(Self {
            exe,
            version: version.to_string(),
        })
    }
    pub fn project_key(&self, root: &Path, tmp: &Path) -> Result<String, Fault> {
        let scratch = tempfile::tempdir_in(tmp).map_err(|_| Fault::Storage)?;
        output(
            &self.exe,
            root,
            Some(scratch.path()),
            &[
                "mcp",
                "add-json",
                "--scope",
                "local",
                "envlume-scope-check",
                r#"{"command":"envlume-scope-check","args":[]}"#,
            ],
        )
        .map_err(|_| Fault::ClientUnsupported)?;
        let data = crate::file::Target::profile(scratch.path(), None)?
            .0
            .read()?
            .ok_or(Fault::ClientUnsupported)?;
        let state: serde_json::Value =
            serde_json::from_slice(&data).map_err(|_| Fault::ClientUnsupported)?;
        let projects = state["projects"]
            .as_object()
            .ok_or(Fault::ClientUnsupported)?;
        if projects.len() != 1 {
            return Err(Fault::ClientUnsupported);
        }
        let (key, project) = projects.iter().next().unwrap();
        if project["mcpServers"]["envlume-scope-check"]["command"] != "envlume-scope-check" {
            return Err(Fault::ClientUnsupported);
        }
        if envlume_workspace::local_canonical(Path::new(key))
            .ok()
            .as_deref()
            != Some(root)
        {
            return Err(Fault::ProjectScopeMismatch);
        }
        Ok(key.clone())
    }
    pub fn verify_local(&self, root: &Path, launch: &Launch) -> Result<(), Fault> {
        if launch.command.contains(['\n', '\r']) || launch.command.contains("${") {
            return Err(Fault::UnsafePath);
        }
        let profile = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let text = output(
            &self.exe,
            root,
            profile.as_deref(),
            &["mcp", "get", "envlume"],
        )?;
        let field = |key: &str| {
            let mut fields = text
                .lines()
                .filter_map(|l| l.trim().strip_prefix(key))
                .map(str::trim);
            fields.next().filter(|_| fields.next().is_none())
        };
        if text.lines().next() != Some("envlume:")
            || field("Scope:") != Some("Local config (private to you in this project)")
            || field("Type:") != Some("stdio")
            || field("Command:") != Some(&launch.command)
            || field("Args:") != Some(launch.args.join(" ").as_str())
        {
            return Err(Fault::ClientRejected);
        }
        if field("Status:").is_some_and(|s| s.contains("Disabled for this project")) {
            return Err(Fault::ClientDisabled);
        }
        if !matches!(
            field("Status:"),
            Some("✔ Connected" | "✓ Connected" | "√ Connected" | "Connected")
        ) {
            return Err(Fault::ClientRejected);
        }
        Ok(())
    }
}
