use crate::{Fault, Login};
use security_framework::{
    base::Error,
    passwords::{PasswordOptions, delete_generic_password, generic_password, set_generic_password},
};
use zeroize::Zeroizing;

fn classify(e: Error) -> Fault {
    match e.code() {
        -25300 => Fault::Missing,             // errSecItemNotFound
        -128 | -25293 => Fault::AccessDenied, // userCanceledErr, errSecAuthFailed
        -26275 => Fault::Damaged,             // errSecDecode
        _ => Fault::Unavailable,              // includes locked/noninteractive keychain
    }
}
pub(crate) fn write(service: &str, account: &str, data: &[u8]) -> Result<(), Fault> {
    set_generic_password(service, account, data).map_err(classify)
}
pub(crate) fn read(service: &str, account: &str) -> Result<Login, Fault> {
    generic_password(PasswordOptions::new_generic_password(service, account))
        .map(|bytes| Login(Zeroizing::new(bytes)))
        .map_err(classify)
}
pub(crate) fn delete(service: &str, account: &str) -> Result<(), Fault> {
    delete_generic_password(service, account).map_err(classify)
}
