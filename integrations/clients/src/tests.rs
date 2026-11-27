use super::*;
use envlume_application::workspaces::add_project;
use envlume_storage::cli_install::Release;
use std::{
    cell::{Cell, RefCell},
    fs,
};
use tempfile::TempDir;
const SECRET: &str = "T15_SYNTHETIC_PRIVATE_VALUE";
struct Harness {
    _tmp: TempDir,
    paths: StorePaths,
    store: Store,
    root: std::path::PathBuf,
    binding: BindingId,
    client: ClientKind,
}
struct Probe {
    present: bool,
    verdict: Result<Verification, Fault>,
    on_call: RefCell<Option<Box<dyn FnOnce()>>>,
}
impl Default for Probe {
    fn default() -> Self {
        Self {
            present: true,
            verdict: Ok(Verification::Gateway),
            on_call: Default::default(),
        }
    }
}
impl Adapter for Probe {
    fn detected(&self, _: ClientKind, _: &Path) -> bool {
        self.present
    }
    fn verify(
        &self,
        _: ClientKind,
        _: &Path,
        cmd: &Launch,
        ctx: &BindingInfo,
    ) -> Result<Verification, Fault> {
        assert!(Path::new(&cmd.command).is_absolute());
        assert_eq!(
            cmd.args,
            ["mcp", "--binding", &ctx.binding_id.0.to_string()]
        );
        if let Some(f) = self.on_call.take() {
            f()
        }
        self.verdict
    }
}
impl Harness {
    fn new(client: ClientKind) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project 空间");
        fs::create_dir(&root).unwrap();
        let paths = StorePaths::at(tmp.path().join("state")).unwrap();
        let store = Store::open(paths.clone()).unwrap();
        let binding = add_project(&store, &root, None, |_, _| true)
            .unwrap()
            .binding_id;
        let mut bytes = vec![0; 256];
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
        let r = Release {
            version: "0.1.0".into(),
            target: cli_install::native_target().into(),
            max_schema: envlume_storage::SCHEMA_VERSION,
            sha256: config::hash(&bytes),
        };
        cli_install::install(&paths, &r, bytes.as_slice(), false).unwrap();
        Self {
            _tmp: tmp,
            root,
            paths,
            store,
            binding,
            client,
        }
    }
    fn path(&self) -> std::path::PathBuf {
        self.root.join(relative(self.client))
    }
    fn fixture(&self) -> Vec<u8> {
        let text = if self.client == ClientKind::Codex {
            format!(
                "# retain comment\r\n[custom]\r\nx = 1\r\n[mcp_servers.third]\r\ncommand = '{SECRET}'\r\n"
            )
        } else {
            format!(
                r#"{{ "mcpServers" : {{ "third": {{"token":"{SECRET}"}} }}, "opaque": 123456789012345678901234567890 }}"#
            )
        };
        fs::create_dir_all(self.path().parent().unwrap()).unwrap();
        fs::write(self.path(), &text).unwrap();
        text.into_bytes()
    }
    fn svc<'a>(&'a self, probe: &'a Probe) -> Service<'a> {
        Service {
            store: &self.store,
            paths: &self.paths,
            origin: &self.root,
            binding: self.binding,
            client: self.client,
            adapter: probe,
        }
    }
    fn row(&self) -> Option<Registration> {
        self.store
            .integration_record(self.binding, &self.root, self.client)
            .unwrap()
    }
    fn external(&self, managed: bool) -> Vec<u8> {
        let content = fs::read_to_string(self.path()).unwrap();
        let change = if managed {
            content.replace("--binding", "--changed")
        } else {
            content.replace(SECRET, "user-edited-third-party")
        };
        assert_ne!(content, change);
        fs::write(self.path(), &change).unwrap();
        change.into()
    }
}
#[test]
fn json_and_toml_lifecycle_preserve_third_party_data_and_private_backups() {
    for client in [
        ClientKind::Codex,
        ClientKind::Cursor,
        ClientKind::ClaudeCode,
    ] {
        let h = Harness::new(client);
        let original = h.fixture();
        let probe = Probe::default();
        let s = h.svc(&probe);
        let preview = s.execute(Action::Preview, |_| panic!()).unwrap();
        assert!(!serde_json::to_string(&preview).unwrap().contains(SECRET));
        assert_eq!(preview.preview.change, ConfigChange::Add);
        assert_eq!(
            s.execute(Action::Install, |_| false).unwrap_err(),
            Fault::Cancelled
        );
        assert_eq!(fs::read(h.path()).unwrap(), original);
        assert!(h.row().is_none());
        let installed = s.execute(Action::Install, |_| true).unwrap();
        assert!(installed.gateway_verified && !installed.client_verified);
        let first = h.row().unwrap();
        let backup = h
            .paths
            .backups()
            .join(format!("integration-{}.bak", first.id.0));
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(
            !fs::read(h.paths.database())
                .unwrap()
                .windows(SECRET.len())
                .any(|w| w == SECRET.as_bytes())
        );
        let wal = h.paths.database().with_extension("db-wal");
        if wal.exists() {
            assert!(
                !fs::read(wal)
                    .unwrap()
                    .windows(SECRET.len())
                    .any(|w| w == SECRET.as_bytes())
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(backup.metadata().unwrap().permissions().mode() & 0o077, 0);
        }
        #[cfg(windows)]
        {
            let ps = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            let script = r#"$ErrorActionPreference='Stop'; $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User; $acl=(Get-Acl -LiteralPath $env:T15_PRIVATE); $rules=$acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]); if ($rules.Count -ne 1 -or $rules[0].IdentityReference -ne $sid -or $rules[0].FileSystemRights -ne 'FullControl') { exit 1 }"#;
            assert!(
                std::process::Command::new(ps)
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .env("T15_PRIVATE", &backup)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let saved = fs::read(h.path()).unwrap();
        assert!(String::from_utf8_lossy(&saved).contains(SECRET));
        if client != ClientKind::Codex {
            assert!(String::from_utf8_lossy(&saved).contains("123456789012345678901234567890"))
        }
        s.execute(Action::Install, |_| true).unwrap();
        assert_eq!(h.row().unwrap().id, first.id);
        assert_eq!(fs::read(h.path()).unwrap(), saved);
        s.execute(Action::Uninstall, |_| true).unwrap();
        assert_eq!(fs::read(h.path()).unwrap(), original);
        assert!(h.row().is_none());
        s.execute(Action::Uninstall, |_| panic!()).unwrap();
    }
}
#[test]
fn refusal_and_concurrent_edits_keep_the_external_authors_changes() {
    for client in [ClientKind::Codex, ClientKind::Cursor] {
        let h = Harness::new(client);
        let original = h.fixture();
        let bad = Probe {
            verdict: Err(Fault::ClientRejected),
            ..Default::default()
        };
        assert_eq!(
            h.svc(&bad).execute(Action::Install, |_| true).unwrap_err(),
            Fault::ClientRejected
        );
        assert_eq!(fs::read(h.path()).unwrap(), original);
        assert!(h.row().is_none());
        let path = h.path();
        *bad.on_call.borrow_mut() = Some(Box::new(move || {
            let txt = fs::read_to_string(&path).unwrap();
            fs::write(path, txt.replace(SECRET, "outside-edit")).unwrap()
        }));
        assert_eq!(
            h.svc(&bad).execute(Action::Install, |_| true).unwrap_err(),
            Fault::ClientRejected
        );
        let after = fs::read_to_string(h.path()).unwrap();
        assert!(after.contains("outside-edit"));
        assert!(!after.contains("--binding"));
        let probe = Probe::default();
        let s = h.svc(&probe);
        assert_eq!(
            s.execute(Action::Install, |_| {
                fs::write(h.path(), &original).unwrap();
                true
            })
            .unwrap_err(),
            Fault::ConcurrentEdit
        );
        assert_eq!(fs::read(h.path()).unwrap(), original);
        s.execute(Action::Install, |_| true).unwrap();
        let edited = h.external(true);
        assert_eq!(
            s.execute(Action::Uninstall, |_| true).unwrap_err(),
            Fault::EntryChanged
        );
        assert_eq!(fs::read(h.path()).unwrap(), edited);
    }
}
#[test]
fn crash_after_publish_is_recoverable_by_a_new_service() {
    for client in [ClientKind::Cursor, ClientKind::Codex] {
        let h = Harness::new(client);
        let original = h.fixture();
        let interrupted = Probe::default();
        *interrupted.on_call.borrow_mut() = Some(Box::new(|| panic!("fixture interruption")));
        let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            h.svc(&interrupted).execute(Action::Install, |_| true)
        }));
        assert!(died.is_err());
        assert_eq!(h.row().unwrap().pending.as_deref(), Some("INSTALL"));
        let healthy = Probe::default();
        let svc = h.svc(&healthy);
        assert_eq!(
            svc.execute(Action::Preview, |_| panic!()).unwrap().state,
            "RECOVERY_REQUIRED"
        );
        assert_eq!(
            svc.execute(Action::Install, |_| true).unwrap().state,
            "RECOVERED"
        );
        assert_eq!(fs::read(h.path()).unwrap(), original);
        svc.execute(Action::Install, |_| true).unwrap();
        h.external(false);
        svc.execute(Action::Uninstall, |_| true).unwrap();
        let final_text = fs::read_to_string(h.path()).unwrap();
        assert!(
            final_text.contains("user-edited-third-party") && !final_text.contains("--binding")
        );
    }
}
#[test]
fn malformed_foreign_missing_and_unsafe_paths_fail_closed() {
    for (client, malformed, collision) in [
        (
            ClientKind::Cursor,
            "{",
            "{\"mcpServers\":{\"envlume\":{\"note\":\"third\"}}}",
        ),
        (
            ClientKind::Codex,
            "[broken",
            "[mcp_servers.envlume]\ncommand='third'",
        ),
    ] {
        let h = Harness::new(client);
        let probe = Probe::default();
        let c = Cell::new(false);
        let s = h.svc(&probe);
        h.fixture();
        for (text, err) in [
            (malformed, Fault::InvalidConfig),
            (collision, Fault::NameConflict),
        ] {
            fs::write(h.path(), text).unwrap();
            assert_eq!(
                s.execute(Action::Install, |_| {
                    c.set(true);
                    true
                })
                .unwrap_err(),
                err
            );
            assert!(!c.get());
            assert_eq!(fs::read_to_string(h.path()).unwrap(), text);
        }
        fs::remove_file(h.path()).unwrap();
        let absent = Probe {
            present: false,
            ..Default::default()
        };
        assert_eq!(
            h.svc(&absent)
                .execute(Action::Install, |_| panic!())
                .unwrap_err(),
            Fault::ClientUnavailable
        );
        s.execute(Action::Install, |_| true).unwrap();
        s.execute(Action::Uninstall, |_| true).unwrap();
        assert!(!h.path().exists());
        #[cfg(unix)]
        {
            let other = h._tmp.path().join("outside");
            fs::write(&other, "{}").unwrap();
            std::os::unix::fs::symlink(&other, h.path()).unwrap();
            assert_eq!(
                s.execute(Action::Install, |_| true).unwrap_err(),
                Fault::UnsafePath
            );
            assert_eq!(fs::read_to_string(other).unwrap(), "{}");
        }
    }
}

