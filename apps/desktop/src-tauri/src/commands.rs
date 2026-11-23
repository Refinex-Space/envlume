use envlume_application::projects::{ProjectList, StorageError};
use envlume_storage::{Store, StorePaths};
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
    builder.invoke_handler(tauri::generate_handler![list_projects])
}
