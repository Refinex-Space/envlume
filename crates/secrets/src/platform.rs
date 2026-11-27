#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(super) use macos::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::*;

#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported {
    use crate::{Fault, Login};
    pub(crate) fn write(_: &str, _: &str, _: &[u8]) -> Result<(), Fault> {
        Err(Fault::Unavailable)
    }
    pub(crate) fn read(_: &str, _: &str) -> Result<Login, Fault> {
        Err(Fault::Unavailable)
    }
    pub(crate) fn delete(_: &str, _: &str) -> Result<(), Fault> {
        Err(Fault::Unavailable)
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
pub(super) use unsupported::*;
