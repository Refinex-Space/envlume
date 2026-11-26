//! Offline tool discovery. Private executable paths stay in the local result for
//! the task runner; only the diagnostic projection is serialized.
mod host;
mod metadata;
mod probe;
mod versions;
use envlume_application::{ErrorCode, ports::ToolchainInspector, result::ToolDiagnostic};
use envlume_domain::{Alias, ToolRule, ToolchainProfile};
use host::{Host, children};
use metadata::{Hints, prop, read};
pub use metadata::{MISE_PRIORITY, mise_rules};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
pub use versions::valid_requirement;

pub const TOOLS: &[&str] = &[
    "jdk", "maven", "gradle", "node", "npm", "pnpm", "yarn", "python", "uv", "git", "rustc",
    "cargo", "docker", "mise",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub tool: &'static str,
    pub source: &'static str,
    pub constraint_source: &'static str,
    pub requirement: String,
    pub version: Option<String>,
    pub path_summary: Option<String>,
    pub status: &'static str,
    pub details: Vec<String>,
    pub repair: Option<String>,
    #[serde(skip)]
    pub executable: Option<PathBuf>,
    #[serde(skip)]
    pub java_home: Option<PathBuf>,
}
impl Selection {
    fn projection(&self) -> ToolDiagnostic {
        let mut summary = format!(
            "{}; constraint {} ({}); {}",
            self.source, self.requirement, self.constraint_source, self.status
        );
        if let Some(path) = &self.path_summary {
            summary.push_str(&format!("; {path}"));
        }
        for detail in &self.details {
            summary.push_str("; ");
            summary.push_str(detail);
        }
        ToolDiagnostic {
            tool: Alias::try_from(self.tool.to_owned()).unwrap(),
            version: self.version.clone(),
            path_summary: Some(summary),
            repair: self.repair.clone(),
        }
    }
    fn problem(&mut self, status: &'static str, repair: &str) {
        self.status = status;
        self.repair = Some(repair.into());
        self.executable = None;
        self.java_home = None;
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub schema_version: u8,
    pub tools: Vec<Selection>,
}
pub struct LocalToolchain;

fn names(tool: &str) -> Vec<String> {
    let base = match tool {
        "jdk" => "java",
        "maven" => "mvn",
        "python" if !cfg!(windows) => "python3",
        t => t,
    };
    if cfg!(windows) {
        let mut names = vec![format!("{base}.exe")];
        if matches!(tool, "maven" | "gradle" | "npm" | "pnpm" | "yarn") {
            names.extend([format!("{base}.cmd"), format!("{base}.bat")]);
        }
        names
    } else if tool == "python" {
        vec!["python3".into(), "python".into()]
    } else {
        vec![base.into()]
    }
}
fn executable(path: &Path) -> bool {
    let Ok(m) = fs::metadata(path) else {
        return false;
    };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
fn matches(tool: &str, actual: &str, req: &str) -> bool {
    let actual = if tool == "jdk" {
        actual.strip_prefix("1.").unwrap_or(actual)
    } else {
        actual
    };
    versions::matches(actual, req)
}
fn vendor(s: Option<&str>) -> &'static str {
    let v = s.unwrap_or("").to_ascii_lowercase();
    for (key, name) in [
        ("azul", "Azul"),
        ("adoptium", "Temurin"),
        ("adoptopenjdk", "AdoptOpenJDK"),
        ("oracle", "Oracle"),
        ("microsoft", "Microsoft"),
        ("amazon", "Corretto"),
        ("bellsoft", "Liberica"),
        ("homebrew", "Homebrew"),
        ("red hat", "Red Hat"),
        ("jetbrains", "JetBrains"),
        ("openjdk", "OpenJDK"),
    ] {
        if v.contains(key) {
            return name;
        }
    }
    "unknown"
}
fn arch(s: Option<&str>) -> &'static str {
    match s.unwrap_or("") {
        "aarch64" | "arm64" => "arm64",
        "amd64" | "x86_64" | "x64" => "x64",
        "i386" | "x86" | "i586" => "x86",
        _ => "unknown",
    }
}

struct Found {
    exe: PathBuf,
    version: Option<String>,
    details: Vec<String>,
    home: Option<PathBuf>,
    broken: Option<&'static str>,
}
fn package_version(exe: &Path, launcher: &Path, tool: &str) -> Option<String> {
    let bin = launcher.parent()?;
    let base = bin.parent()?;
    let mut manifests = vec![
        bin.join("node_modules").join(tool).join("package.json"),
        base.join("lib/node_modules")
            .join(tool)
            .join("package.json"),
    ];
    manifests.extend(
        exe.ancestors()
            .skip(1)
            .take(3)
            .map(|p| p.join("package.json")),
    );
    for p in manifests {
        if let Some(pkg) = read(&p).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            && pkg["name"].as_str() == Some(tool)
            && let Some(v) = pkg["version"].as_str().and_then(versions::version)
        {
            return Some(v.to_owned());
        }
    }
    None
}

