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

fn identity(path: &Path) -> io::Result<String> {
    let key = match get_file_id(path)? {
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
    let born = fs::metadata(path)?
        .created()?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?;
    Ok(format!("{key}-{}", born.as_nanos()))
}

pub fn discover(path: &Path) -> io::Result<Workspace> {
    let entry_path = entry_path(path)?;
    #[cfg(windows)]
    windows_paths::check_local_chain(&entry_path, 0)?;
    let canonical_path = entry_path.canonicalize()?;
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
                .components()
                .any(|p| p.as_os_str() == "worktrees")
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
        // Reject alternate data streams and device spellings before probing a path.
        if path
            .components()
            .any(|c| matches!(c,Component::Normal(s) if s.to_string_lossy().contains(':')))
        {
            return Err(invalid());
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
