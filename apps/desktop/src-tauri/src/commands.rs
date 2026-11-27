use envlume_application::projects::{ProjectList, StorageError};
use envlume_application::{ErrorCode, Session, result::Report};
use envlume_domain::{Actor, BindingId};
use envlume_storage::{LocalRegistration, Store, StorePaths};
use std::path::PathBuf;

/// Set by the native host. IPC arguments cannot switch the binding or its origin.
#[derive(Clone)]
pub struct DesktopBinding(pub Option<(BindingId, PathBuf)>);

#[tauri::command]
async fn list_environments(
    paths: tauri::State<'_, StorePaths>,
    binding: tauri::State<'_, DesktopBinding>,
) -> Result<Report, ErrorCode> {
    let paths = paths.inner().clone();
    let selected = binding.0.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (id, origin) = selected.ok_or(ErrorCode::ProjectNotTrusted)?;
        let store = Store::open(paths).map_err(|e| match e {
            StorageError::StorageBusy => ErrorCode::Timeout,
            _ => ErrorCode::InternalError,
        })?;
        Ok(
            Session::from_registration(LocalRegistration { store, origin }, id, Actor::Desktop)?
                .environment_list(serde_json::json!({})),
        )
    })
    .await
    .map_err(|_| ErrorCode::InternalError)?;
    Ok(result.unwrap_or_else(|code| {
        let mut report = Report::new("environment_list");
        report.fail(code, code.summary());
        report
    }))
}

#[tauri::command]
async fn list_projects(paths: tauri::State<'_, StorePaths>) -> Result<ProjectList, StorageError> {
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        envlume_application::projects::list_projects(&Store::open(paths)?)
    })
    .await
    .map_err(|_| StorageError::StorageUnavailable)?
}

pub fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![list_projects, list_environments])
}
