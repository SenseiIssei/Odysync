//! System tray icon with menu for background operation.
//!
//! The tray lets the user open the window, scan for updates, or quit.
//! When the window is closed, the app continues running in the tray
//! so scheduled background scans can still notify the user.

use crate::state::AppState;

use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

/// Bring the window back, also when it is minimised.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Odysync", true, None::<&str>)?;
    let scan = MenuItem::with_id(app, "scan", "Scan for Updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &scan, &quit])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Odysync")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                show_window(app);
            }
            "scan" => {
                show_window(app);
                let _ = app.emit("tray-scan", ());
            }
            "quit" => {
                // While something is being installed the window asks first.
                if app.state::<AppState>().is_applying() {
                    show_window(app);
                    let _ = app.emit("tray-quit", ());
                } else {
                    app.exit(0);
                }
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}
