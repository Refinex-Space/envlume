//! Native-host CLI installation. Callers supply a release from their trusted package.
use crate::{Error as StorageError, StorePaths, connect, paths, schema};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tempfile::{Builder, NamedTempFile};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub target: String,
    pub max_schema: i64,
    pub sha256: String,
}

pub fn native_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") if cfg!(target_env = "msvc") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstallError {
    NotInstalled,
    NoPrevious,
    InvalidRelease,
    IncompatibleSchema,
    Downgrade,
    UnverifiedInstallation,
    Busy,
    AccessDenied,
    UnsafePath,
    StorageUnavailable,
}

impl InstallError {
    pub fn message(self) -> &'static str {
        match self {
            Self::NotInstalled => "Install the CLI from the desktop package.",
            Self::NoPrevious => "No previous CLI is available to restore.",
            Self::InvalidRelease => "Check the bundled CLI version, target and checksum.",
            Self::IncompatibleSchema => "Use a CLI compatible with the current catalogue.",
            Self::Downgrade => "Use explicit restore for a compatible previous CLI.",
            Self::UnverifiedInstallation => {
                "The stable executable or its receipt could not be verified."
            }
            Self::Busy => {
                "Close running CLI or MCP processes and retry after the current operation."
            }
            Self::AccessDenied | Self::UnsafePath => {
                "Check ownership and write permissions on the private installation directory."
            }
            Self::StorageUnavailable => "Check local storage access and free space, then retry.",
        }
    }
}
impl From<StorageError> for InstallError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::PrivateDirectoryRequired => Self::UnsafePath,
            StorageError::StorageBusy => Self::Busy,
            StorageError::UpgradeRequired | StorageError::InvalidDatabase => {
                Self::IncompatibleSchema
            }
            _ => Self::StorageUnavailable,
        }
    }
}
impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        #[cfg(windows)]
        if matches!(e.raw_os_error(), Some(32 | 33)) {
            return Self::Busy;
        }
        if e.kind() == io::ErrorKind::PermissionDenied {
            Self::AccessDenied
        } else {
            Self::StorageUnavailable
        }
    }
}

type Result<T> = std::result::Result<T, InstallError>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCli {
    pub path: PathBuf,
    pub release: Release,
}

impl Release {
    fn validate(&self) -> Result<()> {
        if native_target() == "unsupported"
            || self.target != native_target()
            || self.version.len() > 128
            || semver::Version::parse(&self.version).is_err()
            || self.max_schema < 1
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(InstallError::InvalidRelease);
        }
        Ok(())
    }
    fn backup(&self, bin: &Path) -> PathBuf {
        bin.join(format!("previous-{}.bin", self.sha256))
    }
}

const RECEIPT: &str = "cli-releases.json";
fn stable(bin: &Path) -> PathBuf {
    bin.join(if cfg!(windows) {
        "envlume.exe"
    } else {
        "envlume"
    })
}

// If publication is interrupted, the live digest selects either the first (new)
// or second (old) release. The third retains the old release's rollback target.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    releases: Vec<Release>,
    skip_on_startup: Option<String>,
}

fn receipt(bin: &Path) -> Result<Receipt> {
    let path = bin.join(RECEIPT);
    paths::check_file(&path)?;
    let mut buf = Vec::new();
    match File::open(path) {
        Ok(f) => f.take(8193).read_to_end(&mut buf)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Receipt::default()),
        Err(e) => return Err(e.into()),
    };
    let record: Receipt =
        serde_json::from_slice(&buf).map_err(|_| InstallError::UnverifiedInstallation)?;
    let list = &record.releases;
    if buf.len() > 8192
        || list.is_empty()
        || list.len() > 3
        || list.iter().any(|r| r.validate().is_err())
    {
        return Err(InstallError::UnverifiedInstallation);
    }
    Ok(record)
}

