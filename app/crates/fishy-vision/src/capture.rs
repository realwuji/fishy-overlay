//! Cross-platform screen capture on top of the `xcap` crate.
//!
//! Frames live in memory only: nothing in this module (or crate) ever writes a
//! captured frame to disk or sends it over a network.
//!
//! Target ids are stable for as long as the underlying OS object exists:
//! `"monitor:<n>"` and `"window:<n>"`, where `<n>` is the OS display / window id.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("screen recording permission missing")]
    PermissionDenied,
    #[error("target not found: {0}")]
    TargetNotFound(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTarget {
    /// Stable id: "monitor:<n>" or "window:<n>".
    pub id: String,
    /// "monitor" | "window"
    pub kind: String,
    pub title: String,
    pub app: String,
    /// Logical (point) coordinates of the target on the virtual desktop.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Device pixels per logical pixel (2.0 on Retina).
    pub scale: f32,
}

/// One captured frame, RGBA8, row-major, no padding.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Logical desktop position of the frame's top-left corner.
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale: f32,
}

/// Windows smaller than this (logical units, either side) are never offered.
const MIN_WINDOW_SIDE: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Monitor,
    Window,
}

/// Parse `"monitor:<n>"` / `"window:<n>"`.
fn parse_id(id: &str) -> Option<(TargetKind, u32)> {
    let (kind, n) = id.split_once(':')?;
    let n: u32 = n.trim().parse().ok()?;
    match kind {
        "monitor" => Some((TargetKind::Monitor, n)),
        "window" => Some((TargetKind::Window, n)),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod permission {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub fn has() -> bool {
        // SAFETY: plain C call without arguments, available since macOS 10.15.
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request() {
        // SAFETY: plain C call without arguments; shows the system prompt once.
        unsafe {
            CGRequestScreenCaptureAccess();
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod permission {
    pub fn has() -> bool {
        true
    }
    pub fn request() {}
}

/// Is Screen Recording permission granted? (always true on Windows.)
pub fn has_permission() -> bool {
    permission::has()
}

/// Trigger the OS permission prompt where one exists.
pub fn request_permission() {
    permission::request()
}

fn map_err(e: xcap::XCapError) -> VisionError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("permission") || lower.contains("denied") || lower.contains("not authorized")
    {
        VisionError::PermissionDenied
    } else {
        VisionError::Capture(msg)
    }
}

/// Convert device pixels to logical units where xcap reports device pixels.
fn to_logical(v: u32, scale: f32) -> u32 {
    if cfg!(target_os = "macos") || scale <= 0.0 {
        // xcap reports points on macOS.
        v
    } else {
        (v as f32 / scale).round() as u32
    }
}

/// Like [`to_logical`] for a signed desktop coordinate. On Windows xcap reports the position of
/// a monitor or window in physical virtual-desktop pixels, so it must be scaled the same way as
/// its size to be comparable with the logical coordinates the overlay is placed in. (With mixed
/// DPI the scale of the target's own monitor is used, which is exact whenever the overlay sits
/// on the same monitor as the captured region.)
fn to_logical_pos(v: i32, scale: f32) -> i32 {
    if cfg!(target_os = "macos") || scale <= 0.0 {
        v
    } else {
        (v as f32 / scale).round() as i32
    }
}

fn is_own_window(w: &xcap::Window, title: &str, app: &str) -> bool {
    if w.pid().map(|p| p == std::process::id()).unwrap_or(false) {
        return true;
    }
    let app = app.to_lowercase();
    let title = title.to_lowercase();
    app == "fishy"
        || app.starts_with("fishy ")
        || title == "fishy"
        || title.starts_with("fishy overlay")
}

fn monitor_target(m: &xcap::Monitor) -> Result<CaptureTarget, VisionError> {
    let id = m.id().map_err(map_err)?;
    let scale = m.scale_factor().unwrap_or(1.0).max(1.0);
    Ok(CaptureTarget {
        id: format!("monitor:{id}"),
        kind: "monitor".into(),
        title: m.name().unwrap_or_default(),
        app: String::new(),
        x: to_logical_pos(m.x().unwrap_or(0), scale),
        y: to_logical_pos(m.y().unwrap_or(0), scale),
        width: to_logical(m.width().map_err(map_err)?, scale),
        height: to_logical(m.height().map_err(map_err)?, scale),
        scale,
    })
}

fn window_target(w: &xcap::Window) -> Option<CaptureTarget> {
    if w.is_minimized().unwrap_or(false) {
        return None;
    }
    let title = w.title().unwrap_or_default();
    let app = w.app_name().unwrap_or_default();
    if is_own_window(w, &title, &app) {
        return None;
    }
    let scale = w
        .current_monitor()
        .and_then(|m| m.scale_factor())
        .unwrap_or(1.0)
        .max(1.0);
    let width = to_logical(w.width().ok()?, scale);
    let height = to_logical(w.height().ok()?, scale);
    if width < MIN_WINDOW_SIDE || height < MIN_WINDOW_SIDE {
        return None;
    }
    Some(CaptureTarget {
        id: format!("window:{}", w.id().ok()?),
        kind: "window".into(),
        title,
        app,
        x: to_logical_pos(w.x().unwrap_or(0), scale),
        y: to_logical_pos(w.y().unwrap_or(0), scale),
        width,
        height,
        scale,
    })
}

/// Monitors first, then visible normal windows (front-most first).
pub fn list_targets() -> Result<Vec<CaptureTarget>, VisionError> {
    let mut out = Vec::new();
    for m in xcap::Monitor::all().map_err(map_err)? {
        if let Ok(t) = monitor_target(&m) {
            out.push(t);
        }
    }
    // A failure to enumerate windows must not hide the monitors.
    if let Ok(windows) = xcap::Window::all() {
        out.extend(windows.iter().filter_map(window_target));
    }
    Ok(out)
}

fn into_frame(
    img: image::RgbaImage,
    origin: (i32, i32),
    logical_w: u32,
    fallback_scale: f32,
) -> Frame {
    let (width, height) = img.dimensions();
    let scale = if logical_w > 0 {
        (width as f32 / logical_w as f32).max(1.0)
    } else {
        fallback_scale
    };
    Frame {
        width,
        height,
        rgba: img.into_raw(),
        origin_x: origin.0,
        origin_y: origin.1,
        scale,
    }
}

/// Capture one target. The returned frame is RGBA with the logical desktop
/// origin and the effective device-pixel scale of the capture.
pub fn grab(target_id: &str) -> Result<Frame, VisionError> {
    let (kind, n) =
        parse_id(target_id).ok_or_else(|| VisionError::TargetNotFound(target_id.to_string()))?;
    if !has_permission() {
        return Err(VisionError::PermissionDenied);
    }
    match kind {
        TargetKind::Monitor => {
            let m = xcap::Monitor::all()
                .map_err(map_err)?
                .into_iter()
                .find(|m| m.id().map(|i| i == n).unwrap_or(false))
                .ok_or_else(|| VisionError::TargetNotFound(target_id.to_string()))?;
            let t = monitor_target(&m)?;
            let img = m.capture_image().map_err(map_err)?;
            Ok(into_frame(img, (t.x, t.y), t.width, t.scale))
        }
        TargetKind::Window => {
            let w = xcap::Window::all()
                .map_err(map_err)?
                .into_iter()
                .find(|w| w.id().map(|i| i == n).unwrap_or(false))
                .ok_or_else(|| VisionError::TargetNotFound(target_id.to_string()))?;
            let scale = w
                .current_monitor()
                .and_then(|m| m.scale_factor())
                .unwrap_or(1.0)
                .max(1.0);
            let (x, y) = (
                to_logical_pos(w.x().unwrap_or(0), scale),
                to_logical_pos(w.y().unwrap_or(0), scale),
            );
            let logical_w = to_logical(w.width().map_err(map_err)?, scale);
            let img = w.capture_image().map_err(map_err)?;
            Ok(into_frame(img, (x, y), logical_w, scale))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids() {
        assert_eq!(parse_id("monitor:3"), Some((TargetKind::Monitor, 3)));
        assert_eq!(
            parse_id("window:123456"),
            Some((TargetKind::Window, 123456))
        );
        assert_eq!(parse_id("window:"), None);
        assert_eq!(parse_id("screen:1"), None);
        assert_eq!(parse_id("monitor"), None);
    }

    #[test]
    fn unknown_target_is_reported() {
        match grab("nonsense") {
            Err(VisionError::TargetNotFound(_)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn logical_position_follows_the_size_conversion() {
        if cfg!(target_os = "macos") {
            assert_eq!(to_logical_pos(-1920, 2.0), -1920);
        } else {
            assert_eq!(to_logical_pos(1800, 1.5), 1200);
            assert_eq!(to_logical_pos(-1920, 1.5), -1280);
            assert_eq!(to_logical_pos(600, 1.0), 600);
        }
    }

    #[test]
    fn logical_conversion() {
        if cfg!(target_os = "macos") {
            assert_eq!(to_logical(1440, 2.0), 1440);
        } else {
            assert_eq!(to_logical(2880, 2.0), 1440);
        }
    }
}
