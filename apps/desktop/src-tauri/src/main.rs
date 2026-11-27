#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod commands;
use envlume_storage::StorePaths;
use tauri::Manager;

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|s| s == "--integration") {
        return envlume_clients::command(&args[1..], StorePaths::local);
    }
    if args.first().is_some_and(|a| a == "--cli") {
        return cli::maintenance(&args[1..]);
    }
    commands::with_commands(tauri::Builder::default())
        .setup(|app| {
            let mut args = std::env::args_os().skip(1);
            let binding = if args.next().as_deref() == Some(std::ffi::OsStr::new("--binding")) {
                let id = args
                    .next()
                    .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                    .filter(|_| args.next().is_none())
                    .ok_or_else(|| std::io::Error::other("Expected --binding <uuid>"))?;
                Some((envlume_domain::BindingId(id), std::env::current_dir()?))
            } else {
                None
            };
            app.manage(commands::DesktopBinding(binding));
            let paths = StorePaths::local().map_err(|e| std::io::Error::other(e.message()))?;
            #[cfg(feature = "bundled-cli")]
            {
                let installed = cli::install(&paths, true);
                if let Err(error) = &installed {
                    eprintln!("Envlume CLI installation: {error:?}. {}", error.message());
                }
                app.manage(installed);
            }
            app.manage(paths);
            let window = app
                .get_webview_window("main")
                .expect("configured main window");
            window.set_title(&format!("Envlume {}", envlume_application::VERSION))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Envlume desktop");
    std::process::ExitCode::SUCCESS
}
