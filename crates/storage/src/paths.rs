use crate::{Error, Result};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct StorePaths {
    root: PathBuf,
}

impl StorePaths {
    pub fn local() -> Result<Self> {
        let base = dirs::data_local_dir().ok_or(Error::StorageUnavailable)?;
        fs::create_dir_all(&base).map_err(|_| Error::StorageUnavailable)?;
        Self::at(base.join("Envlume"))
    }
    /// A trusted local host (including a test) supplies the application directory.
    pub fn at(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() || root.file_name().is_none() {
            return Err(Error::PrivateDirectoryRequired);
        }
        // macOS may locate the private temp directory through /var -> /private/var.
        // Resolve only the system parent. Keep NOFOLLOW on the application tree.
        #[cfg(unix)]
        let root = root
            .parent()
            .ok_or(Error::PrivateDirectoryRequired)?
            .canonicalize()
            .map_err(|_| Error::StorageUnavailable)?
            .join(root.file_name().ok_or(Error::PrivateDirectoryRequired)?);
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("data").join("envlume.db")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub(crate) fn lock_cli_bin(&self) -> Result<(PathBuf, fs::File)> {
        prepare_private(self.root(), false)?;
        let lock = crate::schema::migration_lock(self)?;
        prepare_private(self.root(), true)?;
        Ok((self.root.join("bin"), lock))
    }
    pub(crate) fn prepare(&self) -> Result<()> {
        if !self.root.is_absolute() {
            return Err(Error::PrivateDirectoryRequired);
        }
        prepare_private(&self.root, false)?;
        for path in [
            self.database(),
            self.database().with_extension("db-wal"),
            self.database().with_extension("db-shm"),
            self.database().with_extension("db-journal"),
        ] {
            check_file(&path)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn prepare_private(root: &Path, cli: bool) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let names: &[&str] = if cli {
        &["", "data", "bin"]
    } else {
        &["", "data", "logs", "tmp", "backups"]
    };
    for name in names {
        let dir = if name.is_empty() {
            root.to_owned()
        } else {
            root.join(name)
        };
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(Error::StorageUnavailable),
        }
        let m = fs::symlink_metadata(&dir).map_err(|_| Error::StorageUnavailable)?;
        if !m.is_dir() || !private(&m) {
            return Err(Error::PrivateDirectoryRequired);
        }
    }
    Ok(())
}
#[cfg(unix)]
fn private(m: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid takes no arguments and has no preconditions.
    m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0
}

#[cfg(windows)]
fn prepare_private(root: &Path, cli: bool) -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    // Use Windows PowerShell's built-in .NET ACL APIs. Path data is never a command.
    let shell = std::env::var_os("SystemRoot").ok_or(Error::StorageUnavailable)?;
    let output =
        Command::new(Path::new(&shell).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("private-directories.ps1"),
            ])
            .env("ENVLUME_ACL_ROOT", root)
            .env("ENVLUME_ACL_SCOPE", if cli { "cli" } else { "catalogue" })
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| Error::StorageUnavailable)?;
    if !output.success() {
        return Err(Error::PrivateDirectoryRequired);
    }
    Ok(())
}

pub(crate) fn check_file(path: &Path) -> Result<()> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(Error::StorageUnavailable),
    };
    if !m.is_file() {
        return Err(Error::PrivateDirectoryRequired);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !private(&m) || m.nlink() != 1 {
            return Err(Error::PrivateDirectoryRequired);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(Error::PrivateDirectoryRequired);
        }
    }
    Ok(())
}

pub(crate) fn create_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