fn digest(mut input: impl Read) -> Result<String> {
    let mut hash = Sha256::new();
    io::copy(&mut input, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn active(path: &Path, list: &[Release]) -> Result<Option<usize>> {
    paths::check_file(path)?;
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o100 == 0 {
            return Err(InstallError::UnverifiedInstallation);
        }
    }
    let sum = digest(file)?;
    list.iter()
        .take(2)
        .position(|r| r.sha256 == sum)
        .map(Some)
        .ok_or(InstallError::UnverifiedInstallation)
}

fn compatibility(paths: &StorePaths, r: &Release) -> Result<Option<i64>> {
    let path = paths.database();
    paths::check_file(&path)?;
    for ext in ["db-wal", "db-shm", "db-journal"] {
        paths::check_file(&path.with_extension(ext))?;
    }
    if path.try_exists()? {
        let mut conn = connect(&path, false)?;
        let tx = conn.transaction().map_err(crate::database_error)?;
        let db = schema::version(&tx)?;
        if db > r.max_schema {
            return Err(InstallError::IncompatibleSchema);
        }
        return Ok((db > 0).then_some(db));
    } else {
        for ext in ["db-wal", "db-shm", "db-journal"] {
            if path.with_extension(ext).try_exists()? {
                return Err(InstallError::IncompatibleSchema);
            }
        }
    }
    Ok(None)
}

fn stage(bin: &Path) -> Result<NamedTempFile> {
    Ok(Builder::new().prefix(".cli-stage-").tempfile_in(bin)?)
}
fn native_image(mut f: File) -> Result<bool> {
    let mut header = [0u8; 64];
    if f.read_exact(&mut header).is_err() {
        return Ok(false);
    }
    if cfg!(windows) {
        if &header[..2] != b"MZ" {
            return Ok(false);
        }
        let offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
        f.seek(SeekFrom::Start(offset.into()))?;
        let mut pe = [0u8; 26];
        return Ok(f.read_exact(&mut pe).is_ok()
            && &pe[..6] == b"PE\0\0\x64\x86"
            && pe[24..] == [0x0b, 0x02]);
    }
    let cpu = if cfg!(target_arch = "aarch64") {
        0x0100000cu32
    } else {
        0x01000007u32
    };
    Ok(header[..4] == 0xfeedfacfu32.to_le_bytes()
        && header[4..8] == cpu.to_le_bytes()
        && header[12..16] == 2u32.to_le_bytes())
}

fn staged_image(bin: &Path, mut input: impl Read, r: &Release) -> Result<NamedTempFile> {
    let mut f = stage(bin)?;
    io::copy(&mut input, &mut f)?;
    if !native_image(f.reopen()?)? || digest(f.reopen()?)? != r.sha256 {
        return Err(InstallError::InvalidRelease);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    f.as_file().sync_all()?;
    Ok(f)
}
fn publish(f: NamedTempFile, path: &Path) -> Result<()> {
    // Close the writable handle before publishing an executable.
    f.into_temp_path().persist(path).map_err(|e| {
        #[cfg(windows)]
        if matches!(e.error.raw_os_error(), Some(5 | 32 | 33))
            && path
                .metadata()
                .is_ok_and(|m| m.is_file() && !m.permissions().readonly())
        {
            return InstallError::Busy;
        }
        InstallError::from(e.error)
    })?;
    #[cfg(unix)]
    File::open(path.parent().expect("file in CLI bin"))?.sync_all()?;
    Ok(())
}

// The trusted native host owns the input manifest. Validate the complete copied image before touching
// the live name. Receipt order describes both sides of the atomic rename, including after a crash.
fn write_receipt(bin: &Path, record: &Receipt) -> Result<()> {
    let mut file = stage(bin)?;
    file.write_all(&serde_json::to_vec(record).map_err(|_| InstallError::InvalidRelease)?)?;
    file.as_file().sync_all()?;
    publish(file, &bin.join(RECEIPT))
}

fn with_catalogue_backup(
    paths: &StorePaths,
    version: Option<i64>,
    change: impl FnOnce() -> Result<InstalledCli>,
) -> Result<InstalledCli> {
    let Some(db) = version else { return change() };
    let backup = paths.backups().join(format!(
        "pre-cli-{}.db",
        envlume_domain::RequestId::generate().0
    ));
    schema::snapshot(&connect(&paths.database(), false)?, &backup, db)?;
    let result = change();
    // A sharing violation happens before activation. Retrying a busy binary must
    // not accumulate duplicate snapshots. Other I/O failures keep the verified copy.
    if matches!(result, Err(InstallError::Busy)) {
        fs::remove_file(backup)?;
    }
    result
}

fn switch(
    bin: &Path,
    list: &[Release],
    index: Option<usize>,
    new: &Release,
    image: NamedTempFile,
    skip_on_startup: Option<String>,
) -> Result<InstalledCli> {
    let path = stable(bin);
    let mut next = vec![new.clone()];
    if let Some(i) = index {
        let current = &list[i];
        let backup = current.backup(bin);
        paths::check_file(&backup)?;
        publish(staged_image(bin, File::open(&path)?, current)?, &backup)?;
        next.push(current.clone());
        if let Some(prior) = list.get(i + 1)
            && prior != new
        {
            next.push(prior.clone())
        }
    }
    write_receipt(
        bin,
        &Receipt {
            releases: next,
            skip_on_startup,
        },
    )?;
    publish(image, &path)?;
    Ok(InstalledCli {
        path,
        release: new.clone(),
    })
}

// This prefix is reserved for partial writes. Only prune named, previously recorded backups;
// never remove unrelated executables from a user's bin directory.
fn clean_staging(bin: &Path, list: &[Release], index: Option<usize>) -> Result<()> {
    for entry in fs::read_dir(bin)? {
        let e = entry?;
        if e.file_name().to_string_lossy().starts_with(".cli-stage-") && e.file_type()?.is_file() {
            fs::remove_file(e.path())?;
        }
    }
    if let Some(i) = index {
        for r in list {
            if r != &list[i] && Some(r) != list.get(i + 1) {
                match fs::remove_file(r.backup(bin)) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(())
}

/// Return a verified absolute path for local integration code. Never searches PATH or cwd.
pub fn locate(paths: &StorePaths) -> Result<InstalledCli> {
    let (bin, _lock) = paths.lock_cli_bin()?;
    let record = receipt(&bin)?;
    let list = &record.releases;
    let path = stable(&bin);
    let i = active(&path, list)?.ok_or(InstallError::NotInstalled)?;
    compatibility(paths, &list[i])?;
    Ok(InstalledCli {
        path,
        release: list[i].clone(),
    })
}

/// Install from the desktop's embedded CLI only. Automatic startup respects an explicit restore.
pub fn install(
    paths: &StorePaths,
    new: &Release,
    image: impl Read,
    automatic: bool,
) -> Result<InstalledCli> {
    new.validate()?;
    let (bin, _lock) = paths.lock_cli_bin()?;
    let record = receipt(&bin)?;
    let list = &record.releases;
    let path = stable(&bin);
    let i = active(&path, list)?;
    clean_staging(&bin, list, i)?;
    if let Some(i) = i {
        let old = &list[i];
        if old == new || (automatic && record.skip_on_startup.as_ref() == Some(&new.sha256)) {
            compatibility(paths, old)?;
            return Ok(InstalledCli {
                path,
                release: old.clone(),
            });
        }
        if semver::Version::parse(&new.version)
            .unwrap()
            .cmp_precedence(&semver::Version::parse(&old.version).unwrap())
            .is_lt()
        {
            if automatic {
                compatibility(paths, old)?;
                return Ok(InstalledCli {
                    path,
                    release: old.clone(),
                });
            }
            return Err(InstallError::Downgrade);
        }
    } else if list.len() > 1 {
        // A previously installed file disappeared. Do not discard the recovery copy.
        return Err(InstallError::UnverifiedInstallation);
    }
    let schema_version = compatibility(paths, new)?;
    let staged = staged_image(&bin, image, new)?;
    if i.is_none() {
        return switch(&bin, list, i, new, staged, None);
    }
    with_catalogue_backup(paths, schema_version, || {
        switch(&bin, list, i, new, staged, None)
    })
}

pub fn restore(paths: &StorePaths) -> Result<InstalledCli> {
    let (bin, _lock) = paths.lock_cli_bin()?;
    let record = receipt(&bin)?;
    let list = &record.releases;
    let path = stable(&bin);
    let i = match active(&path, list) {
        Ok(i) => i,
        Err(InstallError::UnverifiedInstallation) => None,
        Err(e) => return Err(e),
    };
    clean_staging(&bin, list, i)?;
    let prev = list
        .get(i.unwrap_or(0) + 1)
        .ok_or(InstallError::NoPrevious)?;
    let schema_version = compatibility(paths, prev)?;
    let backup = prev.backup(&bin);
    paths::check_file(&backup)?;
    let staged = staged_image(&bin, File::open(backup)?, prev)?;
    let skip = Some(list[i.unwrap_or(0)].sha256.clone());
    with_catalogue_backup(paths, schema_version, || {
        if i.is_none() {
            // No intact current file to archive. Keep the same recovery chain through
            // both the receipt update and rename, so the operation can be retried.
            write_receipt(
                &bin,
                &Receipt {
                    releases: list.clone(),
                    skip_on_startup: skip,
                },
            )?;
            publish(staged, &path)?;
            return Ok(InstalledCli {
                path,
                release: prev.clone(),
            });
        }
        switch(&bin, list, i, prev, staged, skip)
    })
}

#[cfg(all(test, any(target_os = "macos", windows)))]
#[path = "cli_install_tests.rs"]
mod tests;
