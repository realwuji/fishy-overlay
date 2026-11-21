//! User-level actions shared by hotkeys, the tray menu and commands.
//!
//! Anything that can block (starting/stopping the session, calibration) runs
//! on a short-lived helper thread so it never stalls the main/UI thread.

use crate::hotkeys::{self, HotkeyAction};
use crate::model::{EVT_PEEK, EVT_SETTINGS, MAIN_WINDOW};
use crate::settings::{PeekMode, Settings};
use crate::state;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Persist and broadcast new settings; keeps the session and hotkeys in step.
pub fn commit_settings(app: &AppHandle, settings: Settings) -> Result<Settings, String> {
    let st = state::get(app);
    let old = st.settings.get();
    let saved = st.settings.set(settings)?;
    let _ = app.emit(EVT_SETTINGS, &saved);
    st.session.settings_changed();
    if saved.hotkeys != old.hotkeys || saved.peek.mode != old.peek.mode {
        report_hotkey_warnings(
            app,
            hotkeys::apply(app, &saved.hotkeys, saved.peek.mode != PeekMode::Off),
        );
    }
    if saved.screen_share.hide_overlay != old.screen_share.hide_overlay {
        st.overlay.set_hidden_from_capture(saved.screen_share.hide_overlay);
    }
    if saved.peek.mode != old.peek.mode {
        // A new mode always starts hidden (or irrelevant when off).
        set_peek(app, false);
    }
    Ok(saved)
}

/// Read-modify-write helper for settings edited from Rust (HUD toggle, arrow cycling).
pub fn mutate_settings(app: &AppHandle, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let mut s = state::get(app).settings.get();
    f(&mut s);
    commit_settings(app, s)
}

pub fn report_hotkey_warnings(app: &AppHandle, warnings: Vec<String>) {
    let st = state::get(app);
    if warnings.is_empty() {
        return;
    }
    let text = warnings.join(" ");
    eprintln!("fishy: hotkeys: {text}");
    st.session.note(Some(text));
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn toggle_overlay(app: &AppHandle) {
    let st = state::get(app);
    st.overlay.set_wanted(!st.overlay.wanted());
}

pub fn toggle_hud(app: &AppHandle) {
    if let Err(e) = mutate_settings(app, |s| s.set_hud_visible(!s.hud_visible())) {
        state::get(app).session.note(Some(e));
    }
}

/// Cycle `arrows.count` through 1..=5 and persist it.
pub fn cycle_arrows(app: &AppHandle) {
    if let Err(e) = mutate_settings(app, |s| {
        let next = s.arrow_count() % 5 + 1;
        s.set_arrow_count(next);
    }) {
        state::get(app).session.note(Some(e));
    }
}

/// Re-run board detection (no-op when no session is running).
pub fn recalibrate(app: &AppHandle) {
    let st = state::get(app);
    if !st.session.is_running() {
        return;
    }
    thread::spawn(move || {
        if let Err(e) = st.session.recalibrate_board() {
            st.session.note(Some(e));
        }
    });
}

/// Start the session if stopped, stop it if running.
pub fn toggle_session(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let st = state::get(&app);
        if st.session.is_running() {
            st.session.stop();
        } else if let Err(e) = st.session.start() {
            // Surface the reason (fair-play notice, missing engine, ...) where the user can see it.
            st.session.note(Some(e));
            show_main_window(&app);
        }
    });
}

/// Stop everything and exit the application.
pub fn quit(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        state::get(&app).session.stop();
        app.exit(0);
    });
}

pub fn run_hotkey(app: &AppHandle, action: HotkeyAction) {
    match action {
        HotkeyAction::ToggleOverlay => toggle_overlay(app),
        HotkeyAction::ToggleHud => toggle_hud(app),
        HotkeyAction::CycleArrows => cycle_arrows(app),
        HotkeyAction::Recalibrate => recalibrate(app),
        HotkeyAction::Peek => peek_key(app, true),
    }
}

/// Longest a held peek key may stay "down" without a release event (missed edge failsafe).
const PEEK_HOLD_FAILSAFE: Duration = Duration::from_secs(45);

/// Reveal or hide the overlay content (peek mode) and tell the overlay window.
pub fn set_peek(app: &AppHandle, revealed: bool) {
    let st = state::get(app);
    let epoch = st.overlay.set_peek_revealed(revealed);
    let _ = app.emit(EVT_PEEK, serde_json::json!({ "revealed": revealed }));
    if !revealed {
        return;
    }
    // Auto-hide: toggle mode honours `autoHideSec`; hold mode has a failsafe in case a release is missed.
    let settings = st.settings.get();
    let after = match settings.peek.mode {
        PeekMode::Toggle if settings.peek.auto_hide_sec > 0 => {
            Some(Duration::from_secs(u64::from(settings.peek.auto_hide_sec)))
        }
        PeekMode::Hold => Some(PEEK_HOLD_FAILSAFE),
        _ => None,
    };
    if let Some(after) = after {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(after);
            let st = state::get(&app);
            if st.overlay.peek_epoch() == epoch && st.overlay.peek_revealed() {
                set_peek(&app, false);
            }
        });
    }
}

/// Peek key edge. `pressed == false` is the release (only meaningful in hold mode).
pub fn peek_key(app: &AppHandle, pressed: bool) {
    let st = state::get(app);
    match st.settings.get().peek.mode {
        PeekMode::Off => {}
        PeekMode::Hold => {
            if pressed != st.overlay.peek_revealed() {
                set_peek(app, pressed);
            }
        }
        PeekMode::Toggle => {
            if pressed {
                set_peek(app, !st.overlay.peek_revealed());
            }
        }
    }
}
