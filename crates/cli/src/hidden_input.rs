// Disable echo before publishing the prompt, including the interval before the
// password reader enters its own raw mode. Use its same controlling terminal.
use envlume_application::credentials::Fault;
use envlume_secrets::Login;
use std::io::{self, Write};
use zeroize::Zeroizing;

#[cfg(unix)]
mod terminal {
    use std::{fs::File, io, os::fd::AsRawFd};
    pub struct Guard {
        file: File,
        mode: libc::termios,
    }
    impl Guard {
        pub fn open() -> io::Result<Self> {
            let file = File::open("/dev/tty")?;
            let mut original = std::mem::MaybeUninit::uninit();
            if unsafe { libc::tcgetattr(file.as_raw_fd(), original.as_mut_ptr()) } == -1 {
                return Err(io::Error::last_os_error());
            }
            let mode = unsafe { original.assume_init() };
            let mut hidden = mode;
            hidden.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ISIG);
            if unsafe { libc::tcsetattr(file.as_raw_fd(), libc::TCSAFLUSH, &hidden) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { file, mode })
        }
        pub fn restore(&self) -> io::Result<()> {
            if unsafe { libc::tcsetattr(self.file.as_raw_fd(), libc::TCSANOW, &self.mode) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}
#[cfg(windows)]
mod terminal {
    use std::{
        fs::{File, OpenOptions},
        io,
        os::windows::io::AsRawHandle,
    };
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_PROCESSED_INPUT, GetConsoleMode, SetConsoleMode,
    };
    pub struct Guard {
        file: File,
        mode: u32,
    }
    impl Guard {
        pub fn open() -> io::Result<Self> {
            let file = OpenOptions::new().read(true).write(true).open("CONIN$")?;
            let mut mode = 0;
            if unsafe { GetConsoleMode(file.as_raw_handle(), &mut mode) } == 0
                || unsafe {
                    SetConsoleMode(
                        file.as_raw_handle(),
                        mode & !(ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT),
                    )
                } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { file, mode })
        }
        pub fn restore(&self) -> io::Result<()> {
            if unsafe { SetConsoleMode(self.file.as_raw_handle(), self.mode) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}
impl Drop for terminal::Guard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn field(prompt: &str) -> io::Result<Zeroizing<String>> {
    let mut terminal = io::stderr().lock();
    write!(terminal, "{prompt}")?;
    terminal.flush()?;
    rpassword::read_password().map(Zeroizing::new)
}
pub(super) fn login() -> Result<Login, Fault> {
    let hidden = terminal::Guard::open().map_err(|_| Fault::AccessDenied)?;
    let user = field("Database username (hidden): ").map_err(|_| Fault::AccessDenied)?;
    let password = field("Database password (hidden): ").map_err(|_| Fault::AccessDenied)?;
    hidden.restore().map_err(|_| Fault::Unavailable)?;
    Login::new(&user, &password)
}
