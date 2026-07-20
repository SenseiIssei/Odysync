mod commands;
mod state;
mod tray;

use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

pub fn run() {
    let dirs = directories::ProjectDirs::from("dev", "SenseiIssei", "Odysync");
    let file_writer = if let Some(d) = &dirs {
        let log_dir = d.data_dir().join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        Some(tracing_appender::rolling::never(&log_dir, "odysync.log"))
    } else {
        None
    };

    let file_layer = file_writer.map(|w| {
        tracing_subscriber::fmt::layer()
            .with_writer(w)
            .with_ansi(false)
    });

    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(console_layer)
        .with(file_layer)
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
            commands::get_update_history,
            commands::clear_update_history,
            commands::get_hardware_info,
            commands::list_installed_packages,
            commands::get_logs,
            commands::list_profiles,
            commands::create_profile,
            commands::delete_profile,
            commands::get_offline_cache_status,
            commands::clear_offline_cache,
            commands::list_offline_cache,
            commands::clear_offline_manifest,
            commands::remove_offline_entry,
            commands::download_offline_installer,
            commands::verify_offline_cache,
            commands::quit_app,
            commands::restart_as_admin,
            commands::list_startup_programs,
            commands::toggle_startup_program,
            commands::list_backups,
            commands::create_backup,
            commands::restore_backup,
            commands::delete_backup,
            commands::is_system_protection_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Odysync GUI");
}
