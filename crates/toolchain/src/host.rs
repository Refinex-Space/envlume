use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct Host {
    pub bins: Vec<PathBuf>,
    pub gui_bins: Vec<PathBuf>,
    pub preferred: BTreeMap<&'static str, Vec<PathBuf>>,
    pub homes: BTreeMap<&'static str, Vec<PathBuf>>,
    pub mise: Option<PathBuf>,
    pub settings: Option<PathBuf>,
    pub excluded: Option<PathBuf>,
}
pub(crate) fn children(path: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .take(256)
        .collect();
    dirs.sort();
    dirs
}
fn absolute_var(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}
impl Host {
    pub fn local() -> Self {
        let home = dirs::home_dir();
        let mut h = Self {
            excluded: env::current_dir()
                .ok()
                .and_then(|p| envlume_workspace::local_canonical(&p).ok()),
            ..Self::default()
        };
        h.bins.extend(
            env::var_os("PATH")
                .iter()
                .flat_map(|p| env::split_paths(p))
                .filter(|p| p.is_absolute()),
        );
        for (tool, key) in [
            ("jdk", "JAVA_HOME"),
            ("maven", "MAVEN_HOME"),
            ("maven", "M2_HOME"),
            ("gradle", "GRADLE_HOME"),
        ] {
            if let Some(p) = absolute_var(key) {
                h.preferred.entry(tool).or_default().push(p);
            }
        }
        // Hosted Windows environments, as well as local multi-JDK installations,
        // often publish separate homes alongside their active JAVA_HOME.
        for v in [8, 11, 17, 21, 25] {
            for arch in ["X64", "ARM64"] {
                if let Some(p) = absolute_var(&format!("JAVA_HOME_{v}_{arch}")) {
                    h.homes.entry("jdk").or_default().push(p);
                }
            }
        }
        let mut roots = Vec::new();
        if let Some(user) = &home {
            h.settings = Some(user.join(".m2/settings.xml"));
            h.mise = absolute_var("MISE_DATA_DIR")
                .or_else(|| {
                    Some(if cfg!(windows) {
                        dirs::data_local_dir()
                            .unwrap_or_else(|| user.clone())
                            .join("mise")
                    } else {
                        user.join(".local/share/mise")
                    })
                })
                .map(|p| p.join("installs"));
            roots.push(("jdk", user.join("Library/Java/JavaVirtualMachines")));
            for (tool, name) in [("jdk", "java"), ("maven", "maven"), ("gradle", "gradle")] {
                roots.push((tool, user.join(".sdkman/candidates").join(name)));
            }
            roots.push(("node", user.join(".nvm/versions/node")));
            roots.push(("python", user.join(".pyenv/versions")));
            let rustup = absolute_var("RUSTUP_HOME").unwrap_or_else(|| user.join(".rustup"));
            if let Some(settings) = h.permitted(&rustup.join("settings.toml"), None)
                && let Some(cfg) =
                    crate::metadata::read(&settings).and_then(|s| s.parse::<toml::Table>().ok())
                && let Some(name) = cfg.get("default_toolchain").and_then(|n| n.as_str())
                && !name.starts_with('.')
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
            {
                for tool in ["rustc", "cargo"] {
                    h.preferred
                        .entry(tool)
                        .or_default()
                        .push(rustup.join("toolchains").join(name));
                }
            }
            roots.push(("rustc", rustup.join("toolchains")));
            roots.push(("cargo", rustup.join("toolchains")));
            h.gui_bins.push(user.join(".local/bin"));
        }
        #[cfg(target_os = "macos")]
        {
            roots.push(("jdk", PathBuf::from("/Library/Java/JavaVirtualMachines")));
            for p in [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/Library/Developer/CommandLineTools/usr/bin",
                "/Applications/Xcode.app/Contents/Developer/usr/bin",
                "/usr/bin",
            ] {
                h.gui_bins.push(p.into());
            }
            for base in ["/opt/homebrew/opt", "/usr/local/opt"] {
                for name in [
                    "openjdk",
                    "openjdk@8",
                    "openjdk@11",
                    "openjdk@17",
                    "openjdk@21",
                    "openjdk@25",
                ] {
                    h.homes.entry("jdk").or_default().push(
                        Path::new(base)
                            .join(name)
                            .join("libexec/openjdk.jdk/Contents/Home"),
                    );
                }
            }
        }
        #[cfg(windows)]
        {
            for key in ["ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(p) = absolute_var(key) {
                    for dir in [
                        "Java",
                        "Eclipse Adoptium",
                        "Microsoft",
                        "Zulu",
                        "Amazon Corretto",
                    ] {
                        roots.push(("jdk", p.join(dir)));
                    }
                    h.gui_bins.extend([
                        p.join("nodejs"),
                        p.join("Git/cmd"),
                        p.join("Docker/Docker/resources/bin"),
                    ]);
                }
            }
            if let Some(p) = absolute_var("NVM_HOME") {
                roots.push(("node", p));
            }
            if let Some(p) = dirs::data_local_dir() {
                roots.push(("python", p.join("Programs/Python")));
            }
            if let Some(p) = dirs::data_dir() {
                h.gui_bins.push(p.join("npm"));
            }
        }
        for (tool, root) in roots {
            if let Some(root) = h.permitted(&root, None) {
                h.homes.entry(tool).or_default().extend(children(&root));
            }
        }
        h
    }
    /// Repository files can request versions, but cannot nominate an executable.
    pub fn permitted(&self, path: &Path, workspace: Option<&Path>) -> Option<PathBuf> {
        if !path.is_absolute() {
            return None;
        }
        let p = envlume_workspace::local_canonical(path).ok()?;
        if workspace
            .into_iter()
            .chain(self.excluded.as_deref())
            .any(|r| p.starts_with(r) || path.starts_with(r))
        {
            return None;
        }
        Some(p)
    }
}
