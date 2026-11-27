use crate::{Fault, Login, MAX_BLOB};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, ERROR_CANCELLED, ERROR_NOT_FOUND, GetLastError},
    Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    },
};
use zeroize::{Zeroize, Zeroizing};
fn name(service: &str, account: &str) -> Vec<u16> {
    format!("{service}/{account}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
fn last_error() -> Fault {
    match unsafe { GetLastError() } {
        ERROR_NOT_FOUND => Fault::Missing,
        ERROR_ACCESS_DENIED | ERROR_CANCELLED => Fault::AccessDenied,
        _ => Fault::Unavailable,
    }
}
pub(crate) fn write(service: &str, account: &str, data: &[u8]) -> Result<(), Fault> {
    let mut target = name(service, account);
    let item = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        CredentialBlobSize: data.len() as u32,
        CredentialBlob: data.as_ptr().cast_mut(),
        ..Default::default()
    };
    // Input buffers outlive this synchronous API. The database username lives inside the blob.
    if unsafe { CredWriteW(&item, 0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}
struct Returned(*mut CREDENTIALW);
impl Drop for Returned {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) }
    }
}
pub(crate) fn read(service: &str, account: &str) -> Result<Login, Fault> {
    let target = name(service, account);
    let mut raw = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
        return Err(last_error());
    }
    if raw.is_null() {
        return Err(Fault::Damaged);
    }
    let owned = Returned(raw);
    let item = unsafe { &*owned.0 };
    let len = item.CredentialBlobSize as usize;
    if len == 0 || len > MAX_BLOB || item.CredentialBlob.is_null() {
        return Err(Fault::Damaged);
    }
    // CredReadW owns one buffer (including its blob) until CredFree.
    let blob = unsafe { std::slice::from_raw_parts_mut(item.CredentialBlob, len) };
    let copy = Login(Zeroizing::new(blob.to_vec()));
    blob.zeroize();
    Ok(copy)
}
pub(crate) fn delete(service: &str, account: &str) -> Result<(), Fault> {
    let target = name(service, account);
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}
