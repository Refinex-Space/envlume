//! All project entries are fixed single components below directory handles.
use crate::config::hash;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
use envlume_application::integrations::Fault;
use envlume_domain::{ClientKind, IntegrationId};
#[cfg(windows)]
use std::{fs::File, path::PathBuf};
use std::{
    io::{self, Read, Write},
    path::Path,
};

pub(crate) const MAX: usize = 1024 * 1024;
pub(crate) fn relative(client: ClientKind) -> &'static Path {
    Path::new(match client {
        ClientKind::ClaudeCode => ".mcp.json",
        ClientKind::Cursor => ".cursor/mcp.json",
        ClientKind::Codex => ".codex/config.toml",
    })
}
pub(crate) fn fingerprint(bytes: Option<&[u8]>) -> String {
    bytes.map(hash).unwrap_or_else(|| "absent".into())
}
pub(crate) struct Target {
    dir: Dir,
    #[cfg(windows)]
    path: PathBuf,
    name: String,
}
impl Target {
    pub fn open(
        binding: &envlume_application::workspaces::BindingInfo,
        client: ClientKind,
        create: bool,
    ) -> Result<Option<Self>, Fault> {
        let root = &binding.canonical_path;
        let base = Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|_| Fault::UnsafePath)?;
        let id = envlume_workspace::directory_identity(
            &base
                .try_clone()
                .map_err(|_| Fault::UnsafePath)?
                .into_std_file(),
        )
        .map_err(|_| Fault::UnsafePath)?;
        if binding.root_identity.as_deref() != Some(&id) {
            return Err(Fault::UntrustedBinding);
        }
        let rel = relative(client);
        let parent = rel.parent().expect("fixed relative path");
        let dir = if parent.as_os_str().is_empty() {
            base
        } else {
            if create {
                match base.create_dir(parent) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(_) => return Err(Fault::Storage),
                }
            }
            if let Ok(meta) = base.symlink_metadata(parent) {
                if meta.file_type().is_symlink() {
                    return Err(Fault::UnsafePath);
                }
                #[cfg(windows)]
                {
                    use cap_std::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(Fault::UnsafePath);
                    }
                }
            }
            match base.open_dir_nofollow(parent) {
                Ok(d) => d,
                Err(e) if e.kind() == io::ErrorKind::NotFound && !create => {
                    if base.symlink_metadata(parent).is_ok() {
                        return Err(Fault::UnsafePath);
                    }
                    return Ok(None);
                }
                Err(_) => return Err(Fault::UnsafePath),
            }
        };
        Ok(Some(Self {
            dir,
            #[cfg(windows)]
            path: root.join(parent),
            name: rel.file_name().unwrap().to_str().unwrap().into(),
        }))
    }
    fn read_named(&self, name: &str) -> Result<Option<Vec<u8>>, Fault> {
        let mut opts = OpenOptions::new();
        opts.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        opts.custom_flags(libc::O_NONBLOCK);
        let f = match self.dir.open_with(name, &opts) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Fault::UnsafePath),
        };
        let meta = f.metadata().map_err(|_| Fault::Storage)?;
        if !meta.is_file() {
            return Err(Fault::UnsafePath);
        }
        if cap_fs_ext::MetadataExt::nlink(&meta) != 1 {
            return Err(Fault::UnsafePath);
        }
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(Fault::UnsafePath);
            }
        }
        let mut buf = Vec::new();
        f.take(MAX as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|_| Fault::Storage)?;
        if buf.len() > MAX {
            return Err(Fault::InvalidConfig);
        }
        Ok(Some(buf))
    }
    pub fn read(&self) -> Result<Option<Vec<u8>>, Fault> {
        self.read_named(&self.name)
    }
    fn sync(&self) -> Result<(), Fault> {
        #[cfg(unix)]
        self.dir
            .try_clone()
            .and_then(|d| d.into_std_file().sync_all())
            .map_err(|_| Fault::Storage)?;
        Ok(())
    }
    fn temp(&self, name: &str, data: &[u8], _existing: bool) -> Result<(), Fault> {
        if data.len() > MAX {
            return Err(Fault::InvalidConfig);
        }
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        #[cfg(windows)]
        opts.access_mode(0x40000000 | 0x00040000).share_mode(0);
        let mut f = self
            .dir
            .open_with(name, &opts)
            .map_err(|_| Fault::Storage)?
            .into_std();
        #[cfg(windows)]
        private_acl(&f)?;
        f.write_all(data).map_err(|_| Fault::Storage)?;
        #[cfg(unix)]
        if _existing {
            let mut ro = OpenOptions::new();
            ro.read(true)
                .follow(FollowSymlinks::No)
                .custom_flags(libc::O_NONBLOCK);
            let original = self
                .dir
                .open_with(&self.name, &ro)
                .map_err(|_| Fault::ConcurrentEdit)?
                .into_std();
            let meta = original.metadata().map_err(|_| Fault::Storage)?;
            if !meta.is_file() || meta.permissions().readonly() {
                return Err(Fault::UnsafePath);
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::fd::AsRawFd;
                // Preserve the old security descriptor on the replacement using open handles.
                if unsafe {
                    libc::fcopyfile(
                        original.as_raw_fd(),
                        f.as_raw_fd(),
                        std::ptr::null_mut(),
                        libc::COPYFILE_SECURITY,
                    )
                } != 0
                {
                    return Err(Fault::UnsafePath);
                }
            }
            #[cfg(target_os = "linux")]
            f.set_permissions(meta.permissions())
                .map_err(|_| Fault::UnsafePath)?;
        }
        f.sync_all().map_err(|_| Fault::Storage)
    }
    // Only the never-published writing file may contain a partial copy. A swap
    // or delete can leave an external version behind; verify its hash before cleanup.
    pub fn clear_interrupted(
        &self,
        row: &envlume_application::integrations::Registration,
    ) -> Result<(), Fault> {
        let prefix = format!(".envlume-{}", row.id.0);
        let displaced = format!("{prefix}.displaced");
        // ReplaceFileW can move the old file aside without publishing the new one
        // (ERROR_UNABLE_TO_MOVE_REPLACEMENT_2). Recover the captured source first.
        if self.read()?.is_none()
            && self.read_named(&format!("{prefix}.ready"))?.is_some()
            && self.read_named(&displaced)?.is_some()
        {
            self.new_name(&displaced, &self.name)?;
        }
        for ext in ["writing", "ready", "displaced", "late"] {
            let name = format!("{prefix}.{ext}");
            if let Some(bytes) = self.read_named(&name)? {
                let sum = hash(&bytes);
                if ext != "writing"
                    && ![&row.source, &row.original, &row.published, &row.removal]
                        .iter()
                        .any(|v| v.as_ref() == Some(&sum))
                {
                    return Err(Fault::RecoveryConflict);
                }
                self.dir.remove_file(&name).map_err(|_| Fault::Storage)?;
            }
        }
        self.sync()
    }
    // Capture the displaced inode during publication, then verify its contents. A
    // noncooperating editor racing the final hash check must not lose its version.
    pub fn replace(
        &self,
        token: IntegrationId,
        expected: &str,
        data: Option<&[u8]>,
    ) -> Result<(), Fault> {
        if fingerprint(self.read()?.as_deref()) != expected {
            return Err(Fault::ConcurrentEdit);
        }
        let temp = format!(".envlume-{}.ready", token.0);
        let writing = format!(".envlume-{}.writing", token.0);
        let displaced = format!(".envlume-{}.displaced", token.0);
        for suffix in ["writing", "ready", "displaced", "late"] {
            let scratch = format!(".envlume-{}.{suffix}", token.0);
            match self.dir.symlink_metadata(&scratch) {
                Ok(_) => return Err(Fault::RecoveryConflict),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(_) => return Err(Fault::UnsafePath),
            }
        }
        if let Some(bytes) = data {
            self.temp(&writing, bytes, expected != "absent")?;
            self.new_name(&writing, &temp)
                .map_err(|_| Fault::RecoveryConflict)?;
        }
        let publish = (|| {
            if fingerprint(self.read()?.as_deref()) != expected {
                return Err(Fault::ConcurrentEdit);
            }
            match (expected == "absent", data) {
                (true, Some(_)) => self.new_name(&temp, &self.name)?,
                (true, None) => (),
                (false, _) => {
                    let saved = if data.is_some() {
                        self.exchange(&temp, &displaced)?
                    } else {
                        self.new_name(&self.name, &displaced)?;
                        displaced.as_str()
                    };
                    if fingerprint(self.read_named(saved)?.as_deref()) != expected {
                        // Keep the captured external version if a second editor also changed
                        // the destination. Never clean up such a displaced file as staging.
                        if fingerprint(self.read()?.as_deref()) != fingerprint(data) {
                            return Err(Fault::RecoveryConflict);
                        }
                        if data.is_none() {
                            self.new_name(saved, &self.name)
                                .map_err(|_| Fault::RecoveryConflict)?;
                        } else {
                            let late = format!(".envlume-{}.late", token.0);
                            let prior = self
                                .exchange(saved, &late)
                                .map_err(|_| Fault::RecoveryConflict)?;
                            if fingerprint(self.read_named(prior)?.as_deref()) != fingerprint(data)
                            {
                                return Err(Fault::RecoveryConflict);
                            }
                            self.dir.remove_file(prior).map_err(|_| Fault::Storage)?;
                        }
                        return Err(Fault::ConcurrentEdit);
                    }
                    self.dir.remove_file(saved).map_err(|_| Fault::Storage)?;
                }
            }
            self.sync()
        })();
        // A swapped-out original may now occupy temp. On ambiguous errors preserve it.
        if matches!(publish, Err(Fault::ConcurrentEdit))
            && data.is_some()
            && self.read_named(&temp)?.as_deref() == data
        {
            self.dir.remove_file(&temp).map_err(|_| Fault::Storage)?;
        }
        publish
    }
    fn new_name(&self, source: &str, dest: &str) -> Result<(), Fault> {
        #[cfg(unix)]
        return self.rename_flags(source, dest, false);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
            let wide = |name: &str| {
                self.path
                    .join(name)
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            if unsafe {
                MoveFileExW(
                    wide(source).as_ptr(),
                    wide(dest).as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(Fault::ConcurrentEdit);
            }
            Ok(())
        }
    }
    #[cfg(unix)]
    fn rename_flags(&self, source: &str, dest: &str, swap: bool) -> Result<(), Fault> {
        use std::{ffi::CString, os::fd::AsRawFd};
        let fd = self.dir.as_raw_fd();
        let from = CString::new(source).unwrap();
        let to = CString::new(dest).unwrap();
        // Both names are fixed or locally generated, relative to the pinned directory.
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                fd,
                from.as_ptr(),
                fd,
                to.as_ptr(),
                if swap {
                    libc::RENAME_SWAP
                } else {
                    libc::RENAME_EXCL
                },
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                fd,
                from.as_ptr(),
                fd,
                to.as_ptr(),
                if swap {
                    libc::RENAME_EXCHANGE
                } else {
                    libc::RENAME_NOREPLACE
                },
            )
        };
        if result != 0 {
            return Err(Fault::ConcurrentEdit);
        }
        Ok(())
    }
    #[cfg(unix)]
    fn exchange<'a>(&self, source: &'a str, _backup: &'a str) -> Result<&'a str, Fault> {
        self.rename_flags(source, &self.name, true)?;
        Ok(source)
    }
    #[cfg(windows)]
    fn exchange<'a>(&self, source: &'a str, backup: &'a str) -> Result<&'a str, Fault> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
        let wide = |n: &str| {
            self.path
                .join(n)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let (from, to, bak) = (wide(source), wide(&self.name), wide(backup));
        // cap-std opens directory handles without FILE_SHARE_DELETE on Windows.
        if unsafe {
            ReplaceFileW(
                to.as_ptr(),
                from.as_ptr(),
                bak.as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(match io::Error::last_os_error().raw_os_error() {
                Some(1177) => Fault::RecoveryConflict,
                Some(32 | 33) => Fault::Busy,
                Some(2 | 3 | 80 | 183) => Fault::ConcurrentEdit,
                Some(5) => Fault::UnsafePath,
                _ => Fault::Storage,
            });
        }
        Ok(backup)
    }
}

#[cfg(windows)]
fn private_acl(file: &File) -> Result<(), Fault> {
    use std::{os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            SetKernelObjectSecurity,
        },
    };
    let sddl = "D:P(A;;FA;;;OW)"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut sd = ptr::null_mut();
    // Install a protected owner-rights DACL on the exclusive, still empty staging file.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut sd,
            ptr::null_mut(),
        ) == 0
        {
            return Err(Fault::Storage);
        }
        let ok = SetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            sd,
        );
        LocalFree(sd);
        if ok == 0 {
            return Err(Fault::Storage);
        }
    }
    Ok(())
}
