//! Native project configuration transactions shared by desktop and CLI.
mod claude;
mod layout;
use layout::Layout;
mod command;
mod config;
mod file;
mod probe;
pub use command::command;
use config::{Edit, signature};
use envlume_application::{
    integrations::{
        Action, Adapter, ConfigChange, Fault, Launch, Registration, Verification, client_name,
    },
    workspaces::BindingInfo,
};
use envlume_domain::{BindingId, ClientKind, IntegrationId};
use envlume_storage::{Store, StorePaths, cli_install};
use file::{Target, fingerprint, relative};
pub use probe::LocalAdapter;
use serde::Serialize;
use std::path::Path;

/// Fixed phase names only: never log configuration, paths, arguments or client replies.
fn trace(phase: &'static str) {
    if std::env::var_os("RUST_LOG").as_deref()
        == Some(std::ffi::OsStr::new("envlume_clients=debug"))
    {
        eprintln!("envlume_clients: {phase}");
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub client: &'static str,
    pub scope: &'static str,
    pub location: String,
    pub binding: BindingId,
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    pub backup_ref: Option<IntegrationId>,
    pub key: &'static str,
    pub change: ConfigChange,
    pub launch: Option<Launch>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub preview: Preview,
    pub state: &'static str,
    pub gateway_verified: bool,
    pub client_verified: bool,
}
impl Outcome {
    fn new(preview: Preview, state: &'static str) -> Self {
        Self {
            preview,
            state,
            gateway_verified: false,
            client_verified: false,
        }
    }
}

/// No client-controlled paths: resolve the trusted origin for every phase and use a fixed layout.
pub struct Service<'a> {
    pub store: &'a Store,
    pub paths: &'a StorePaths,
    pub origin: &'a Path,
    pub binding: BindingId,
    pub client: ClientKind,
    pub adapter: &'a dyn Adapter,
    pub shared_project: bool,
}
struct Plan {
    view: Preview,
    layout: Layout,
    root: BindingInfo,
    row: Option<Registration>,
    before: Option<Vec<u8>>,
}
impl Service<'_> {
    fn plan(&self, action: Action) -> Result<Plan, Fault> {
        let root = self.store.integration_binding(self.binding, self.origin)?;
        let row = self
            .store
            .integration_record(self.binding, self.origin, self.client)?;
        let layout = Layout::resolve(self, action, &root, row.as_ref())?;
        let location = layout.location(&root);
        if row
            .as_ref()
            .is_some_and(|r| r.location != location || r.original.is_none())
        {
            return Err(Fault::NameConflict);
        }
        let target = layout.open(&root, false)?;
        let before = target.map(|t| t.read()).transpose()?.flatten();
        let entry = layout.document(before.as_deref(), Edit::Inspect)?.entry;
        let (detected, version) = match &layout {
            Layout::Project(_) => (
                self.adapter.detected(self.client, &root.canonical_path),
                None,
            ),
            Layout::Local { client, .. } => (
                client.is_some() || claude::Client::available(&root.canonical_path),
                client.as_ref().map(|c| c.version.clone()),
            ),
        };
        let change = ConfigChange::for_entry(action, row.as_ref(), entry.as_deref());
        let launch = if matches!(
            action,
            Action::Detect | Action::Uninstall | Action::PreviewUninstall
        ) || matches!(
            change,
            ConfigChange::Recover | ConfigChange::Edited | ConfigChange::Conflict
        ) {
            None
        } else {
            let cli = cli_install::locate(self.paths).map_err(|_| Fault::CliUnavailable)?;
            Some(Launch {
                command: cli.path.to_str().ok_or(Fault::UnsafePath)?.into(),
                args: vec!["mcp".into(), "--binding".into(), self.binding.0.to_string()],
            })
        };
        Ok(Plan {
            view: Preview {
                client: client_name(self.client),
                scope: layout.scope(),
                location: if layout.project_key().is_some() {
                    location.to_string_lossy().into_owned()
                } else {
                    relative(self.client).to_string_lossy().into_owned()
                },
                project_key: layout.project_key().map(str::to_owned),
                client_version: version,
                binding: self.binding,
                detected,
                backup_ref: row
                    .as_ref()
                    .filter(|r| r.original.as_deref() != Some("absent"))
                    .map(|r| r.id),
                key: if layout.project_key().is_some() {
                    "projects[projectKey].mcpServers.envlume"
                } else if self.client == ClientKind::Codex {
                    "mcp_servers.envlume"
                } else {
                    "mcpServers.envlume"
                },
                change,
                launch,
            },
            root,
            row,
            before,
            layout,
        })
    }

    fn removal(
        &self,
        layout: &Layout,
        r: &Registration,
        before: Option<&[u8]>,
    ) -> Result<Option<Vec<u8>>, Fault> {
        let doc = layout.document(before, Edit::Inspect)?;
        if doc.entry.as_deref() != Some(&r.fingerprint) {
            return Err(Fault::EntryChanged);
        }
        if r.published.as_deref() == Some(&fingerprint(before)) {
            if r.original.as_deref() == Some("absent") {
                return Ok(None);
            }
            let backup = self.paths.load_config_backup(r.id, file::MAX)?;
            let bytes = layout.restore_backup(before.ok_or(Fault::RecoveryConflict)?, backup)?;
            if Some(&fingerprint(Some(&bytes))) != r.original.as_ref() {
                return Err(Fault::RecoveryConflict);
            }
            return Ok(Some(bytes));
        }
        Ok(Some(layout.document(before, Edit::Remove)?.bytes))
    }
    fn publish(
        &self,
        t: &Target,
        id: IntegrationId,
        expected: &str,
        after: Option<&[u8]>,
    ) -> Result<(), Fault> {
        self.store
            .with_integration_binding(self.binding, self.origin, || t.replace(id, expected, after))
    }
    fn save(&self, r: &Registration) -> Result<(), Fault> {
        self.store.put_integration(self.origin, r)
    }
    fn finish_removed(&self, r: &mut Registration) -> Result<(), Fault> {
        r.source = None;
        r.pending = None;
        r.removal = None;
        r.verification = None;
        r.status = "REMOVED".into();
        self.save(r)
    }
    fn recover(
        &self,
        layout: &Layout,
        root: &BindingInfo,
        r: &mut Registration,
    ) -> Result<(), Fault> {
        self.store.integration_binding(self.binding, self.origin)?;
        let t = layout.open(root, false)?;
        if let Some(t) = &t {
            t.clear_interrupted(r)?;
        }
        let now = t.as_ref().map(|f| f.read()).transpose()?.flatten();
        let sum = fingerprint(now.as_deref());
        if r.pending.as_deref() == Some("REMOVE") {
            if r.removal.as_deref() == Some(&sum)
                || layout
                    .document(now.as_deref(), Edit::Inspect)?
                    .entry
                    .is_none()
            {
                return self.finish_removed(r);
            }
            if layout
                .document(now.as_deref(), Edit::Inspect)?
                .entry
                .as_deref()
                != Some(&r.fingerprint)
            {
                return Err(Fault::RecoveryConflict);
            }
            r.source = None;
            r.pending = None;
            r.removal = None;
            return self.save(r);
        }
        if r.original.as_deref() != Some(&sum) {
            let entry = layout.document(now.as_deref(), Edit::Inspect)?.entry;
            if entry.is_some() {
                let restore = self
                    .removal(layout, r, now.as_deref())
                    .map_err(|_| Fault::RecoveryConflict)?;
                self.paths.save_change_backup(
                    r.id,
                    &layout.change_backup(
                        now.as_deref().ok_or(Fault::RecoveryConflict)?,
                        &r.fingerprint,
                    )?,
                )?;
                self.publish(
                    t.as_ref().ok_or(Fault::RecoveryConflict)?,
                    r.id,
                    &sum,
                    restore.as_deref(),
                )?;
            }
        }
        self.finish_removed(r)
    }
    fn confirm_probe(&self, plan: &Plan, r: &Registration) -> Result<Verification, Fault> {
        trace("check_binding");
        let live = self.store.integration_binding(self.binding, self.origin)?;
        let t = plan.layout.open(&live, false)?.ok_or(Fault::EntryChanged)?;
        let launch = plan.view.launch.as_ref().ok_or(Fault::CliUnavailable)?;
        trace("check_cli");
        let stable = cli_install::locate(self.paths).map_err(|_| Fault::CliUnavailable)?;
        if stable.path != Path::new(&launch.command) || signature(launch) != r.fingerprint {
            return Err(Fault::EntryChanged);
        }
        let verify_entry = || {
            if plan
                .layout
                .document(t.read()?.as_deref(), Edit::Inspect)?
                .entry
                .as_deref()
                != Some(&r.fingerprint)
            {
                Err(Fault::EntryChanged)
            } else {
                Ok(())
            }
        };
        verify_entry()?;
        let verdict = self
            .adapter
            .verify(self.client, &live.canonical_path, launch, &live)?;
        self.store.integration_binding(self.binding, self.origin)?;
        verify_entry()?;
        if let Layout::Local { client, .. } = &plan.layout {
            client
                .as_ref()
                .ok_or(Fault::ClientUnavailable)?
                .verify_local(&live.canonical_path, launch)?;
            self.store.integration_binding(self.binding, self.origin)?;
            verify_entry()?;
            return Ok(Verification::Client);
        }
        Ok(verdict)
    }
    fn checked(&self, plan: Plan, mut r: Registration, fresh: bool) -> Result<Outcome, Fault> {
        trace("checking");
        let verdict = match self.confirm_probe(&plan, &r) {
            Ok(v) => v,
            Err(e) => {
                if fresh {
                    self.recover(&plan.layout, &plan.root, &mut r)?;
                } else {
                    r.status = "STALE".into();
                    r.verification = None;
                    self.save(&r)?;
                }
                return Err(e);
            }
        };
        r.source = None;
        r.pending = None;
        trace("verified");
        r.status = "INSTALLED".into();
        r.verification = Some(
            match verdict {
                Verification::Client => "CLIENT",
                Verification::Gateway => "GATEWAY",
            }
            .into(),
        );
        self.save(&r)?;
        trace("registered");
        Ok(Outcome {
            preview: plan.view,
            state: if verdict == Verification::Client {
                "CONNECTED"
            } else {
                "CONFIGURED"
            },
            gateway_verified: true,
            client_verified: verdict == Verification::Client,
        })
    }
    pub fn execute(
        &self,
        action: Action,
        confirm: impl FnOnce(&Preview) -> bool,
    ) -> Result<Outcome, Fault> {
        let guard = self.paths.lock_integrations()?;
        let plan = self.plan(action)?;
        if matches!(
            action,
            Action::Detect | Action::Preview | Action::PreviewUninstall
        ) {
            let state = match plan.view.change {
                ConfigChange::Recover => "RECOVERY_REQUIRED",
                ConfigChange::Conflict => "CONFLICT",
                ConfigChange::Edited => "STALE",
                _ => match plan.row.as_ref() {
                    Some(r) if r.status == "STALE" => "STALE",
                    Some(_) => "CONFIGURED",
                    None => "NOT_REGISTERED",
                },
            };
            let checked = state == "CONFIGURED" && plan.view.detected;
            let level = plan.row.as_ref().and_then(|r| r.verification.as_deref());
            return Ok(Outcome {
                preview: plan.view,
                state: if checked && level == Some("CLIENT") {
                    "CONNECTED"
                } else {
                    state
                },
                gateway_verified: checked && level.is_some(),
                client_verified: checked && level == Some("CLIENT"),
            });
        }
        match plan.view.change {
            ConfigChange::Conflict => return Err(Fault::NameConflict),
            ConfigChange::Edited => return Err(Fault::EntryChanged),
            _ => (),
        }
        if action == Action::Verify {
            if plan.view.change == ConfigChange::Recover {
                return Err(Fault::RecoveryConflict);
            }
            let r = plan.row.clone().ok_or(Fault::NotRegistered)?;
            if !plan.view.detected {
                return Err(Fault::ClientUnavailable);
            }
            return self.checked(plan, r, false);
        }
        if action == Action::Install
            && plan.view.change != ConfigChange::Recover
            && !plan.view.detected
        {
            return Err(Fault::ClientUnavailable);
        }
        if action == Action::Uninstall && plan.row.is_none() {
            return Ok(Outcome::new(plan.view, "REMOVED"));
        }
        drop(guard);
        if !confirm(&plan.view) {
            return Err(Fault::Cancelled);
        }
        let _guard = self.paths.lock_integrations()?;
        let current = self.plan(action)?;
        if current.row != plan.row
            || current.layout != plan.layout
            || current.before != plan.before
            || current.view != plan.view
            || current.root.root_identity != plan.root.root_identity
        {
            return Err(Fault::ConcurrentEdit);
        }
        if let Some(mut r) = current.row.clone() {
            if r.pending.is_some() {
                self.recover(&current.layout, &current.root, &mut r)?;
                return Ok(Outcome::new(current.view, "RECOVERED"));
            }
            if action == Action::Install {
                return self.checked(current, r, false);
            }
            let bytes = self.removal(&current.layout, &r, current.before.as_deref())?;
            self.paths.save_change_backup(
                r.id,
                &current.layout.change_backup(
                    current.before.as_deref().ok_or(Fault::EntryChanged)?,
                    &r.fingerprint,
                )?,
            )?;
            r.pending = Some("REMOVE".into());
            r.source = Some(fingerprint(current.before.as_deref()));
            r.removal = Some(fingerprint(bytes.as_deref()));
            self.save(&r)?;
            let t = current
                .layout
                .open(&current.root, false)?
                .ok_or(Fault::ConcurrentEdit)?;
            self.publish(
                &t,
                r.id,
                &fingerprint(current.before.as_deref()),
                bytes.as_deref(),
            )?;
            self.finish_removed(&mut r)?;
            return Ok(Outcome::new(current.view, "REMOVED"));
        }
        let launch = current.view.launch.as_ref().ok_or(Fault::CliUnavailable)?;
        let after = current
            .layout
            .document(current.before.as_deref(), Edit::Add(launch))?
            .bytes;
        if after.len() > file::MAX {
            return Err(Fault::InvalidConfig);
        }
        let id = IntegrationId::generate();
        if let Some(bytes) = &current.before {
            self.paths
                .save_config_backup(id, &current.layout.original_backup(bytes, &after)?)?;
        }
        let mut r = Registration {
            id,
            binding: self.binding,
            client: self.client,
            location: current.layout.location(&current.root),
            project_key: current.view.project_key.clone(),
            scope: current.view.scope.into(),
            fingerprint: signature(launch),
            status: "STALE".into(),
            original: Some(fingerprint(current.before.as_deref())),
            published: Some(fingerprint(Some(&after))),
            pending: Some("INSTALL".into()),
            source: Some(fingerprint(current.before.as_deref())),
            removal: None,
            verification: None,
        };
        self.save(&r)?;
        let t = current
            .layout
            .open(&current.root, true)?
            .ok_or(Fault::UnsafePath)?;
        match self.publish(
            &t,
            r.id,
            &fingerprint(current.before.as_deref()),
            Some(&after),
        ) {
            Err(Fault::ConcurrentEdit) => {
                // The competing editor won publication. Never adopt an independently
                // created entry, even if it happens to match our launch descriptor.
                self.finish_removed(&mut r)?;
                return Err(Fault::ConcurrentEdit);
            }
            result => result?,
        }
        self.checked(current, r, true)
    }
}

#[cfg(all(test, any(target_os = "macos", windows)))]
mod tests;