fn inspect(
    host: &Host,
    tool: &str,
    launcher: &Path,
    workspace: Option<&Path>,
    cwd: &Path,
    requirement: &str,
    deadline: std::time::Instant,
) -> Option<Found> {
    let exe = host.permitted(launcher, workspace)?;
    if !names(tool).iter().any(|n| {
        launcher
            .file_name()
            .is_some_and(|f| f.to_str().is_some_and(|f| f.eq_ignore_ascii_case(n)))
    }) && tool != "python"
    {
        return None;
    }
    if !executable(&exe) {
        return None;
    }
    let mut f = Found {
        exe: exe.clone(),
        version: None,
        details: Vec::new(),
        home: None,
        broken: None,
    };
    match tool {
        "jdk" => {
            let home = exe.parent()?.parent()?;
            let release = read(&home.join("release")).unwrap_or_default();
            let declared = prop(&release, "JAVA_VERSION").and_then(versions::version);
            f.version = declared.map(str::to_owned);
            f.details.push(format!(
                "vendor {}; arch {}",
                vendor(prop(&release, "IMPLEMENTOR").or_else(|| prop(&release, "JAVA_VENDOR"))),
                arch(prop(&release, "OS_ARCH"))
            ));
            let javac = home
                .join("bin")
                .join(if cfg!(windows) { "javac.exe" } else { "javac" });
            // Don't launch nonmatching JDKs just to rediscover their release version.
            if declared.is_some_and(|v| !matches(tool, v, requirement)) {
                return Some(f);
            }
            let java = probe::installed_version(&exe, tool, cwd, deadline);
            let compiler = host
                .permitted(&javac, workspace)
                .filter(|p| p.starts_with(home))
                .and_then(|p| probe::installed_version(&p, "javac", cwd, deadline));
            if let Some(v) = &java {
                f.details.push(format!("java {v}"));
            }
            if let Some(v) = &compiler {
                f.details.push(format!("javac {v}"));
            }
            if java.is_none() || compiler.is_none() {
                f.broken =
                    Some("Select a complete installed JDK with working bin/java and bin/javac.");
            } else if java != compiler || declared.is_some_and(|v| Some(v) != java.as_deref()) {
                f.broken = Some(
                    "java, javac and the release metadata disagree; select one intact JDK Home.",
                );
            }
            f.version = java.or(f.version);
            f.home = Some(home.to_owned());
        }
        "maven" | "gradle" => {
            let home = exe.parent()?.parent()?;
            let prefix = if tool == "maven" {
                "maven-core-"
            } else {
                "gradle-core-"
            };
            // Homebrew links bin/mvn to a distribution beneath libexec.
            for dir in [home.join("lib"), home.join("libexec/lib")] {
                if let Some(dir) = host.permitted(&dir, workspace) {
                    f.version = children(&dir)
                        .iter()
                        .filter_map(|p| {
                            p.file_name()?
                                .to_str()?
                                .strip_prefix(prefix)?
                                .strip_suffix(".jar")
                                .and_then(versions::version)
                                .map(str::to_owned)
                        })
                        .next();
                    if f.version.is_some() {
                        break;
                    }
                }
            }
        }
        "npm" | "pnpm" | "yarn" => f.version = package_version(&exe, launcher, tool),
        "docker" | "mise" => (), // presence only, including Docker with its daemon stopped
        _ => f.version = probe::installed_version(&exe, tool, cwd, deadline),
    }
    Some(f)
}

fn at_home(home: &Path, tool: &str) -> Vec<PathBuf> {
    let mut bases = vec![home.join("bin")];
    if matches!(tool, "python" | "node") {
        bases.push(home.to_owned());
    }
    if tool == "jdk" {
        bases.push(home.join("Contents/Home/bin"));
    }
    bases
        .into_iter()
        .flat_map(|p| names(tool).into_iter().map(move |n| p.join(n)))
        .collect()
}

