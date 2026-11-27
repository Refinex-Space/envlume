//! Local discovery only. Repository files provide hints, never authorization.
use file_id::{FileId, get_file_id};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub entry_path: PathBuf,
    pub canonical_path: PathBuf,
    #[serde(skip)]
    pub root_identity: String,
    #[serde(skip)]
    pub fingerprint: String,
    pub git_kind: &'static str,
    pub hints: Vec<&'static str>,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "unsupported workspace path or Git metadata",
    )
}

pub fn entry_path(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| c == Component::ParentDir) {
        return Err(invalid());
    }
    #[cfg(windows)]
    windows_paths::validate_input(path)?;
    let path = std::path::absolute(path)?;
    if path.to_str().is_none() {
        return Err(invalid());
    }
    Ok(path.components().collect())
}

/// Resolve a local filesystem object using the same Windows drive/reparse checks as binding discovery.
pub fn local_canonical(path: &Path) -> io::Result<PathBuf> {
    let path = entry_path(path)?;
    #[cfg(windows)]
    windows_paths::check_local_chain(&path, 0)?;
    path.canonicalize()
}

fn identity(path: &Path) -> io::Result<String> {
    format_identity(get_file_id(path)?, fs::metadata(path)?.created()?)
}

fn format_identity(id: FileId, created: std::time::SystemTime) -> io::Result<String> {
    let key = match id {
        FileId::Inode {
            device_id,
            inode_number,
        } => format!("u-{device_id:x}-{inode_number:x}"),
        FileId::HighRes {
            volume_serial_number,
            file_id,
        } => format!("w-{volume_serial_number:x}-{file_id:x}"),
        FileId::LowRes {
            volume_serial_number,
            file_index,
        } => format!("wl-{volume_serial_number:x}-{file_index:x}"),
    };
    let born = created.duration_since(UNIX_EPOCH).map_err(|_| invalid())?;
    Ok(format!("{key}-{}", born.as_nanos()))
}

/// Compare a pinned directory handle with the identity stored at registration.
pub fn directory_identity(file: &fs::File) -> io::Result<String> {
    let meta = file.metadata()?;
    if !meta.is_dir() {
        return Err(invalid());
    }
    #[cfg(unix)]
    let id = {
        use std::os::unix::fs::MetadataExt;
        FileId::new_inode(meta.dev(), meta.ino())
    };
    #[cfg(windows)]
    let id = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx,
        };
        let mut high = FILE_ID_INFO::default();
        // Both queries write a fixed-size OS structure to caller-owned storage.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut high as *mut FILE_ID_INFO).cast(),
                std::mem::size_of_val(&high) as u32,
            )
        } != 0
        {
            FileId::new_high_res(
                high.VolumeSerialNumber,
                u128::from_le_bytes(high.FileId.Identifier),
            )
        } else {
            let mut low = BY_HANDLE_FILE_INFORMATION::default();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut low) } == 0 {
                return Err(io::Error::last_os_error());
            }
            FileId::new_low_res(
                low.dwVolumeSerialNumber,
                (u64::from(low.nFileIndexHigh) << 32) | u64::from(low.nFileIndexLow),
            )
        }
    };
    format_identity(id, meta.created()?)
}

