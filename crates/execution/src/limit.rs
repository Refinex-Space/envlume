use envlume_application::tasks::Template;
use envlume_domain::{BindingId, RequestId};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// Locks are held by the worker, across CLI processes. A fresh UUID prevents
/// a late cancel command from cancelling a subsequent run.
pub struct Slot {
    _lock: File,
    marker: PathBuf,
    stop: PathBuf,
}
const SLOTS: u8 = 2;
impl Slot {
    pub fn acquire(
        directory: &Path,
        binding: BindingId,
        template: Template,
        request: RequestId,
    ) -> io::Result<Self> {
        for n in 0..SLOTS {
            let prefix = format!("task-slot-{n}-");
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let lock = options.open(directory.join(format!("task-slot-{n}.lock")))?;
            match lock.try_lock() {
                Ok(()) => (),
                Err(fs::TryLockError::WouldBlock) => continue,
                Err(fs::TryLockError::Error(e)) => return Err(e),
            }
            // With the slot exclusively held, remove only prior run markers.
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(&prefix) && (name.ends_with(".run") || name.ends_with(".stop"))
                {
                    remove(&entry.path())?;
                }
            }
            let marker = directory.join(format!(
                "{prefix}{}-{}-{}.run",
                binding.0,
                template.key(),
                request.0
            ));
            let stop = marker.with_extension("stop");
            File::create_new(&marker)?;
            return Ok(Self {
                _lock: lock,
                marker,
                stop,
            });
        }
        Err(io::ErrorKind::WouldBlock.into())
    }
    pub fn cancelled(&self) -> io::Result<bool> {
        self.stop.try_exists()
    }
}
fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        let _ = remove(&self.marker);
        let _ = remove(&self.stop);
    }
}
pub fn cancel(directory: &Path, binding: BindingId, template: Template) -> io::Result<usize> {
    let mut count = 0;
    for n in 0..SLOTS {
        let prefix = format!("task-slot-{n}-{}-{}-", binding.0, template.key());
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !name.ends_with(".run") {
                continue;
            }
            let path = entry.path();
            match fs::rename(&path, path.with_extension("stop")) {
                Ok(()) => count += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(count)
}
