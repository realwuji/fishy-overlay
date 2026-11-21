//! Tauri commands: exactly the `Commands` interface in `src/shared/types.ts`.
//!
//! Commands that wait on the session thread are `async` and hop to a blocking
//! worker so they never occupy the main/UI thread.

use crate::actions;
use crate::model::{BoardState, SessionStatus};
use crate::settings::Settings;
use crate::state::AppState;
use crate::util::guard;
use fishy_vision::{
    CaptureTarget, OrientationMode, has_permission, list_targets, request_permission,
};
use serde_json::Value;
use tauri::{AppHandle, State};

/// Run blocking work off the async runtime's cooperative threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Internal error: {e}"))
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

/// Accepts a (possibly partial) settings object: missing fields take defaults, then values are clamped.
#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Value) -> Result<Settings, String> {
    let parsed = Settings::from_value(settings).map_err(|e| format!("Invalid settings: {e}"))?;
    actions::commit_settings(&app, parsed)
}

#[tauri::command]
pub fn reset_settings(app: AppHandle, state: State<'_, AppState>) -> Result<Settings, String> {
    actions::commit_settings(&app, state.settings.get().factory_reset())
}

/// Capture targets the user can pick, minus Fishy's own windows.
pub fn selectable_targets(all: Vec<CaptureTarget>) -> Vec<CaptureTarget> {
    all.into_iter()
        .filter(|t| !(t.kind == "window" && t.app.eq_ignore_ascii_case("fishy")))
        .collect()
}

#[tauri::command]
pub async fn list_capture_targets() -> Result<Vec<CaptureTarget>, String> {
    blocking(|| match guard(list_targets) {
        Ok(Ok(targets)) => Ok(selectable_targets(targets)),
        Ok(Err(e)) => Err(format!("Could not list screens and windows: {e}")),
        Err(e) => Err(format!("Could not list screens and windows: {e}")),
    })
    .await?
}

#[tauri::command]
pub async fn start_session(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    let session = state.session.clone();
    blocking(move || session.start()).await?
}

#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    let session = state.session.clone();
    blocking(move || session.stop()).await
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    let session = state.session.clone();
    blocking(move || session.status()).await
}

#[tauri::command]
pub async fn calibrate_start_position(state: State<'_, AppState>) -> Result<BoardState, String> {
    let session = state.session.clone();
    blocking(move || session.calibrate_start_position()).await?
}

#[tauri::command]
pub async fn recalibrate_board(state: State<'_, AppState>) -> Result<BoardState, String> {
    let session = state.session.clone();
    blocking(move || session.recalibrate_board()).await?
}

#[tauri::command]
pub async fn set_known_position(
    state: State<'_, AppState>,
    fen: String,
) -> Result<BoardState, String> {
    let session = state.session.clone();
    blocking(move || session.set_known_position(fen)).await?
}

/// Which colour is at the bottom of the screen: `"auto"` (detected, the default), `"white"` or
/// `"black"` (final). Stored in `capture.orientation`; a running session picks it up at once.
#[tauri::command]
pub fn set_board_orientation(app: AppHandle, orientation: String) -> Result<Settings, String> {
    let mode = parse_orientation(&orientation)?;
    actions::mutate_settings(&app, |s| s.capture.orientation = mode)
}

fn parse_orientation(s: &str) -> Result<OrientationMode, String> {
    OrientationMode::parse(s)
        .ok_or_else(|| format!("Unknown orientation '{s}': use \"auto\", \"white\" or \"black\"."))
}

#[tauri::command]
pub fn set_overlay_visible(state: State<'_, AppState>, visible: bool) {
    state.overlay.set_wanted(visible);
}

/// Reveal or hide the overlay content in peek mode (used by the control window's preview button).
#[tauri::command]
pub fn set_peek(app: tauri::AppHandle, revealed: bool) {
    crate::actions::set_peek(&app, revealed);
}

#[tauri::command]
pub fn get_peek(state: State<'_, AppState>) -> bool {
    state.overlay.peek_revealed()
}

#[tauri::command]
pub fn set_click_through(state: State<'_, AppState>, enabled: bool) {
    state.overlay.set_click_through(enabled);
}

#[tauri::command]
pub async fn request_screen_permission(state: State<'_, AppState>) -> Result<bool, String> {
    let session = state.session.clone();
    blocking(move || {
        let _ = guard(request_permission);
        let granted = guard(has_permission).unwrap_or(false);
        // Refresh the cached status so the UI sees the new permission state.
        let _ = session.status();
        granted
    })
    .await
}

/// Where the OS lets the user grant Screen Recording; a no-op on platforms without such a gate.
#[tauri::command]
pub fn open_screen_permission_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open System Settings: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(kind: &str, app: &str) -> CaptureTarget {
        CaptureTarget {
            id: format!("{kind}:1"),
            kind: kind.into(),
            title: String::new(),
            app: app.into(),
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            scale: 1.0,
        }
    }

    #[test]
    fn orientation_argument_is_validated() {
        assert_eq!(parse_orientation("auto"), Ok(OrientationMode::Auto));
        assert_eq!(parse_orientation("white"), Ok(OrientationMode::White));
        assert_eq!(parse_orientation("black"), Ok(OrientationMode::Black));
        assert!(parse_orientation("Black").is_err());
        assert!(parse_orientation("").is_err());
    }

    #[test]
    fn own_windows_are_not_selectable() {
        let out = selectable_targets(vec![
            t("window", "Fishy"),
            t("window", "Google Chrome"),
            t("monitor", ""),
        ]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|x| !x.app.eq_ignore_ascii_case("fishy")));
    }
}