pub fn discover(path: &Path) -> io::Result<Workspace> {
    let entry_path = entry_path(path)?;
    let canonical_path = local_canonical(&entry_path)?;
    if !canonical_path.is_dir() || canonical_path.to_str().is_none() {
        return Err(invalid());
    }
    let root_identity = identity(&canonical_path)?;
    let mut hash = Sha256::new();
    hash.update(root_identity.as_bytes());
    let git = canonical_path.join(".git");
    let git_kind = match fs::symlink_metadata(&git) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => "none",
        Err(e) => return Err(e),
        Ok(m) if is_link(&m) => return Err(invalid()),
        Ok(m) if m.is_dir() => {
            hash.update(identity(&git)?.as_bytes());
            "repository"
        }
        Ok(m) if m.is_file() => {
            // Keep a gitfile's contents as a hint. It may point outside the selected workspace.
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
                options.custom_flags(
                    windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
                );
            }
            let file = options.open(&git)?;
            let m = file.metadata()?;
            if !m.is_file() || is_link(&m) {
                return Err(invalid());
            }
            let mut bytes = Vec::new();
            file.take(8193).read_to_end(&mut bytes)?;
            if bytes.len() > 8192 {
                return Err(invalid());
            }
            let pointer = std::str::from_utf8(&bytes)
                .map_err(|_| invalid())?
                .trim_end_matches(['\r', '\n'])
                .strip_prefix("gitdir: ")
                .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                .ok_or_else(invalid)?;
            hash.update(&bytes);
            if Path::new(pointer)
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "worktrees")
            {
                "worktree"
            } else {
                "gitfile"
            }
        }
        _ => return Err(invalid()),
    };
    hash.update(git_kind.as_bytes());
    let mut hints = Vec::new();
    for (name, hint) in [
        ("pom.xml", "maven"),
        ("package.json", "node"),
        ("build.gradle", "gradle"),
        ("build.gradle.kts", "gradle"),
        ("pyproject.toml", "python"),
        ("Cargo.toml", "rust"),
    ] {
        if fs::symlink_metadata(canonical_path.join(name))
            .is_ok_and(|m| m.is_file() && !is_link(&m))
            && !hints.contains(&hint)
        {
            hints.push(hint);
        }
    }
    Ok(Workspace {
        entry_path,
        canonical_path,
        root_identity,
        fingerprint: format!("{:x}", hash.finalize()),
        git_kind,
        hints,
    })
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

#[cfg(windows)]
mod windows_paths {
    use super::*;
    use std::path::Prefix;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    pub(super) fn validate_input(path: &Path) -> io::Result<()> {
        match path.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) if path.is_absolute() => (),
                _ => return Err(invalid()),
            },
            Some(Component::RootDir) => return Err(invalid()),
            _ => (),
        }
        // Do not let Win32 reinterpret an apparently ordinary component as
        // a parent directory, an alternate data stream, or a DOS device.
        for component in path.components() {
            if let Component::Normal(name) = component {
                let name = name.to_str().ok_or_else(invalid)?;
                let stem = name
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim_end()
                    .to_ascii_uppercase();
                let serial = stem
                    .strip_prefix("COM")
                    .or_else(|| stem.strip_prefix("LPT"));
                if name.contains(':')
                    || name.ends_with([' ', '.'])
                    || matches!(
                        stem.as_str(),
                        "NUL" | "CON" | "PRN" | "AUX" | "CONIN$" | "CONOUT$"
                    )
                    || serial.is_some_and(|n| {
                        matches!(
                            n,
                            "0" | "1"
                                | "2"
                                | "3"
                                | "4"
                                | "5"
                                | "6"
                                | "7"
                                | "8"
                                | "9"
                                | "¹"
                                | "²"
                                | "³"
                        )
                    })
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_local_chain(path: &Path, depth: usize) -> io::Result<()> {
        validate_input(path)?;
        if depth > 32 {
            return Err(invalid());
        }
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Err(invalid());
        };
        let (Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)) = prefix.kind() else {
            return Err(invalid());
        };
        let root = [u16::from(drive), 58, 92, 0];
        // SAFETY: root is a NUL-terminated UTF-16 drive root. Only local disks qualify.
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 6) {
            return Err(invalid());
        }
        let mut current = PathBuf::new();
        let mut components = path.components();
        while let Some(c) = components.next() {
            current.push(c);
            if matches!(c, Component::Normal(_)) && is_link(&fs::symlink_metadata(&current)?) {
                let target = fs::read_link(&current)?;
                validate_input(&target)?;
                let target = current
                    .parent()
                    .ok_or_else(invalid)?
                    .join(target)
                    .join(components.as_path());
                return check_local_chain(&target, depth + 1);
            }
        }
        Ok(())
    }
}