impl Host {
    fn candidates(
        &self,
        tool: &str,
        rule: Option<&ToolRule>,
        workspace: Option<&Path>,
    ) -> Vec<(&'static str, PathBuf)> {
        if let Some(selected) = rule.and_then(|r| r.selected_path.as_deref()) {
            let Some(path) = self.permitted(selected, workspace) else {
                return vec![];
            };
            let candidates = if path.is_dir() {
                at_home(&path, tool)
            } else {
                vec![selected.to_owned()]
            };
            return candidates.into_iter().map(|p| ("explicit", p)).collect();
        }
        let mut found = Vec::new();
        if rule.is_some_and(|r| r.priority == MISE_PRIORITY)
            && let Some(installs) = &self.mise
            && let Some(dir) = self.permitted(
                &installs.join(match tool {
                    "jdk" => "java",
                    "rustc" | "cargo" => "rust",
                    t => t,
                }),
                workspace,
            )
        {
            for home in children(&dir) {
                found.extend(
                    at_home(&home, tool)
                        .into_iter()
                        .map(|p| ("mise-install", p)),
                );
            }
        }
        if let Some(homes) = self.preferred.get(tool) {
            for home in homes {
                found.extend(
                    at_home(home, tool)
                        .into_iter()
                        .map(|p| ("selected-home", p)),
                );
            }
        }
        for bin in &self.bins {
            found.extend(
                names(tool)
                    .into_iter()
                    .map(|n| ("search-path", bin.join(n))),
            );
        }
        if let Some(homes) = self.homes.get(tool) {
            for home in homes {
                found.extend(
                    at_home(home, tool)
                        .into_iter()
                        .map(|p| ("installed-home", p)),
                );
            }
        }
        found
    }
    fn resolve_tool(
        &self,
        tool: &'static str,
        workspace: Option<&Path>,
        rules: &[ToolRule],
        hints: &Hints,
        cwd: &Path,
    ) -> Selection {
        let rule = rules
            .iter()
            .filter(|r| r.tool.as_str() == tool)
            .min_by_key(|r| (r.selected_path.is_none(), r.priority));
        let native = hints.constraints.get(tool);
        let (req, source) = match rule {
            Some(r) => (
                r.version_requirement.as_str(),
                if r.selected_path.is_some() {
                    "explicit"
                } else if r.priority == MISE_PRIORITY {
                    "approved-mise"
                } else {
                    "profile-lock"
                },
            ),
            None => native.map_or(("*", "host"), |v| (v.as_str(), "project")),
        };
        let mut item = Selection {
            tool,
            source: if rule.is_some_and(|r| r.selected_path.is_some()) {
                "explicit"
            } else {
                "host"
            },
            constraint_source: source,
            requirement: if valid_requirement(req) {
                req.to_owned()
            } else {
                "unsupported".into()
            },
            version: None,
            path_summary: None,
            status: "missing",
            details: vec![],
            repair: None,
            executable: None,
            java_home: None,
        };
        if !valid_requirement(req) {
            item.problem(
                "conflict",
                "Use a supported numeric version requirement or set an explicit local profile.",
            );
            return item;
        }
        if rule.is_none()
            && let Some(v) = hints.wrappers.get(tool)
        {
            item.source = "wrapper";
            item.path_summary = Some(format!(
                "<workspace>/{}",
                names(if tool == "maven" { "mvnw" } else { "gradlew" })[0].trim_end_matches(".exe")
            ));
            item.version = v.clone();
            item.status = "metadata-only";
            item.details
                .push("Wrapper metadata only; task approval required before use".into());
            return item;
        }
        if tool == "python"
            && hints.venv
            && rule.is_none_or(|r| r.selected_path.is_none() && r.priority != MISE_PRIORITY)
        {
            item.source = "venv";
            item.path_summary = Some("<workspace>/.venv".into());
            if let Some((path, v)) = workspace.and_then(metadata::venv) {
                item.version = Some(v.clone());
                if matches(tool, &v, req) {
                    item.executable = Some(path);
                    item.status = "metadata-only";
                } else {
                    item.problem(
                        "conflict",
                        "Recreate .venv with an interpreter matching the project requirement.",
                    );
                }
            } else {
                item.problem(
                    "missing",
                    "Recreate .venv with a working local interpreter or select Python explicitly.",
                );
            }
            return item;
        }
        let mut seen = HashSet::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for (origin, path) in self.candidates(tool, rule, workspace) {
            if std::time::Instant::now() >= deadline {
                item.problem(
                    "unverified",
                    "Local probe timed out. Select a specific installed executable and refresh.",
                );
                return item;
            }
            let Some(real) = self.permitted(&path, workspace) else {
                continue;
            };
            if !seen.insert(real) {
                continue;
            }
            let Some(f) = inspect(self, tool, &path, workspace, cwd, req, deadline) else {
                continue;
            };
            let presence = matches!(tool, "docker" | "mise") && req == "*";
            let good = f.broken.is_none()
                && (presence || f.version.as_deref().is_some_and(|v| matches(tool, v, req)));
            if item.path_summary.is_none() || good {
                item.source = origin;
                item.version = f.version;
                item.details = f.details;
                item.path_summary = Some(format!("<installation>/{}", names(tool)[0]));
                if good {
                    item.status = if presence { "present" } else { "available" };
                    item.executable = Some(f.exe);
                    item.java_home = f.home;
                    item.repair = None;
                    break;
                }
                let repair=f.broken.unwrap_or(if item.version.is_some(){"Select an installed version matching the constraint or update the local profile."}else{"Select a verified local installation; this launcher did not provide a version."});
                item.problem(
                    if item.version.is_some() {
                        "conflict"
                    } else {
                        "unverified"
                    },
                    repair,
                );
            }
        }
        if item.path_summary.is_none() {
            item.problem(
                "missing",
                if tool == "jdk" {
                    "Select an installed JDK Home containing bin/java and bin/javac."
                } else if tool == "python" {
                    "Select an existing Python interpreter or recreate the virtual environment."
                } else {
                    "Select an installed local executable or install the required tool and refresh."
                },
            );
        }
        if rule.is_some()
            && let Some(native) = native
            && (!valid_requirement(native)
                || item
                    .version
                    .as_deref()
                    .is_some_and(|v| !matches(tool, v, native)))
        {
            item.problem("conflict","The selected tool conflicts with the project declaration. Align the profile and project versions.");
        }
        item
    }
    pub(crate) fn inventory(
        &self,
        workspace: Option<&Path>,
        rules: &[ToolRule],
    ) -> Result<Inventory, ErrorCode> {
        let cwd = tempfile::tempdir().map_err(|_| ErrorCode::InternalError)?;
        let hints = workspace.map(Hints::load).unwrap_or_default();
        let tools = std::thread::scope(|scope| {
            let jobs: Vec<_> = TOOLS
                .iter()
                .map(|tool| {
                    scope.spawn(|| self.resolve_tool(tool, workspace, rules, &hints, cwd.path()))
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join().map_err(|_| ErrorCode::InternalError))
                .collect::<Result<Vec<_>, _>>()
        })?;
        let mut tools = tools;
        let java = tools[0]
            .java_home
            .as_ref()
            .and(tools[0].version.as_deref())
            .map(str::to_owned);
        for item in &mut tools {
            if matches!(item.tool, "maven" | "gradle") {
                if item.path_summary.is_some()
                    && let Some(v) = &java
                {
                    item.details.push(format!("paired JDK {v}"));
                } else if item.executable.is_some() || item.source == "wrapper" {
                    item.problem("conflict", "Select a working JDK Home for the build tool.");
                }
            }
            if item.tool == "maven" {
                item.details.push(
                    if self
                        .settings
                        .as_ref()
                        .is_some_and(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
                    {
                        "user settings configured"
                    } else {
                        "user settings absent"
                    }
                    .into(),
                );
            }
            if ["npm", "pnpm", "yarn"].contains(&item.tool)
                && (hints.locks.contains(&item.tool) || hints.manager.as_deref() == Some(item.tool))
            {
                item.details.push(format!(
                    "lockfile: {}; packageManager: {}",
                    hints.locks.join(", "),
                    hints.manager.as_deref().unwrap_or("unspecified")
                ));
                if hints.locks.len() > 1
                    || hints.manager.as_ref().is_some_and(|m| {
                        !hints.locks.is_empty() && !hints.locks.contains(&m.as_str())
                    })
                {
                    item.problem(
                        "conflict",
                        "Align packageManager with one lockfile, then refresh.",
                    );
                }
            }
            if item.tool == "git" && workspace.is_some() {
                item.details.push(format!("workspace: {}", hints.git));
            }
            if item.tool == "cargo" && hints.rust {
                item.details.push("Cargo.toml present".into());
            }
            if item.tool == "uv" && hints.venv {
                item.details
                    .push("project virtual environment declared".into());
            }
        }
        Ok(Inventory {
            schema_version: 1,
            tools,
        })
    }
}
impl LocalToolchain {
    pub fn host_doctor() -> Result<Inventory, ErrorCode> {
        Host::local().inventory(None, &[])
    }
    pub fn resolve(
        &self,
        workspace: &Path,
        profile: &ToolchainProfile,
    ) -> Result<Inventory, ErrorCode> {
        Host::local().inventory(Some(workspace), &profile.rules)
    }
}
impl ToolchainInspector for LocalToolchain {
    fn diagnose(
        &self,
        workspace: &Path,
        profile: &ToolchainProfile,
    ) -> Result<Vec<ToolDiagnostic>, ErrorCode> {
        Ok(self
            .resolve(workspace, profile)?
            .tools
            .iter()
            .map(Selection::projection)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn launcher(dir: &Path, tool: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(names(tool)[0].clone());
        fs::write(&p, "fixture launcher; must never execute").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
    fn maven(root: &Path, version: &str) -> PathBuf {
        let p = launcher(&root.join("bin"), "maven");
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(
            root.join("lib").join(format!("maven-core-{version}.jar")),
            [],
        )
        .unwrap();
        p
    }
    fn rule(tool: &str, req: &str, path: Option<PathBuf>, priority: u16) -> ToolRule {
        ToolRule {
            tool: Alias::try_from(tool.to_owned()).unwrap(),
            version_requirement: req.into(),
            selected_path: path,
            priority,
        }
    }
    #[test]
    fn active_search_path_precedes_an_unselected_installation() {
        let tmp = tempfile::tempdir().unwrap();
        let active = maven(&tmp.path().join("active"), "3.9.12");
        let other = tmp.path().join("other");
        maven(&other, "3.8.8");
        let mut host = Host::default();
        host.bins.push(active.parent().unwrap().into());
        host.homes.insert("maven", vec![other]);
        let selected = host.resolve_tool("maven", None, &[], &Hints::default(), tmp.path());
        assert_eq!(selected.version.as_deref(), Some("3.9.12"));
    }
    #[test]
    fn explicit_missing_path_does_not_fall_back_and_wrapper_never_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path().join("workspace");
        let home = tmp.path().join("installed Maven");
        fs::create_dir_all(w.join(".mvn/wrapper")).unwrap();
        let exe = maven(&home, "3.9.12");
        fs::write(
            w.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }),
            "this is repository code",
        )
        .unwrap();
        fs::write(w.join(".mvn/wrapper/maven-wrapper.properties"),"distributionUrl=https://private.invalid/apache-maven-3.8.8-bin.zip?token=TEST_ONLY_CANARY").unwrap();
        let mut host = Host::default();
        host.bins.push(home.join("bin"));
        let hints = Hints::load(&w);
        let inspect =
            |rules: &[ToolRule]| host.resolve_tool("maven", Some(&w), rules, &hints, tmp.path());
        let wrapper = inspect(&[]);
        assert_eq!(wrapper.source, "wrapper");
        assert_eq!(wrapper.version.as_deref(), Some("3.8.8"));
        assert!(wrapper.executable.is_none());
        assert!(
            !serde_json::to_string(&wrapper)
                .unwrap()
                .contains("private.invalid")
        );
        let explicit = inspect(&[rule("maven", "3", Some(exe), 0)]);
        assert_eq!(explicit.source, "explicit");
        assert_eq!(explicit.version.as_deref(), Some("3.9.12"));
        let missing = inspect(&[rule("maven", "3", Some(tmp.path().join("missing")), 0)]);
        assert_eq!(missing.status, "missing");
        assert!(missing.repair.is_some());
        assert!(missing.executable.is_none());
    }
    #[test]
    fn approved_mise_pins_use_existing_installs_and_are_snapshots() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path().join("repo");
        fs::create_dir(&w).unwrap();
        fs::write(
            w.join("mise.toml"),
            "[tools]\nmaven='3.8'\n[env]\nSECRET='TEST_ONLY_CANARY'",
        )
        .unwrap();
        let pins = mise_rules(&w).unwrap();
        fs::write(w.join("mise.toml"), "[tools]\nmaven='3.9'").unwrap();
        let installs = tmp.path().join("mise/installs");
        maven(&installs.join("maven/3.8.8"), "3.8.8");
        let active = tmp.path().join("active");
        maven(&active, "3.9.12");
        let h = Host {
            bins: vec![active.join("bin")],
            mise: Some(installs),
            ..Host::default()
        };
        let resolve = |rules: &[ToolRule]| {
            h.resolve_tool("maven", Some(&w), rules, &Hints::load(&w), tmp.path())
        };
        assert_eq!(resolve(&[]).version.as_deref(), Some("3.9.12"));
        let approved = resolve(&pins);
        assert_eq!(approved.source, "mise-install");
        assert_eq!(approved.version.as_deref(), Some("3.8.8"));
        assert_eq!(
            resolve(&[rule("maven", "3.8", Some(active), 0)]).status,
            "conflict"
        );
    }
}
