//! Pure coordinate maths: captured-frame pixels -> overlay-window logical coordinates.
//!
//! Three spaces are involved:
//! * frame pixels: device pixels of the captured image, origin at the frame's top-left;
//! * logical desktop: points on the virtual desktop (may be negative on monitors left of / above the primary);
//! * overlay-local: CSS px inside the overlay window, whose top-left sits at the overlay rect's origin.

use crate::model::{BoardGeometry, LogicalRect};
use fishy_vision::{BoardRect, CaptureTarget, Frame, Orientation};

/// Where a frame came from: its logical top-left and device-pixels-per-point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSpace {
    pub origin_x: i32,
    pub origin_y: i32,
    pub scale: f32,
}

impl From<&Frame> for FrameSpace {
    fn from(f: &Frame) -> Self {
        FrameSpace {
            origin_x: f.origin_x,
            origin_y: f.origin_y,
            scale: f.scale,
        }
    }
}

/// The overlay rectangle that exactly covers a capture target.
pub fn overlay_rect_for(target: &CaptureTarget) -> LogicalRect {
    LogicalRect {
        x: target.x as f64,
        y: target.y as f64,
        width: target.width as f64,
        height: target.height as f64,
    }
}

/// Map a board rect in frame pixels into overlay-local logical coordinates.
pub fn board_to_overlay(
    rect: &BoardRect,
    frame: FrameSpace,
    overlay: &LogicalRect,
    orientation: Orientation,
) -> BoardGeometry {
    let scale = if frame.scale.is_finite() && frame.scale > 0.0 {
        frame.scale as f64
    } else {
        1.0
    };
    BoardGeometry {
        x: frame.origin_x as f64 + rect.x as f64 / scale - overlay.x,
        y: frame.origin_y as f64 + rect.y as f64 / scale - overlay.y,
        size: rect.size as f64 / scale,
        flipped: orientation == Orientation::Black,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, size: f32) -> BoardRect {
        BoardRect { x, y, size }
    }

    #[test]
    fn one_x_primary_monitor() {
        let overlay = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let g = board_to_overlay(
            &rect(400.0, 100.0, 800.0),
            FrameSpace {
                origin_x: 0,
                origin_y: 0,
                scale: 1.0,
            },
            &overlay,
            Orientation::White,
        );
        assert_eq!(
            g,
            BoardGeometry {
                x: 400.0,
                y: 100.0,
                size: 800.0,
                flipped: false
            }
        );
    }

    #[test]
    fn retina_two_x_halves_pixels() {
        let overlay = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        };
        let g = board_to_overlay(
            &rect(800.0, 200.0, 1200.0),
            FrameSpace {
                origin_x: 0,
                origin_y: 0,
                scale: 2.0,
            },
            &overlay,
            Orientation::Black,
        );
        assert_eq!(
            g,
            BoardGeometry {
                x: 400.0,
                y: 100.0,
                size: 600.0,
                flipped: true
            }
        );
    }

    #[test]
    fn monitor_at_negative_origin() {
        // Secondary monitor to the left of and above the primary.
        let overlay = LogicalRect {
            x: -1920.0,
            y: -300.0,
            width: 1920.0,
            height: 1080.0,
        };
        let g = board_to_overlay(
            &rect(100.0, 50.0, 640.0),
            FrameSpace {
                origin_x: -1920,
                origin_y: -300,
                scale: 1.0,
            },
            &overlay,
            Orientation::White,
        );
        assert_eq!((g.x, g.y, g.size), (100.0, 50.0, 640.0));
    }

    #[test]
    fn negative_origin_retina() {
        let overlay = LogicalRect {
            x: -1440.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        };
        let g = board_to_overlay(
            &rect(1000.0, 400.0, 800.0),
            FrameSpace {
                origin_x: -1440,
                origin_y: 0,
                scale: 2.0,
            },
            &overlay,
            Orientation::White,
        );
        assert_eq!((g.x, g.y, g.size), (500.0, 200.0, 400.0));
    }

    #[test]
    fn frame_origin_differs_from_overlay_origin() {
        // Frame is a window at (200, -50); overlay covers a larger rect starting at (100, -100).
        let overlay = LogicalRect {
            x: 100.0,
            y: -100.0,
            width: 900.0,
            height: 700.0,
        };
        let g = board_to_overlay(
            &rect(40.0, 20.0, 400.0),
            FrameSpace {
                origin_x: 200,
                origin_y: -50,
                scale: 2.0,
            },
            &overlay,
            Orientation::White,
        );
        assert_eq!((g.x, g.y, g.size), (120.0, 60.0, 200.0));
    }

    #[test]
    fn bad_scale_is_treated_as_one() {
        let overlay = LogicalRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let g = board_to_overlay(
            &rect(8.0, 8.0, 80.0),
            FrameSpace {
                origin_x: 0,
                origin_y: 0,
                scale: 0.0,
            },
            &overlay,
            Orientation::White,
        );
        assert_eq!(g.size, 80.0);
    }

    #[test]
    fn overlay_rect_follows_target() {
        let t = CaptureTarget {
            id: "window:7".into(),
            kind: "window".into(),
            title: "t".into(),
            app: "a".into(),
            x: -10,
            y: 20,
            width: 800,
            height: 600,
            scale: 2.0,
        };
        assert_eq!(
            overlay_rect_for(&t),
            LogicalRect {
                x: -10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0
            }
        );
    }
}
