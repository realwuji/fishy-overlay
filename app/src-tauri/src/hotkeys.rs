//! Global shortcuts from `settings.hotkeys`, registered through
//! `tauri-plugin-global-shortcut`. Failures (unparsable, duplicate, or already
//! taken by another app) are returned as warnings and never fatal.

use crate::actions;
use crate::settings::HotkeySettings;
use std::str::FromStr;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleOverlay,
    ToggleHud,
    CycleArrows,
    Recalibrate,
    Peek,
}

impl HotkeyAction {
    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::ToggleOverlay => "Toggle overlay",
            HotkeyAction::ToggleHud => "Toggle HUD",
            HotkeyAction::CycleArrows => "Cycle arrows",
            HotkeyAction::Recalibrate => "Recalibrate board",
            HotkeyAction::Peek => "Peek",
        }
    }
}

/// Parse one accelerator string. Empty (or whitespace) means "no shortcut".
pub fn parse_hotkey(text: &str) -> Result<Option<Shortcut>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Shortcut::from_str(text)
        .map(Some)
        .map_err(|e| format!("\"{text}\" is not a valid shortcut ({e})"))
}

/// Parse all four bindings, reporting problems per action. Returns the usable
/// bindings plus human-readable warnings (parse errors, duplicates).
pub fn plan(h: &HotkeySettings, peek_enabled: bool) -> (Vec<(HotkeyAction, Shortcut)>, Vec<String>) {
    let entries = [
        (HotkeyAction::ToggleOverlay, &h.toggle_overlay),
        (HotkeyAction::ToggleHud, &h.toggle_hud),
        (HotkeyAction::CycleArrows, &h.cycle_arrows),
        (HotkeyAction::Recalibrate, &h.recalibrate),
        (HotkeyAction::Peek, &h.peek),
    ];
    let mut ok: Vec<(HotkeyAction, Shortcut)> = Vec::new();
    let mut warnings = Vec::new();
    for (action, text) in entries {
        if action == HotkeyAction::Peek && !peek_enabled {
            continue;
        }
        match parse_hotkey(text) {
            Ok(None) => {}
            Ok(Some(sc)) => {
                if let Some((other, _)) = ok.iter().find(|(_, s)| *s == sc) {
                    warnings.push(format!(
                        "{} and {} use the same shortcut ({}); only the first is active.",
                        other.label(),
                        action.label(),
                        text.trim()
                    ));
                } else {
                    ok.push((action, sc));
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", action.label())),
        }
    }
    (ok, warnings)
}

/// Replace all registered shortcuts with the ones in `h`. Returns warnings.
pub fn apply(app: &AppHandle, h: &HotkeySettings, peek_enabled: bool) -> Vec<String> {
    let gs = app.global_shortcut();
    let mut warnings = Vec::new();
    if let Err(e) = gs.unregister_all() {
        warnings.push(format!("Could not clear previous shortcuts: {e}"));
    }
    let (bindings, mut parse_warnings) = plan(h, peek_enabled);
    warnings.append(&mut parse_warnings);
    for (action, shortcut) in bindings {
        let result = gs.on_shortcut(shortcut, move |app, _sc, event| {
            match (action, event.state) {
                // Peek needs the release edge for hold mode.
                (HotkeyAction::Peek, state) => actions::peek_key(app, state == ShortcutState::Pressed),
                (_, ShortcutState::Pressed) => actions::run_hotkey(app, action),
                _ => {}
            }
        });
        if let Err(e) = result {
            warnings.push(format!(
                "{} shortcut could not be registered (it may be in use by another app): {e}",
                action.label()
            ));
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn default_hotkeys_parse() {
        let (ok, warnings) = plan(&Settings::default().hotkeys, true);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(ok.len(), 5);
        assert_eq!(plan(&Settings::default().hotkeys, false).0.len(), 4);
    }

    #[test]
    fn accepts_common_forms() {
        for s in [
            "Alt+Space",
            "alt+h",
            "CommandOrControl+Shift+K",
            "Ctrl+Alt+1",
            "F9",
            "Shift+F12",
        ] {
            assert!(parse_hotkey(s).unwrap().is_some(), "{s}");
        }
    }

    #[test]
    fn empty_disables_a_binding() {
        assert_eq!(parse_hotkey("").unwrap(), None);
        assert_eq!(parse_hotkey("   ").unwrap(), None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_hotkey("Alt+").is_err());
        assert!(parse_hotkey("Banana+Q").is_err());
        assert!(parse_hotkey("Alt+Shift").is_err());
    }

    #[test]
    fn duplicates_and_errors_become_warnings() {
        let mut h = Settings::default().hotkeys;
        h.toggle_hud = "Alt+Space".into();
        h.cycle_arrows = "nonsense++".into();
        h.recalibrate = String::new();
        h.peek = String::new();
        let (ok, warnings) = plan(&h, true);
        assert_eq!(ok.len(), 1);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|w| w.contains("same shortcut")));
    }
}