#[test]
fn interrupted_staging_bytes_are_removed_during_recovery() {
    let h = Harness::new(ClientKind::Cursor);
    let original = h.fixture();
    let probe = Probe::default();
    *probe.on_call.borrow_mut() = Some(Box::new(|| panic!("interrupted")));
    let _stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.svc(&probe).execute(Action::Install, |_| true)
    }))
    .unwrap_err();
    let r = h.row().unwrap();
    // Recreate the pre-rename crash boundary, including an incompletely copied staging file.
    fs::write(h.path(), &original).unwrap();
    let stage = h
        .path()
        .parent()
        .unwrap()
        .join(format!(".envlume-{}.writing", r.id.0));
    fs::write(&stage, &original[..original.len() / 2]).unwrap();
    assert_eq!(
        h.svc(&probe)
            .execute(Action::Uninstall, |_| true)
            .unwrap()
            .state,
        "RECOVERED"
    );
    assert!(!stage.exists());
    assert_eq!(fs::read(h.path()).unwrap(), original);
}
#[test]
fn external_changes_to_managed_fields_during_verification_are_never_reverted() {
    let h = Harness::new(ClientKind::Cursor);
    h.fixture();
    let p = h.path();
    let probe = Probe::default();
    *probe.on_call.borrow_mut() = Some(Box::new(move || {
        let s = fs::read_to_string(&p).unwrap();
        fs::write(p, s.replace("--binding", "changed-by-client")).unwrap();
    }));
    assert_eq!(
        h.svc(&probe)
            .execute(Action::Install, |_| true)
            .unwrap_err(),
        Fault::RecoveryConflict
    );
    assert!(
        fs::read_to_string(h.path())
            .unwrap()
            .contains("changed-by-client")
    );
    assert_eq!(
        h.svc(&probe)
            .execute(Action::Uninstall, |_| true)
            .unwrap_err(),
        Fault::RecoveryConflict
    );
}

