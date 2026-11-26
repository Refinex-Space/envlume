//! Keep the process tree under control from the first instruction of the child.
use std::{
    io,
    process::{ChildStderr, ChildStdout, Command, ExitStatus},
};

#[cfg(unix)]
pub(super) struct Tree {
    child: std::process::Child,
    reaped: bool,
}
#[cfg(unix)]
impl Tree {
    pub fn spawn(cmd: &mut Command) -> io::Result<Self> {
        use std::os::unix::process::CommandExt;
        Ok(Self {
            child: cmd.process_group(0).spawn()?,
            reaped: false,
        })
    }
    pub fn output(&mut self) -> (ChildStdout, ChildStderr) {
        (
            self.child.stdout.take().expect("piped stdout"),
            self.child.stderr.take().expect("piped stderr"),
        )
    }
    pub fn finished(&mut self) -> io::Result<Option<ExitStatus>> {
        // Observe exit without releasing the PID. This reserves the process-group
        // identity until leftover descendants have been signalled.
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { info.assume_init().si_pid() } == 0 {
            return Ok(None);
        }
        self.stop().map(Some)
    }
    pub fn stop(&mut self) -> io::Result<ExitStatus> {
        let group = -(self.child.id() as i32);
        if !self.reaped && unsafe { libc::kill(group, libc::SIGKILL) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                // Darwin returns EPERM for a group containing only its zombie
                // leader. Accept that case only after reaping our own child and
                // verifying that no group remains. Never signal a recycled PID.
                if error.raw_os_error() != Some(libc::EPERM) {
                    return Err(error);
                }
                let Some(status) = self.child.try_wait()? else {
                    return Err(error);
                };
                self.reaped = true;
                if unsafe { libc::kill(group, 0) } < 0
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    return Ok(status);
                }
                return Err(error);
            }
        }
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
}
#[cfg(unix)]
impl Drop for Tree {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.stop();
        }
    }
}

#[cfg(windows)]
pub(super) struct Tree {
    child: Box<dyn process_wrap::std::ChildWrapper>,
}
#[cfg(windows)]
impl Tree {
    pub fn spawn(cmd: Command) -> io::Result<Self> {
        use process_wrap::std::{CommandWrap, JobObject, KillOnDrop};
        // JobObject creates the process suspended, assigns it, then resumes it.
        let child = CommandWrap::from(cmd)
            .wrap(JobObject)
            .wrap(KillOnDrop)
            .spawn()?;
        Ok(Self { child })
    }
    pub fn output(&mut self) -> (ChildStdout, ChildStderr) {
        (
            self.child.stdout().take().unwrap(),
            self.child.stderr().take().unwrap(),
        )
    }
    pub fn finished(&mut self) -> io::Result<Option<ExitStatus>> {
        // A build launcher may exit while a background child still owns pipes.
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0},
            System::Threading::WaitForSingleObject,
        };
        // KillOnDrop owns the JobObjectChild and does not expose its process
        // handle. Borrow the native leader while retaining both wrappers.
        let handle = self
            .child
            .try_inner_child()
            .ok_or(io::ErrorKind::Unsupported)?;
        match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => return self.stop().map(Some),
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => (),
        }
        Ok(None)
    }
    pub fn stop(&mut self) -> io::Result<ExitStatus> {
        self.child.start_kill()?;
        self.child.wait()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn observes_exit_without_reusing_a_reaped_pid() {
        let mut command = Command::new("/usr/bin/true");
        let mut tree = Tree::spawn(&mut command).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(status) = tree.finished().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
