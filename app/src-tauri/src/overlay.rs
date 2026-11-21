//! The transparent, always-on-top, click-through overlay window.
//!
//! The overlay covers exactly the capture target (a whole monitor, or a
//! followed window) so the web renderer can draw in coordinates local to the
//! target. It is an ordinary window: it is deliberately NOT excluded from
//! screen capture or sharing (no content protection / sharing-type flags).
//!
//! Every window operation is dispatched with `run_on_main_thread`, which never
//! blocks the caller. That matters because the session thread and command
//! handlers must not wait on a main thread that may itself be waiting on them.

use crate::model::{EVT_OVERLAY_GEOMETRY, LogicalRect, OVERLAY_WINDOW};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager};

pub struct Overlay {
    app: AppHandle,
    last_rect: Mutex<Option<LogicalRect>>,
    /// The user's show/hide preference (toggle hotkey, tray, UI).
    wanted: AtomicBool,
    /// A session is running (the overlay is never shown otherwise).
    session_active: AtomicBool,
    click_through: AtomicBool,
    /// Peek mode: the overlay content is currently revealed.
    peek_revealed: AtomicBool,
    /// Bumped on every peek change so stale auto-hide timers do nothing.
    peek_epoch: AtomicU64,
}

/// Does the real overlay window differ from `rect` by more than a pixel? (Unknown = no.)
fn window_drifted(w: &tauri::WebviewWindow, rect: LogicalRect) -> bool {
    let scale = w.scale_factor().unwrap_or(1.0);
    let (Ok(p), Ok(s)) = (w.outer_position(), w.outer_size()) else {
        return false;
    };
    let (p, s) = (p.to_logical::<f64>(scale), s.to_logical::<f64>(scale));
    (p.x - rect.x).abs() > 1.5
        || (p.y - rect.y).abs() > 1.5
        || (s.width - rect.width).abs() > 1.5
        || (s.height - rect.height).abs() > 1.5
}

/// Round to whole logical pixels so jitter in window rects does not move the window.
fn snap(r: LogicalRect) -> LogicalRect {
    LogicalRect {
        x: r.x.round(),
        y: r.y.round(),
        width: r.width.round().max(1.0),
        height: r.height.round().max(1.0),
    }
}

impl Overlay {
    pub fn new(app: AppHandle) -> Overlay {
        Overlay {
            app,
            last_rect: Mutex::new(None),
            wanted: AtomicBool::new(true),
            session_active: AtomicBool::new(false),
            click_through: AtomicBool::new(true),
            peek_revealed: AtomicBool::new(false),
            peek_epoch: AtomicU64::new(0),
        }
    }

    fn on_main(&self, f: impl FnOnce(&AppHandle) + Send + 'static) {
        let app = self.app.clone();
        let app2 = app.clone();
        if app.run_on_main_thread(move || f(&app2)).is_err() {
            // The event loop is gone (app is exiting); nothing to do.
        }
    }

