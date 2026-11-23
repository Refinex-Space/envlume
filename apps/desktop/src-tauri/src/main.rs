#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
use envlume_storage::StorePaths;
use tauri::Manager;

fn main() {
    commands::with_commands(tauri::Builder::default())
        .setup(|app| {
            app.manage(StorePaths::local().map_err(|e| std::io::Error::other(e.message()))?);
            let window = app
                .get_webview_window("main")
                .expect("configured main window");
            window.set_title(&format!("Envlume {}", envlume_application::VERSION))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Envlume desktop");
}
