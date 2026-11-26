//! A CLI caller may disconnect without sending an explicit cancellation.
#[cfg(unix)]
fn input_closed() -> bool {
    let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(0, info.as_mut_ptr()) } != 0 {
        return true;
    }
    let kind = unsafe { info.assume_init() }.st_mode & libc::S_IFMT;
    if kind == libc::S_IFSOCK {
        // A half-closed socketpair can report readable EOF without POLLHUP.
        let mut byte = 0u8;
        return unsafe {
            libc::recv(
                0,
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        } == 0;
    }
    kind == libc::S_IFIFO && hung_up(0)
}
#[cfg(unix)]
fn hung_up(fd: libc::c_int) -> bool {
    let mut descriptor = libc::pollfd {
        fd,
        events: 0,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, 0) };
    descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
}
#[cfg(windows)]
fn input_closed() -> bool {
    use windows_sys::Win32::{
        Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED, GetLastError},
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
        System::{
            Console::{GetStdHandle, STD_INPUT_HANDLE},
            Pipes::PeekNamedPipe,
        },
    };
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        GetFileType(input) == FILE_TYPE_PIPE
            && PeekNamedPipe(
                input,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            && matches!(
                GetLastError(),
                ERROR_BROKEN_PIPE | ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED
            )
    }
}

#[cfg(unix)]
pub struct ParentWatch(libc::pid_t);
#[cfg(unix)]
impl ParentWatch {
    pub fn capture() -> std::io::Result<Self> {
        Ok(Self(unsafe { libc::getppid() }))
    }
    pub fn gone(&self) -> bool {
        unsafe { libc::getppid() != self.0 }
    }
}

#[cfg(windows)]
pub struct ParentWatch(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl ParentWatch {
    pub fn capture() -> std::io::Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                    TH32CS_SNAPPROCESS,
                },
                Threading::{GetCurrentProcessId, OpenProcess, PROCESS_SYNCHRONIZE},
            },
        };
        // SAFETY: snapshots and process handles are owned by RAII handles here.
        unsafe {
            let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if raw == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let snap = OwnedHandle::from_raw_handle(raw);
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut next = Process32FirstW(snap.as_raw_handle(), &mut entry);
            while next != 0 {
                if entry.th32ProcessID == GetCurrentProcessId() {
                    let parent = OpenProcess(PROCESS_SYNCHRONIZE, 0, entry.th32ParentProcessID);
                    if parent.is_null() {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(Self(OwnedHandle::from_raw_handle(parent)));
                }
                next = Process32NextW(snap.as_raw_handle(), &mut entry);
            }
        }
        Err(std::io::ErrorKind::NotFound.into())
    }
    pub fn gone(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(self.0.as_raw_handle(), 0)
                != windows_sys::Win32::Foundation::WAIT_TIMEOUT
        }
    }
}

/// A pre-closed input pipe is normal for a headless one-shot invocation.
/// Only an open caller pipe subsequently closing is a disconnect.
pub struct Caller {
    parent: ParentWatch,
    watch_input: bool,
}
impl Caller {
    pub fn capture() -> std::io::Result<Self> {
        Ok(Self {
            parent: ParentWatch::capture()?,
            watch_input: !input_closed(),
        })
    }
    pub fn disconnected(&self) -> bool {
        #[cfg(unix)]
        if hung_up(1) {
            return true;
        }
        self.parent.gone() || (self.watch_input && input_closed())
    }
}
