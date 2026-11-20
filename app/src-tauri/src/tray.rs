//! System tray icon and menu.

use crate::actions;
use crate::model::{EVT_STATUS, SessionStatus};
use crate::state;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Listener};

const ID_SHOW: &str = "show";
const ID_START_STOP: &str = "start_stop";
const ID_OVERLAY: &str = "toggle_overlay";
const ID_HUD: &str = "toggle_hud";
const ID_QUIT: &str = "quit";

pub fn start_stop_label(running: bool) -> &'static str {
    if running {
        "Stop session"
    } else {
        "Start session"
    }
}

pub fn build(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, ID_SHOW, "Show Fishy", true, None::<&str>)?;
    let start_stop = MenuItem::with_id(
        app,
        ID_START_STOP,
        start_stop_label(false),
        true,
        None::<&str>,
    )?;
    let overlay = MenuItem::with_id(app, ID_OVERLAY, "Toggle overlay", true, None::<&str>)?;
    let hud = MenuItem::with_id(app, ID_HUD, "Toggle HUD", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit Fishy", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&show, &sep1, &start_stop, &overlay, &hud, &sep2, &quit],
    )?;

    *state::get(app.handle())
        .tray_start_stop
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(start_stop);

    let mut builder = TrayIconBuilder::with_id("fishy-tray")
        .tooltip("Fishy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            ID_SHOW => actions::show_main_window(app),
            ID_START_STOP => actions::toggle_session(app),
            ID_OVERLAY => actions::toggle_overlay(app),
            ID_HUD => actions::toggle_hud(app),
            ID_QUIT => actions::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                actions::show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    // Keep the Start/Stop label in sync with the session.
    let handle = app.handle().clone();
    app.listen(EVT_STATUS, move |event| {
        let Ok(status) = serde_json::from_str::<SessionStatus>(event.payload()) else {
            return;
        };
        let st = state::get(&handle);
        let guard = st.tray_start_stop.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_ref() {
            let _ = item.set_text(start_stop_label(status.running));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(start_stop_label(true), "Stop session");
        assert_eq!(start_stop_label(false), "Start session");
    }
}
