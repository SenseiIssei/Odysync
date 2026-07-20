mod commands;
mod state;
mod tray;

use tracing_subscriber::EnvFilter;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new())
        .setup(|app| {
            tray::setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide the window instead of closing so the tray icon
                // keeps the app alive for background notifications.
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::apply,
            commands::list_backends,
            commands::get_config,
            commands::save_config,
            commands::hold,
            commands::unhold,
            commands::run_maintenance,
            commands::list_maintenance,
            commands::create_schedule,
            commands::remove_schedule,
            commands::check_schedule,
            commands::create_diagnostics,
            commands::get_system_info,
            commands::background_scan,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Odysync GUI");
}
