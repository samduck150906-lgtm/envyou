//! envyou desktop library: GUI runtime (Tauri) plus the `--mcp` runtime.

pub mod commands;
pub mod mcp_runtime;
pub mod util;

use std::sync::Mutex;

use envyou_core::core::storage::Store;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

use commands::AppState;

/// Launch the retro floating desktop GUI (default mode, spec §2.2).
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            // `envyou://activate?email=…&code=…` links (from the purchase email /
            // success page): surface the window and forward the URL to the
            // frontend, which pre-fills the Activate Pro form.
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // Best-effort runtime registration (prod registers via the
                // installer; this covers dev + Linux).
                let _ = app.deep_link().register_all();
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                        let _ = w.emit("deep-link-activate", urls);
                    }
                });
            }
            // Decide the initial lock state. A password-protected (v2) vault
            // starts locked (None) and waits for the frontend to unlock it; a
            // device-bound or not-yet-created vault opens immediately.
            let path = envyou_core::core::storage::default_data_dir()?
                .join(envyou_core::core::storage::STATE_FILE);
            // Fail closed: if the file can't be read as a well-formed envelope
            // (missing, or corrupted), don't assume it's an unprotected
            // device-bound vault — leave the store unset. `vault_status` hits
            // the same check and will surface the same error to the frontend,
            // which now shows a "couldn't open" screen instead of silently
            // loading (see `is_password_protected`'s doc comment).
            let starts_locked = match std::fs::read_to_string(&path) {
                Ok(raw) => envyou_core::core::crypto::is_password_protected(&raw).unwrap_or(true),
                Err(_) => false,
            };
            let store = if starts_locked {
                None
            } else {
                Some(Store::open_default()?)
            };
            app.manage(AppState {
                store: Mutex::new(store),
            });

            // System tray (spec §2.2: app lives in the tray).
            let show = MenuItem::with_id(app, "show", "Show envyou", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("envyou")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Apply the persisted "always on top" preference (spec §3.2).
            // Skipped when the vault is locked — the setting is applied after
            // unlock instead.
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(guard) = app.state::<AppState>().store.lock() {
                    if let Some(store) = guard.as_ref() {
                        if let Ok(s) = store.load() {
                            let _ = window.set_always_on_top(s.settings.always_on_top);
                        }
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::create_project,
            commands::delete_project,
            commands::rename_project,
            commands::upsert_variable,
            commands::delete_variable,
            commands::save_settings,
            commands::activate_pro,
            commands::activate_certificate,
            commands::link_claude_desktop,
            commands::vault_status,
            commands::unlock_vault,
            commands::set_master_password,
            set_always_on_top,
        ])
        .run(tauri::generate_context!())
        .expect("error while running envyou");
}

/// Toggle the floating window's always-on-top pin (spec §3.2).
#[tauri::command]
fn set_always_on_top(window: tauri::Window, enabled: bool) -> Result<(), String> {
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}