#[cfg(unix)]
#[test]
fn atomic_replace_preserves_existing_project_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(ClientKind::Cursor);
    h.fixture();
    fs::set_permissions(h.path(), fs::Permissions::from_mode(0o640)).unwrap();
    let probe = Probe::default();
    let svc = h.svc(&probe);
    svc.execute(Action::Install, |_| true).unwrap();
    assert_eq!(
        h.path().metadata().unwrap().permissions().mode() & 0o777,
        0o640
    );
    svc.execute(Action::Uninstall, |_| true).unwrap();
    assert_eq!(
        h.path().metadata().unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn recovery_restores_a_displaced_source_before_cleaning_staging_files() {
    let h = Harness::new(ClientKind::Cursor);
    let original = h.fixture();
    let probe = Probe::default();
    *probe.on_call.borrow_mut() = Some(Box::new(|| panic!("interruption")));
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.svc(&probe).execute(Action::Install, |_| true)
    }))
    .unwrap_err();
    let id = h.row().unwrap().id.0;
    let parent = h.path().parent().unwrap().to_owned();
    fs::rename(h.path(), parent.join(format!(".envlume-{id}.ready"))).unwrap();
    fs::write(parent.join(format!(".envlume-{id}.displaced")), &original).unwrap();
    assert_eq!(
        h.svc(&probe)
            .execute(Action::Uninstall, |_| true)
            .unwrap()
            .state,
        "RECOVERED"
    );
    assert_eq!(fs::read(h.path()).unwrap(), original);
}
#[test]
fn pending_removal_distinguishes_interruption_before_and_after_publication() {
    for committed in [false, true] {
        let h = Harness::new(ClientKind::Codex);
        let original = h.fixture();
        let probe = Probe::default();
        let s = h.svc(&probe);
        s.execute(Action::Install, |_| true).unwrap();
        let mut r = h.row().unwrap();
        r.pending = Some("REMOVE".into());
        r.source = r.published.clone();
        r.removal = r.original.clone();
        s.save(&r).unwrap();
        if committed {
            fs::write(h.path(), &original).unwrap();
        }
        s.execute(Action::Uninstall, |_| true).unwrap();
        assert_eq!(h.row().is_none(), committed);
        s.execute(Action::Uninstall, |_| true).unwrap();
        assert_eq!(fs::read(h.path()).unwrap(), original);
    }
}

