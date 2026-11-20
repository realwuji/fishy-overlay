//! Application state shared by commands, tray, hotkeys and the session.

use crate::overlay::Overlay;
use crate::session::SessionController;
use crate::settings::SettingsStore;
use std::sync::{Arc, Mutex};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager, Wry};

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<SettingsStore>,
    pub overlay: Arc<Overlay>,
    pub session: Arc<SessionController>,
    /// Tray "Start/Stop" entry, relabelled as the session starts and stops.
    pub tray_start_stop: Arc<Mutex<Option<MenuItem<Wry>>>>,
}

/// Fetch the managed state from any `AppHandle`.
pub fn get(app: &AppHandle) -> AppState {
    app.state::<AppState>().inner().clone()
}
