#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .expect("configured main window");
            window.set_title(&format!("Envlume {}", envlume_application::VERSION))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Envlume desktop");
}