#[test]
fn exact_foreign_launch_is_not_adopted_and_client_attestation_is_separate() {
    let h = Harness::new(ClientKind::Cursor);
    h.fixture();
    let probe = Probe {
        verdict: Ok(Verification::Client),
        ..Default::default()
    };
    let s = h.svc(&probe);
    let outcome = s.execute(Action::Install, |_| true).unwrap();
    assert_eq!(outcome.state, "CONNECTED");
    assert!(outcome.client_verified);
    assert!(
        s.execute(Action::Detect, |_| panic!())
            .unwrap()
            .client_verified
    );
    let saved = fs::read(h.path()).unwrap();
    let mut r = h.row().unwrap();
    r.status = "REMOVED".into();
    s.save(&r).unwrap();
    assert_eq!(
        s.execute(Action::Install, |_| panic!()).unwrap_err(),
        Fault::NameConflict
    );
    assert_eq!(fs::read(h.path()).unwrap(), saved);
}

#[cfg(windows)]
#[test]
fn windows_sharing_conflict_and_junction_leave_existing_files_untouched() {
    use std::os::windows::fs::OpenOptionsExt;
    let h = Harness::new(ClientKind::Cursor);
    let original = h.fixture();
    let probe = Probe::default();
    let svc = h.svc(&probe);
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(h.path())
        .unwrap();
    assert!(svc.execute(Action::Install, |_| true).is_err());
    assert_eq!(fs::read(h.path()).unwrap(), original);
    drop(held);
    svc.execute(Action::Uninstall, |_| true).unwrap();
    let outside = h._tmp.path().join("outside");
    fs::rename(h.path().parent().unwrap(), &outside).unwrap();
    let cmd =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
    assert!(
        std::process::Command::new(cmd)
            .args(["/d", "/c", "mklink", "/J"])
            .arg(h.path().parent().unwrap())
            .arg(&outside)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        svc.execute(Action::Install, |_| true).unwrap_err(),
        Fault::UnsafePath
    );
    assert_eq!(fs::read(outside.join("mcp.json")).unwrap(), original);
}
