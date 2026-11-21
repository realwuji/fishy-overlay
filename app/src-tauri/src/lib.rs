//! Fishy shell: Tauri app wiring and orchestration.
//!
//! * `settings`  – typed settings with lossless JSON round trip and atomic persistence
//! * `session`   – worker thread: capture -> board -> FEN -> Stockfish -> events
//! * `overlay`   – transparent click-through overlay window management
//! * `geometry`  – frame-pixel to overlay-coordinate maths
//! * `hotkeys`, `tray`, `actions` – user-level controls
//! * `commands`  – the IPC surface declared in `src/shared/types.ts`
//!
//! Fair-play and privacy rules are structural here: frames stay in memory,
//! nothing is sent over a network, no input is ever synthesised, and the
//! overlay is an ordinary, capturable window.

mod actions;
mod calibration;
mod commands;
#[cfg(test)]
mod contract_tests;
mod engine_path;
mod geometry;
mod hotkeys;
mod model;
mod overlay;
pub mod progress;
mod session;
mod settings;
mod state;
mod templates;
mod tray;
mod util;

use model::MAIN_WINDOW;
use state::AppState;
use std::sync::{Arc, Mutex};
use tauri::{Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let (store, recovered) = settings::SettingsStore::open(&config_dir);
            let store = Arc::new(store);

            let overlay = Arc::new(overlay::Overlay::new(handle.clone()));
            overlay.setup();
            overlay.set_hidden_from_capture(store.get().screen_share.hide_overlay);
            let session = Arc::new(session::SessionController::new(
                handle.clone(),
                Arc::clone(&store),
                Arc::clone(&overlay),
                data_dir,
            ));
            app.manage(AppState {
                settings: Arc::clone(&store),
                overlay,
                session: Arc::clone(&session),
                tray_start_stop: Arc::new(Mutex::new(None)),
            });

            tray::build(app)?;
            let warnings = hotkeys::apply(&handle, &store.get().hotkeys, store.get().peek.mode != settings::PeekMode::Off);
            actions::report_hotkey_warnings(&handle, warnings);
            if let Some(reason) = recovered {
                session.note(Some(format!(
                    "{reason}. Settings were reset; the old file was kept as settings.json.bak."
                )));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if state::get(app).session.is_running() {
                    // Keep analysing in the background; the tray brings the window back.
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    actions::quit(app);
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::reset_settings,
            commands::list_capture_targets,
            commands::start_session,
            commands::stop_session,
            commands::get_status,
            commands::calibrate_start_position,
            commands::recalibrate_board,
            commands::set_known_position,
            commands::calibrate_from_position,
            commands::reset_pieces,
            commands::set_board_orientation,
            commands::set_overlay_visible,
            commands::set_peek,
            commands::get_peek,
            commands::set_click_through,
            commands::request_screen_permission,
            commands::open_screen_permission_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Fishy");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            state::get(app).session.stop();
        }
    });
}