    /// One-time window setup. Call from the Tauri `setup` hook.
    pub fn setup(&self) {
        let click = self.click_through.load(Ordering::Relaxed);
        self.on_main(move |app| {
            if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
                let _ = w.set_always_on_top(true);
                let _ = w.set_ignore_cursor_events(click);
                #[cfg(target_os = "macos")]
                macos::configure(&w, click);
            }
        });
    }

    /// Move/resize the overlay to `rect` (logical desktop coordinates) if it changed.
    pub fn apply_rect(&self, rect: LogicalRect) {
        let rect = snap(rect);
        let changed = {
            let mut last = self.last_rect.lock().unwrap_or_else(|e| e.into_inner());
            let changed = *last != Some(rect);
            *last = Some(rect);
            changed
        };
        // Even when the wanted rect is unchanged the real window can have been moved by the OS
        // (Spaces, display changes, window-server constraints); the board overlay would then be
        // drawn offset from the board. Compare with the real frame and put it back.
        self.on_main(move |app| {
            if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
                if changed || window_drifted(&w, rect) {
                    let _ = w.set_position(LogicalPosition::new(rect.x, rect.y));
                    let _ = w.set_size(LogicalSize::new(rect.width, rect.height));
                }
            }
            if changed {
                let _ = app.emit(EVT_OVERLAY_GEOMETRY, rect);
            }
        });
        if changed {
            self.refresh();
        }
    }

    pub fn last_rect(&self) -> Option<LogicalRect> {
        *self.last_rect.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Forget the covered rect (target lost) and hide until a new one arrives.
    pub fn clear_rect(&self) {
        *self.last_rect.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.refresh();
    }

    pub fn set_session_active(&self, active: bool) {
        self.session_active.store(active, Ordering::Relaxed);
        if !active {
            *self.last_rect.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        self.refresh();
    }

    pub fn set_wanted(&self, visible: bool) {
        self.wanted.store(visible, Ordering::Relaxed);
        self.refresh();
    }

    pub fn peek_revealed(&self) -> bool {
        self.peek_revealed.load(Ordering::Relaxed)
    }

    /// Store the peek state and return the new epoch (for auto-hide timers).
    pub fn set_peek_revealed(&self, revealed: bool) -> u64 {
        self.peek_revealed.store(revealed, Ordering::Relaxed);
        self.peek_epoch.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn peek_epoch(&self) -> u64 {
        self.peek_epoch.load(Ordering::Relaxed)
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }

    /// Whether the overlay should currently be on screen.
    pub fn should_show(&self) -> bool {
        self.wanted() && self.session_active.load(Ordering::Relaxed) && self.last_rect().is_some()
    }

    /// Reconcile the real window with the desired visibility.
    pub fn refresh(&self) {
        let show = self.should_show();
        let click = self.click_through.load(Ordering::Relaxed);
        let rect = self.last_rect();
        self.on_main(move |app| {
            let Some(w) = app.get_webview_window(OVERLAY_WINDOW) else {
                return;
            };
            if show {
                let _ = w.show();
                let _ = w.set_always_on_top(true);
                #[cfg(target_os = "macos")]
                macos::configure(&w, click);
                if let Some(r) = rect {
                    let _ = app.emit(EVT_OVERLAY_GEOMETRY, r);
                }
            } else {
                let _ = w.hide();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = click;
        });
    }

    /// Keep the overlay out of screen captures (screen shares, recordings) when `hidden`.
    /// macOS: NSWindow sharing type none; Windows: WDA_EXCLUDEFROMCAPTURE. Fishy's own capture
    /// then also never sees the drawn lines.
    pub fn set_hidden_from_capture(&self, hidden: bool) {
        self.on_main(move |app| {
            if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
                let _ = w.set_content_protected(hidden);
            }
        });
    }

    /// `true` = mouse events pass through to the app below (default).
    pub fn set_click_through(&self, enabled: bool) {
        self.click_through.store(enabled, Ordering::Relaxed);
        self.on_main(move |app| {
            if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
                let _ = w.set_ignore_cursor_events(enabled);
                #[cfg(target_os = "macos")]
                macos::configure(&w, enabled);
            }
        });
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! Make the overlay follow the user across Spaces and sit above full-screen apps.
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    use tauri::WebviewWindow;

    /// Pop-up-menu window level: above normal and floating windows and the menu bar.
    const OVERLAY_LEVEL: isize = 101;

    /// Must be called on the main thread.
    pub fn configure(window: &WebviewWindow, ignore_mouse: bool) {
        let Ok(ptr) = window.ns_window() else { return };
        if ptr.is_null() {
            return;
        }
        // SAFETY: Tauri guarantees `ns_window` is a live NSWindow owned by this
        // WebviewWindow, and we are on the main thread (see callers).
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary,
        );
        ns.setLevel(OVERLAY_LEVEL);
        ns.setIgnoresMouseEvents(ignore_mouse);
    }
}
