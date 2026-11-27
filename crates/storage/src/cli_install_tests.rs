use super::*;
use crate::Store;
use std::io::Cursor;
use tempfile::TempDir;

fn fixture() -> (TempDir, StorePaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::at(dir.path().join("环境 µ space")).unwrap();
    (dir, paths)
}

// Exercise the file transaction with two distinct native images, without executing test bytes.
fn image(tag: u8) -> (Release, Vec<u8>) {
    let mut bytes = vec![0u8; 256];
    if cfg!(windows) {
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
    } else {
        bytes[..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        let cpu: u32 = if cfg!(target_arch = "aarch64") {
            0x0100000c
        } else {
            0x01000007
        };
        bytes[4..8].copy_from_slice(&cpu.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
    }
    bytes.push(tag);
    (
        Release {
            version: format!("0.1.0-dev.{tag}"),
            target: native_target().into(),
            max_schema: crate::SCHEMA_VERSION,
            sha256: digest(bytes.as_slice()).unwrap(),
        },
        bytes,
    )
}
#[test]
fn verify_each_candidate_before_publishing_it() {
    let (_dir, p) = fixture();
    let (a, bytes) = image(1);
    let path = install(&p, &a, bytes.as_slice(), false).unwrap().path;
    let (mut b, mut wrong) = image(2);
    assert_eq!(
        install(&p, &b, &wrong[..80], false).unwrap_err(),
        InstallError::InvalidRelease
    );
    wrong[..4].fill(0);
    b.sha256 = digest(wrong.as_slice()).unwrap();
    assert_eq!(
        install(&p, &b, wrong.as_slice(), false).unwrap_err(),
        InstallError::InvalidRelease
    );
    assert_eq!(locate(&p).unwrap().release, a);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn stale_copy_and_interrupted_receipt_keep_the_last_active_version() {
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    let path = install(&p, &a, one.as_slice(), false).unwrap().path;
    let bin = path.parent().unwrap();
    // A killed copy: a partial temporary file. Later: receipt durable but rename still pending.
    let partial = bin.join(".cli-stage-killed");
    fs::write(&partial, &two[..67]).unwrap();
    fs::write(
        bin.join(RECEIPT),
        serde_json::to_vec(&Receipt {
            releases: vec![b.clone(), a.clone()],
            skip_on_startup: None,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(locate(&p).unwrap().release, a);
    assert_eq!(install(&p, &b, two.as_slice(), false).unwrap().path, path);
    assert!(!partial.exists());
    assert_eq!(restore(&p).unwrap().release, a);
    assert_eq!(install(&p, &b, two.as_slice(), true).unwrap().release, a);
    assert_eq!(fs::read(&path).unwrap(), one);
    assert_eq!(install(&p, &b, two.as_slice(), false).unwrap().release, b);
    fs::write(&path, b"corrupt active").unwrap();
    assert_eq!(
        locate(&p).unwrap_err(),
        InstallError::UnverifiedInstallation
    );
    assert_eq!(restore(&p).unwrap().release, a);
    assert_eq!(fs::read(path).unwrap(), one);
    assert_eq!(install(&p, &b, two.as_slice(), true).unwrap().release, a);
}
#[test]
fn recovery_checks_the_live_schema_without_migrating_it() {
    let (_dir, p) = fixture();
    let (mut old, one) = image(1);
    old.max_schema -= 1;
    let (new, two) = image(2);
    install(&p, &old, one.as_slice(), false).unwrap();
    install(&p, &new, two.as_slice(), false).unwrap();
    Store::open(p.clone()).unwrap();
    let c = connect(&p.database(), true).unwrap();
    c.execute(
        "INSERT INTO projects(id,name,created_at,updated_at) VALUES(randomblob(16),'retained',1,1)",
        [],
    )
    .unwrap();
    assert_eq!(restore(&p).unwrap_err(), InstallError::IncompatibleSchema);
    c.pragma_update(None, "user_version", crate::SCHEMA_VERSION + 1)
        .unwrap();
    let (next, three) = image(3);
    assert_eq!(
        install(&p, &next, three.as_slice(), false).unwrap_err(),
        InstallError::IncompatibleSchema
    );
    assert_eq!(locate(&p).unwrap_err(), InstallError::IncompatibleSchema);
    assert_eq!(
        c.query_row("SELECT name FROM projects", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retained"
    );
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::SCHEMA_VERSION + 1
    );
    let bin = p.root().join("bin");
    assert_eq!(fs::read(stable(&bin)).unwrap(), two);
    assert_eq!(fs::read(old.backup(&bin)).unwrap(), one);
}
struct BrokenCopy(bool);
impl Read for BrokenCopy {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::Error::other("fixture disconnected"));
        }
        self.0 = true;
        Cursor::new(b"partial").read(out)
    }
}
#[test]
fn interrupted_read_never_changes_entry_or_prior_backup() {
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    install(&p, &a, one.as_slice(), false).unwrap();
    install(&p, &b, two.as_slice(), false).unwrap();
    let (c, _) = image(3);
    assert!(install(&p, &c, BrokenCopy(false), false).is_err());
    assert_eq!(locate(&p).unwrap().release, b);
    assert_eq!(restore(&p).unwrap().release, a);
}

#[test]
fn unwritable_directory_preserves_active_and_recovery_bytes() {
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    install(&p, &a, one.as_slice(), false).unwrap();
    let path = install(&p, &b, two.as_slice(), false).unwrap().path;
    let bin = path.parent().unwrap();
    #[cfg(unix)]
    fn writable(path: &Path, yes: bool) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if yes { 0o700 } else { 0o500 }),
        )
        .unwrap();
    }
    #[cfg(windows)]
    fn writable(path: &Path, yes: bool) {
        let script = r#"
$ErrorActionPreference = 'Stop'
$dir=[IO.DirectoryInfo]::new($env:T14_TEST_BIN)
$acl=$dir.GetAccessControl()
$rule=[Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.WindowsIdentity]::GetCurrent().User,'CreateFiles','Deny')
if ($env:T14_TEST_ALLOW -eq 'true') { $acl.RemoveAccessRuleSpecific($rule) } else { $acl.AddAccessRule($rule) }
$dir.SetAccessControl($acl)
"#;
        let ps = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        assert!(
            std::process::Command::new(ps)
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env("T14_TEST_BIN", path)
                .env("T14_TEST_ALLOW", yes.to_string())
                .status()
                .unwrap()
                .success()
        );
    }
    writable(bin, false);
    let (c, three) = image(3);
    let result = install(&p, &c, three.as_slice(), false);
    writable(bin, true);
    assert!(matches!(
        result,
        Err(InstallError::AccessDenied
            | InstallError::UnsafePath
            | InstallError::StorageUnavailable)
    ));
    assert_eq!(fs::read(&path).unwrap(), two);
    assert_eq!(restore(&p).unwrap().release, a);
}
#[test]
fn concurrent_updates_leave_a_complete_verified_entry() {
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    install(&p, &a, one.as_slice(), false).unwrap();
    let start = std::sync::Barrier::new(2);
    // Both callers use the same migration/install lock and retry the same release.
    std::thread::scope(|workers| {
        let job = || {
            start.wait();
            install(&p, &b, two.as_slice(), false).unwrap()
        };
        let left = workers.spawn(job);
        let right = workers.spawn(job);
        assert_eq!(left.join().unwrap().path, right.join().unwrap().path);
    });

    assert_eq!(locate(&p).unwrap().release, b);
    assert_eq!(restore(&p).unwrap().release, a);
}

