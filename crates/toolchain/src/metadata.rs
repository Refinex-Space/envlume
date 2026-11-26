use crate::versions::{valid_requirement, version};
use envlume_domain::{Alias, ToolRule};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const LIMIT: u64 = 512 * 1024;
pub(crate) fn read(path: &Path) -> Option<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(path).ok()?;
    let m = file.metadata().ok()?;
    if !m.is_file() || linked(&m) || m.len() > LIMIT {
        return None;
    }
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text).ok()?;
    (text.len() <= LIMIT as usize).then_some(text)
}
fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.is_symlink()
    }
}
/// Every component below the confirmed root must be local and link-free.
fn project_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut path = root.to_owned();
    for part in name.split('/') {
        path.push(part);
        let m = fs::symlink_metadata(&path).ok()?;
        if linked(&m) {
            return None;
        }
    }
    path.is_file().then_some(path)
}
pub(crate) fn prop<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"'))
}
fn pom_java(text: &str) -> Option<String> {
    use quick_xml::{Reader, events::Event};
    let mut reader = Reader::from_str(text);
    let mut elements = Vec::new();
    let mut fields = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => elements.push(e.local_name().as_ref().to_owned()),
            Ok(Event::End(_)) => {
                elements.pop();
            }
            Ok(Event::Text(e))
                if elements.len() == 3
                    && elements[0] == "project"
                    && elements[1] == "properties" =>
            {
                let k = elements[2].clone();
                if [
                    "maven.compiler.release",
                    "maven.compiler.source",
                    "java.version",
                ]
                .contains(&k.as_str())
                {
                    fields.insert(k, e.trim().to_owned());
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => (),
        }
    }
    for k in [
        "maven.compiler.release",
        "maven.compiler.source",
        "java.version",
    ] {
        if let Some(v) = fields.get(k) {
            let v = if let Some(key) = v.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
                fields.get(key)?
            } else {
                v
            };
            return Some(v.strip_prefix("1.").unwrap_or(v).to_owned());
        }
    }
    None
}
#[derive(Default)]
pub(crate) struct Hints {
    pub constraints: BTreeMap<&'static str, String>,
    pub wrappers: BTreeMap<&'static str, Option<String>>,
    pub manager: Option<String>,
    pub locks: Vec<&'static str>,
    pub venv: bool,
    pub git: &'static str,
    pub rust: bool,
}
impl Hints {
    pub fn load(root: &Path) -> Self {
        let mut h = Self::default();
        let text = |name| project_file(root, name).and_then(|p| read(&p));
        if let Some(v) = text("pom.xml").as_deref().and_then(pom_java) {
            h.constraints.insert("jdk", v);
        }
        if let Some(pkg) =
            text("package.json").and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            if let Some(v) = pkg.pointer("/engines/node").and_then(|v| v.as_str()) {
                h.constraints.insert("node", v.to_owned());
            }
            if let Some(pm) = pkg["packageManager"].as_str() {
                let (name, v) = pm.split_once('@').unwrap_or(("", ""));
                if let Some(tool) = ["npm", "pnpm", "yarn"].into_iter().find(|t| *t == name) {
                    h.manager = Some(tool.to_owned());
                    h.constraints
                        .insert(tool, v.split('+').next().unwrap_or("").to_owned());
                } else {
                    h.manager = Some("unsupported".into());
                }
            }
        }
        if let Some(v) = text(".nvmrc") {
            h.constraints
                .insert("node", v.trim().trim_start_matches('v').to_owned());
        }
        for (file, manager) in [
            ("package-lock.json", "npm"),
            ("npm-shrinkwrap.json", "npm"),
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
        ] {
            if project_file(root, file).is_some() && !h.locks.contains(&manager) {
                h.locks.push(manager);
            }
        }
        if let Some(v) = text("pyproject.toml")
            .and_then(|s| s.parse::<toml::Table>().ok())
            .and_then(|p| {
                p.get("project")?
                    .get("requires-python")?
                    .as_str()
                    .map(str::to_owned)
            })
        {
            h.constraints.insert("python", v);
        }
        if let Some(v) = text(".python-version") {
            h.constraints.insert("python", v.trim().to_owned());
        }
        h.venv = fs::symlink_metadata(root.join(".venv")).is_ok();
        h.git = envlume_workspace::discover(root).map_or("unavailable", |w| w.git_kind);
        h.rust = project_file(root, "Cargo.toml").is_some();
        for (tool, sh, win, file, prefix) in [
            (
                "maven",
                "mvnw",
                "mvnw.cmd",
                ".mvn/wrapper/maven-wrapper.properties",
                "apache-maven-",
            ),
            (
                "gradle",
                "gradlew",
                "gradlew.bat",
                "gradle/wrapper/gradle-wrapper.properties",
                "gradle-",
            ),
        ] {
            if project_file(root, if cfg!(windows) { win } else { sh }).is_some() {
                let v = text(file).and_then(|s| {
                    let filename = prop(&s, "distributionUrl")?
                        .split(['?', '#'])
                        .next()?
                        .rsplit('/')
                        .next()?;
                    let core = filename.strip_prefix(prefix)?;
                    version(
                        core.strip_suffix("-bin.zip")
                            .or_else(|| core.strip_suffix("-all.zip"))?,
                    )
                    .map(str::to_owned)
                });
                h.wrappers.insert(tool, v);
            }
        }
        h
    }
}
pub(crate) fn venv(root: &Path) -> Option<(PathBuf, String)> {
    let cfg = read(&project_file(root, ".venv/pyvenv.cfg")?)?;
    let v = version(prop(&cfg, "version").or_else(|| prop(&cfg, "version_info"))?)?;
    let bin = if cfg!(windows) { "Scripts" } else { "bin" };
    let base = root.join(".venv").join(bin);
    if linked(&fs::symlink_metadata(&base).ok()?) {
        return None;
    }
    let path = base.join(if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    });
    let real = envlume_workspace::local_canonical(&path).ok()?;
    (crate::executable(&real) && crate::probe::native(&real)).then(|| (path, v.to_owned()))
}

pub const MISE_PRIORITY: u16 = 20;
/// Import only a static, numeric [tools] selection; never evaluate plugins, env or hooks.
pub fn mise_rules(root: &Path) -> Result<Vec<ToolRule>, &'static str> {
    let file = project_file(root, "mise.toml")
        .or_else(|| project_file(root, ".mise.toml"))
        .ok_or("Missing local mise.toml")?;
    let config = read(&file)
        .and_then(|s| s.parse::<toml::Table>().ok())
        .ok_or("Invalid mise.toml")?;
    let tools = config
        .get("tools")
        .and_then(|v| v.as_table())
        .ok_or("Missing [tools] in mise.toml")?;
    let mut rules = Vec::new();
    for (name, value) in tools {
        let name = match name.as_str() {
            "java" => "jdk",
            "rust" => "rustc",
            n => n,
        };
        if !crate::TOOLS.contains(&name) {
            continue;
        }
        let v = value
            .as_str()
            .filter(|v| valid_requirement(v) && version(v).is_some())
            .ok_or("Use numeric installed versions in [tools]")?;
        rules.push(ToolRule {
            tool: Alias::try_from(name.to_owned()).unwrap(),
            version_requirement: v.to_owned(),
            selected_path: None,
            priority: MISE_PRIORITY,
        });
    }
    if rules.is_empty() {
        return Err("No supported version pins in [tools]");
    }
    Ok(rules)
}
