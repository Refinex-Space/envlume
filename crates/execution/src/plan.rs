use crate::Launch;
use envlume_application::{
    ErrorCode,
    tasks::{AuthorizedTask, Template},
};
use envlume_toolchain::{Inventory, LocalToolchain, Selection, node_manager};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use url::Url;

pub struct Review {
    pub digest: String,
    pub distribution_digest: Option<String>,
    pub urls: Vec<String>,
}
fn hex(data: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(data))
}
/// Wrapper inputs must be regular files below the bound root. Never follow a
/// repository-provided link while hashing a script, JAR, or properties file.
fn local_file(root: &Path, relative: &str, limit: u64) -> Result<Option<Vec<u8>>, ErrorCode> {
    let mut path = root.to_owned();
    for part in relative.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(ErrorCode::PolicyDenied);
                    }
                }
                if m.is_symlink() {
                    return Err(ErrorCode::PolicyDenied);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ErrorCode::PolicyDenied),
        }
    }
    if !envlume_workspace::local_canonical(&path).is_ok_and(|p| p.starts_with(root)) {
        return Err(ErrorCode::PolicyDenied);
    }
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
    let f = options.open(path).map_err(|_| ErrorCode::PolicyDenied)?;
    let m = f.metadata().map_err(|_| ErrorCode::PolicyDenied)?;
    if !m.is_file() || m.len() > limit {
        return Err(ErrorCode::PolicyDenied);
    }
    let mut bytes = Vec::new();
    f.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::PolicyDenied)?;
    if bytes.len() as u64 > limit {
        return Err(ErrorCode::PolicyDenied);
    }
    Ok(Some(bytes))
}
fn wrapper_urls(properties: &[u8]) -> Result<Vec<String>, ErrorCode> {
    let text = std::str::from_utf8(properties).map_err(|_| ErrorCode::PolicyDenied)?;
    let mut dist = None;
    let mut jar = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', '!']) {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ErrorCode::PolicyDenied)?;
        // Reject ambiguous Java properties encodings instead of approving one
        // interpretation while the repository launcher consumes another.
        if !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
            || value.ends_with('\\')
        {
            return Err(ErrorCode::PolicyDenied);
        }
        let slot = match key {
            "distributionUrl" => &mut dist,
            "wrapperUrl" => &mut jar,
            _ => continue,
        };
        let value = value.trim().replace(r"\:", ":");
        if value.contains('\\') || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ErrorCode::PolicyDenied);
        }
        let url = Url::parse(&value).map_err(|_| ErrorCode::PolicyDenied)?;
        if slot.is_some()
            || url.scheme() != "https"
            || !url.has_host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ErrorCode::PolicyDenied);
        }
        *slot = Some(url.to_string());
    }
    let mut urls = vec![dist.ok_or(ErrorCode::PolicyDenied)?];
    urls.extend(jar);
    Ok(urls)
}
pub fn review(task: &AuthorizedTask) -> Result<Review, ErrorCode> {
    let mut inputs = BTreeMap::new();
    let mut urls = Vec::new();
    match task.template {
        Template::NodeBuild => {
            let pkg = local_file(&task.root, "package.json", 512 * 1024)?
                .ok_or(ErrorCode::ToolchainMissing)?;
            let pkg: serde_json::Value =
                serde_json::from_slice(&pkg).map_err(|_| ErrorCode::InvalidInput)?;
            let script = pkg
                .pointer("/scripts/build")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(ErrorCode::ToolchainMissing)?;
            inputs.insert("build", hex(script));
            inputs.insert("manager", node_manager(&task.root)?.into());
            if let Some(pin) = pkg["packageManager"].as_str() {
                inputs.insert("packageManager", hex(pin));
            }
            for name in ["prebuild", "postbuild"] {
                if let Some(v) = pkg["scripts"][name].as_str() {
                    inputs.insert(name, hex(v));
                }
            }
        }
        Template::MavenTest
            if !task
                .profile
                .rules
                .iter()
                .any(|r| r.tool.as_str() == "maven") =>
        {
            let script = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
            if let Some(bytes) = local_file(&task.root, script, 512 * 1024)? {
                inputs.insert("wrapper", hex(bytes));
                let properties = local_file(
                    &task.root,
                    ".mvn/wrapper/maven-wrapper.properties",
                    64 * 1024,
                )?
                .ok_or(ErrorCode::PolicyDenied)?;
                urls = wrapper_urls(&properties)?;
                inputs.insert("properties", hex(properties));
                for (key, path) in [
                    ("jar", ".mvn/wrapper/maven-wrapper.jar"),
                    ("source", ".mvn/wrapper/MavenWrapperDownloader.java"),
                    ("jvmConfig", ".mvn/jvm.config"),
                    ("mavenConfig", ".mvn/maven.config"),
                ] {
                    if let Some(bytes) = local_file(&task.root, path, 4 * 1024 * 1024)? {
                        inputs.insert(key, hex(bytes));
                    }
                }
            }
        }
        _ => (),
    }
    let spec = serde_json::json!([
        task.binding,
        task.project,
        task.task,
        task.grant,
        task.template.key(),
        task.template.arguments(),
        ".",
        task.timeout.as_secs(),
        task.profile.id,
        task.profile.rules,
        inputs
    ]);
    let digest = hex(spec.to_string());
    let distribution_digest = (!urls.is_empty()).then(|| hex(serde_json::json!(urls).to_string()));
    Ok(Review {
        digest,
        distribution_digest,
        urls,
    })
}
fn selected<'a>(inventory: &'a Inventory, key: &str) -> Result<&'a Selection, ErrorCode> {
    inventory
        .tools
        .iter()
        .find(|t| t.tool == key && t.repair.is_none())
        .ok_or(ErrorCode::ToolchainMissing)
}
fn exe(selection: &Selection) -> Result<&Path, ErrorCode> {
    selection
        .executable
        .as_deref()
        .ok_or(ErrorCode::ToolchainMissing)
}
fn property(key: &str, path: &Path) -> OsString {
    let mut s = OsString::from(key);
    s.push(dunce::simplified(path));
    s
}
fn child_environment(
    home: &Path,
    temp: &Path,
    bins: &[PathBuf],
) -> Result<BTreeMap<OsString, OsString>, ErrorCode> {
    let mut env = BTreeMap::new();
    let home = dunce::simplified(home);
    let temp = dunce::simplified(temp);
    let mut path: Vec<_> = bins
        .iter()
        .map(|p| dunce::simplified(p).to_owned())
        .collect();
    #[cfg(unix)]
    path.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    #[cfg(windows)]
    {
        let windows = system_root()?;
        env.insert("SystemRoot".into(), windows.as_os_str().to_owned());
        env.insert("WINDIR".into(), windows.as_os_str().to_owned());
        env.insert(
            "ComSpec".into(),
            windows.join("System32").join("cmd.exe").into_os_string(),
        );
        env.insert("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into());
        path.extend([
            windows.join("System32"),
            windows
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0"),
        ]);
        env.insert("USERPROFILE".into(), home.as_os_str().to_owned());
        env.insert("APPDATA".into(), home.as_os_str().to_owned());
        env.insert("LOCALAPPDATA".into(), home.as_os_str().to_owned());
    }
    env.insert(
        "PATH".into(),
        std::env::join_paths(path).map_err(|_| ErrorCode::ToolchainMissing)?,
    );
    env.insert("HOME".into(), home.as_os_str().to_owned());
    for key in ["TMPDIR", "TEMP", "TMP"] {
        env.insert(key.into(), temp.as_os_str().to_owned());
    }
    env.insert("CI".into(), "true".into());
    Ok(env)
}
#[cfg(windows)]
fn system_root() -> Result<PathBuf, ErrorCode> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let mut buf = [0u16; 32768];
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return Err(ErrorCode::InternalError);
    }
    Ok(OsString::from_wide(&buf[..n]).into())
}
pub fn prepare(
    task: &AuthorizedTask,
    approval: &Review,
    home: &Path,
    temp: &Path,
) -> Result<Launch, ErrorCode> {
    let inventory = LocalToolchain.resolve(&task.root, &task.profile)?;
    let mut versions = BTreeMap::new();
    let (program, args, env) = match task.template {
        Template::MavenTest => {
            let jdk = selected(&inventory, "jdk")?;
            let maven = selected(&inventory, "maven")?;
            let jhome = jdk.java_home.as_ref().ok_or(ErrorCode::ToolchainMissing)?;
            versions.insert(
                "jdk".into(),
                jdk.version.clone().ok_or(ErrorCode::ToolchainMissing)?,
            );
            versions.insert(
                if maven.source == "wrapper" {
                    "maven-wrapper-declared"
                } else {
                    "maven"
                }
                .into(),
                maven.version.clone().ok_or(ErrorCode::ToolchainMissing)?,
            );
            let mut env = child_environment(home, temp, &[jhome.join("bin")])?;
            env.insert(
                "JAVA_HOME".into(),
                dunce::simplified(jhome).as_os_str().to_owned(),
            );
            env.insert(
                "MAVEN_USER_HOME".into(),
                dunce::simplified(home).join(".m2").into_os_string(),
            );
            if maven.source == "wrapper" {
                if approval.urls.is_empty() {
                    return Err(ErrorCode::PolicyDenied);
                }
                // Maven Wrapper starts its own JVM. Override its default user.home
                // without interpolating any path into the Windows command line.
                let private = dunce::simplified(home)
                    .to_str()
                    .ok_or(ErrorCode::InvalidInput)?;
                if private.chars().any(|c| c == '"' || c.is_control()) {
                    return Err(ErrorCode::InvalidInput);
                }
                env.insert(
                    "JAVA_TOOL_OPTIONS".into(),
                    format!(r#""-Duser.home={private}""#).into(),
                );
                #[cfg(unix)]
                let command = (
                    task.root.join("mvnw"),
                    vec!["-B".into(), "-ntp".into(), "test".into()],
                    env,
                );
                #[cfg(windows)]
                let command = (
                    system_root()?.join("System32").join("cmd.exe"),
                    vec![
                        "/d".into(),
                        "/v:off".into(),
                        "/c".into(),
                        r".\mvnw.cmd".into(),
                        "-B".into(),
                        "-ntp".into(),
                        "test".into(),
                    ],
                    env,
                );
                command
            } else {
                if !approval.urls.is_empty() {
                    return Err(ErrorCode::PolicyDenied);
                }
                let mhome = exe(maven)?
                    .parent()
                    .and_then(Path::parent)
                    .ok_or(ErrorCode::ToolchainMissing)?;
                let distribution = if mhome.join("boot").is_dir() {
                    mhome.to_owned()
                } else {
                    mhome.join("libexec")
                };
                let mhome = distribution.as_path();
                let boot = mhome.join("boot");
                let mut jars = Vec::new();
                for entry in fs::read_dir(boot).map_err(|_| ErrorCode::ToolchainMissing)? {
                    let path = entry.map_err(|_| ErrorCode::ToolchainMissing)?.path();
                    if path.file_name().and_then(OsStr::to_str).is_some_and(|s| {
                        s.starts_with("plexus-classworlds-") && s.ends_with(".jar")
                    }) {
                        jars.push(path);
                    }
                }
                let [jar] = jars.as_slice() else {
                    return Err(ErrorCode::ToolchainMissing);
                };
                let conf = mhome.join("bin/m2.conf");
                if !conf.is_file() {
                    return Err(ErrorCode::ToolchainMissing);
                }
                let mut args = vec![
                    property("-Duser.home=", home),
                    "-classpath".into(),
                    dunce::simplified(jar).as_os_str().to_owned(),
                    property("-Dclassworlds.conf=", &conf),
                    property("-Dmaven.home=", mhome),
                    property("-Dmaven.multiModuleProjectDirectory=", &task.root),
                    "org.codehaus.plexus.classworlds.launcher.Launcher".into(),
                ];
                args.extend(task.template.arguments().iter().map(OsString::from));
                (exe(jdk)?.to_owned(), args, env)
            }
        }
        Template::NodeBuild => {
            let node = selected(&inventory, "node")?;
            let manager = node_manager(&task.root)?;
            let pm = selected(&inventory, manager)?;
            let js = pm
                .package_entry
                .as_ref()
                .ok_or(ErrorCode::ToolchainMissing)?;
            let program = exe(node)?.to_owned();
            versions.insert(
                "node".into(),
                node.version.clone().ok_or(ErrorCode::ToolchainMissing)?,
            );
            versions.insert(
                manager.into(),
                pm.version.clone().ok_or(ErrorCode::ToolchainMissing)?,
            );
            let mut env = child_environment(
                home,
                temp,
                &[program
                    .parent()
                    .ok_or(ErrorCode::ToolchainMissing)?
                    .to_owned()],
            )?;
            env.insert(
                "NODE".into(),
                dunce::simplified(&program).as_os_str().to_owned(),
            );
            (
                program,
                vec![
                    dunce::simplified(js).as_os_str().to_owned(),
                    "run".into(),
                    "build".into(),
                ],
                env,
            )
        }
    };
    Ok(Launch {
        program: dunce::simplified(&program).to_owned(),
        args,
        cwd: dunce::simplified(&task.root).to_owned(),
        env,
        versions,
    })
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use envlume_domain::{Alias, ProfileId, ProjectId, ToolchainProfile};
    use std::{
        io::Read,
        process::{Command, Stdio},
    };

    #[test]
    fn npm_runs_a_lifecycle_in_a_windows_job() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("T07 空格 build");
        let home = base.path().join("home");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&home).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"fixture","scripts":{"build":"node build.cjs"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("build.cjs"),
            "process.stdout.write('fixture-built')",
        )
        .unwrap();
        let root = root.canonicalize().unwrap();
        let inventory = LocalToolchain
            .resolve(
                &root,
                &ToolchainProfile {
                    id: ProfileId::generate(),
                    project_id: ProjectId::generate(),
                    name: Alias::try_from("default".to_owned()).unwrap(),
                    rules: Vec::new(),
                },
            )
            .unwrap();
        let node = exe(selected(&inventory, "node").unwrap()).unwrap();
        let npm = selected(&inventory, "npm")
            .unwrap()
            .package_entry
            .as_ref()
            .unwrap();
        let mut cmd = Command::new(dunce::simplified(node));
        let env = child_environment(&home, base.path(), &[node.parent().unwrap().into()]).unwrap();
        cmd.args([
            dunce::simplified(npm).as_os_str(),
            OsStr::new("run"),
            OsStr::new("build"),
        ])
        .current_dir(dunce::simplified(&root))
        .env_clear()
        .envs(env)
        .env("NODE", dunce::simplified(node))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = crate::tree::Tree::spawn(cmd).unwrap();
        let (mut out, mut err) = child.output();
        let mut stdout = String::new();
        let mut stderr = String::new();
        out.read_to_string(&mut stdout).unwrap();
        err.read_to_string(&mut stderr).unwrap();
        let status = loop {
            if let Some(s) = child.finished().unwrap() {
                break s;
            }
            std::thread::yield_now();
        };
        assert!(status.success(), "Synthetic installed npm failed: {stderr}");
        assert!(
            stdout.contains("fixture-built"),
            "Synthetic npm lifecycle output: {stdout} {stderr}"
        );
    }
}