#[cfg(windows)]
#[test]
fn busy_replacement_preserves_the_existing_rollback_slot() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    let (c, three) = image(3);
    install(&p, &a, one.as_slice(), false).unwrap();
    let path = install(&p, &b, two.as_slice(), false).unwrap().path;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .unwrap();
    assert_eq!(
        install(&p, &c, three.as_slice(), false).unwrap_err(),
        InstallError::Busy
    );
    assert_eq!(locate(&p).unwrap().release, b);
    drop(held);
    assert_eq!(restore(&p).unwrap().release, a);
}

#[test]
fn update_snapshots_committed_wal_without_touching_the_catalogue() {
    let (_dir, p) = fixture();
    let (a, one) = image(1);
    let (b, two) = image(2);
    install(&p, &a, one.as_slice(), false).unwrap();
    Store::open(p.clone()).unwrap();
    let db = connect(&p.database(), true).unwrap();
    db.execute_batch("PRAGMA wal_autocheckpoint=0; INSERT INTO projects(id,name,created_at,updated_at) VALUES(randomblob(16),'before-cli-update',1,1);").unwrap();
    assert!(
        p.database()
            .with_extension("db-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    install(&p, &b, two.as_slice(), false).unwrap();
    install(&p, &b, two.as_slice(), true).unwrap();
    let backups = fs::read_dir(p.backups())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let copy = connect(&backups[0], false).unwrap();
    for c in [&db, &copy] {
        assert_eq!(schema::version(c).unwrap(), crate::SCHEMA_VERSION);
        assert_eq!(
            c.query_row("SELECT name FROM projects", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "before-cli-update"
        );
    }
}
