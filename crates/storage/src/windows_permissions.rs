//! Inspect an existing private catalogue by handle, without shell or module discovery.
//! Initial creation still uses private-directories.ps1 with an explicit protected DACL.
use crate::{Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    io,
    mem::{align_of, size_of},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_TOKEN, ERROR_SUCCESS, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
        INHERIT_ONLY_ACE, IsValidAcl, IsValidSid, NO_PROPAGATE_INHERIT_ACE, OBJECT_INHERIT_ACE,
        OWNER_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, READ_CONTROL,
    },
    System::{
        SystemServices::ACCESS_ALLOWED_ACE_TYPE,
        Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
    },
};

struct User(Vec<usize>);
impl User {
    fn current() -> Result<Self> {
        let mut token = ptr::null_mut();
        // Prefer the effective thread identity, matching WindowsIdentity.GetCurrent().
        // The successful call transfers one real token handle to OwnedHandle.
        unsafe {
            if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) == 0
                && (io::Error::last_os_error().raw_os_error() != Some(ERROR_NO_TOKEN as i32)
                    || OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0)
            {
                return Err(Error::PrivateDirectoryRequired);
            }
            let token = OwnedHandle::from_raw_handle(token);
            let mut needed = 0;
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &mut needed,
            );
            if !(size_of::<TOKEN_USER>()..=65536).contains(&(needed as usize)) {
                return Err(Error::PrivateDirectoryRequired);
            }
            const { assert!(align_of::<usize>() >= align_of::<TOKEN_USER>()) };
            let mut data = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
            if GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                data.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                return Err(Error::PrivateDirectoryRequired);
            }
            Ok(Self(data))
        }
    }
    fn matches(&self, sid: PSID) -> bool {
        // Both SIDs belong to live OS-provided descriptors/buffers.
        unsafe {
            !sid.is_null()
                && IsValidSid(sid) != 0
                && EqualSid((*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid, sid) != 0
        }
    }
}

fn check_acl(file: &File, directory: bool, user: &User) -> Result<()> {
    let (mut owner, mut acl, mut descriptor) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    // The owner and ACL borrow the returned self-relative security descriptor.
    // LocalFree below is paired with GetSecurityInfo on every success path.
    let ok = unsafe {
        if GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        ) != ERROR_SUCCESS
        {
            return Err(Error::PrivateDirectoryRequired);
        }
        let valid = (|| {
            if acl.is_null()
                || IsValidAcl(acl) == 0
                || (*acl).AceCount != 1
                || (directory && !user.matches(owner))
            {
                return false;
            }
            let mut raw = ptr::null_mut();
            if GetAce(acl, 0, &mut raw) == 0 {
                return false;
            }
            let header = &*raw.cast::<windows_sys::Win32::Security::ACE_HEADER>();
            if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE
                || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>()
            {
                return false;
            }
            let ace = &*raw.cast::<ACCESS_ALLOWED_ACE>();
            let flags = u32::from(header.AceFlags);
            ace.Mask == FILE_ALL_ACCESS
                && user.matches(ptr::addr_of!(ace.SidStart).cast_mut().cast())
                && flags & INHERIT_ONLY_ACE == 0
                && (!directory
                    || flags
                        & (CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE | NO_PROPAGATE_INHERIT_ACE)
                        == (CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE))
        })();
        LocalFree(descriptor);
        valid
    };
    if ok {
        Ok(())
    } else {
        Err(Error::PrivateDirectoryRequired)
    }
}

fn exists_private(path: &Path, directory: bool, user: &User) -> Result<bool> {
    let file = match OpenOptions::new()
        .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(Error::PrivateDirectoryRequired),
    };
    let meta = file.metadata().map_err(|_| Error::StorageUnavailable)?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
    {
        return Err(Error::PrivateDirectoryRequired);
    }
    check_acl(&file, directory, user)?;
    Ok(true)
}

pub(super) fn existing(root: &Path, cli: bool) -> Result<bool> {
    let user = User::current()?;
    let dirs: &[&str] = if cli {
        &["", "data", "bin"]
    } else {
        &["", "data", "logs", "tmp", "backups"]
    };
    for name in dirs {
        let dir = if name.is_empty() {
            root.to_path_buf()
        } else {
            root.join(name)
        };
        if !exists_private(&dir, true, &user)? {
            return Ok(false);
        }
    }
    let db = root.join("data").join("envlume.db");
    for ext in ["db", "db-wal", "db-shm", "db-journal"] {
        exists_private(&db.with_extension(ext), false, &user)?;
    }
    if cli {
        for item in fs::read_dir(root.join("bin")).map_err(|_| Error::StorageUnavailable)? {
            let item = item.map_err(|_| Error::StorageUnavailable)?;
            if !item
                .file_type()
                .map_err(|_| Error::StorageUnavailable)?
                .is_dir()
            {
                exists_private(&item.path(), false, &user)?;
            }
        }
    }
    Ok(true)
}
