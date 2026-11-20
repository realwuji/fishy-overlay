//! Orientation detection: coordinate labels, structural evidence, the resolver and the pipeline.
//!
//! Real-screenshot tests use the user's own chess.com screenshots in the git-ignored
//! `tests/fixtures/local/` and skip (notice + pass) when they are absent.

use crate::labels::{Corner, classify_builtin_corner};
use crate::orient::*;
use crate::real_tests::{PLACEMENT, RECT, fixture, hl};
use crate::test_support::{erase_labels, flipped_view, frame_of_image, placement_from_fen};
use crate::*;
use image::{RgbaImage, imageops};
use std::path::PathBuf;

/// Board of the second screenshot (a full chess.com page, start position).
const START_RECT: BoardRect = BoardRect {
    x: 316.0,
    y: 177.0,
    size: 992.0,
};

fn start_fixture() -> Option<RgbaImage> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/local/chesscom-start-with-page.png");
    if !path.exists() {
        eprintln!(
            "SKIP: {} is absent (third-party screenshot, not committed)",
            path.display()
        );
        return None;
    }
    Some(image::open(path).expect("readable png").to_rgba8())
}

fn scale_to(img: &RgbaImage, rect: BoardRect, k: f32) -> (RgbaImage, BoardRect) {
    let (w, h) = (
        (img.width() as f32 * k).round() as u32,
        (img.height() as f32 * k).round() as u32,
    );
    (
        imageops::resize(img, w, h, imageops::FilterType::Lanczos3),
        BoardRect {
            x: rect.x * k,
            y: rect.y * k,
            size: rect.size * k,
        },
    )
}

fn builtin() -> Recognizer {
    Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap()
}

/// Overlay a 50% yellow highlight on a whole square (label included) or only on its background.
fn highlight_square(img: &mut RgbaImage, rect: &BoardRect, r: usize, c: usize, over_label: bool) {
    let s = rect.size / 8.0;
    let frame = frame_of_image(img);
    let cols = crate::labels::board_colours(&frame, rect);
    let bg = cols.bgs[r * 8 + c];
    let label = crate::labels::label_candidates(&cols, r, c)[0];
    let (x0, y0) = (
        (rect.x + c as f32 * s).round() as u32,
        (rect.y + r as f32 * s).round() as u32,
    );
    let near = |p: [f32; 3], q: [f32; 3], d: f32| {
        (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>().sqrt() <= d
    };
    for y in y0..y0 + s as u32 {
        for x in x0..x0 + s as u32 {
            let px = img.get_pixel_mut(x, y);
            let p = [px[0] as f32, px[1] as f32, px[2] as f32];
            // Without `over_label` only pixels that are the background (or darkened by AA) tint.
            if !over_label && !near(p, bg, 6.0) {
                continue;
            }
            let _ = label;
            let h = hl(p);
            for k in 0..3 {
                px[k] = h[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// coordinate labels
// ---------------------------------------------------------------------------

#[test]
fn builtin_label_masks_are_small_and_bundled_for_chesscom_only() {
    let t = Templates::builtin(Site::ChessCom).unwrap();
    let labels = t.data.get("labels").expect("labels section");
    assert_eq!(labels["glyphs"].as_object().unwrap().len(), 4);
    let len = serde_json::to_string(labels).unwrap().len();
    assert!(len < 6_000, "{len} bytes");
    let frame = Frame {
        width: 1200,
        height: 1200,
        rgba: vec![128; 1200 * 1200 * 4],
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    };
    let rect = BoardRect {
        x: 20.0,
        y: 20.0,
        size: 1000.0,
    };
    assert_eq!(detect_orientation(&frame, &rect, Site::Lichess), None);
    assert_eq!(detect_orientation(&frame, &rect, Site::Generic), None);
    assert_eq!(
        detect_orientation(&frame, &rect, Site::ChessCom),
        None,
        "blank board"
    );
    // Old-style templates (no labels) still load and simply cannot detect.
    let mut old = t.clone();
    old.data.as_object_mut().unwrap().remove("labels");
    assert!(Recognizer::new(&old).is_some());
    assert_eq!(detect_orientation_with(&frame, &rect, &old), None);
}

#[test]
fn white_at_the_bottom_is_read_from_both_real_screenshots() {
    let mut seen = 0;
    for (name, img, rect) in [
        ("midgame", fixture(), RECT),
        ("start with page", start_fixture(), START_RECT),
    ] {
        let Some(img) = img else { continue };
        seen += 1;
        let frame = frame_of_image(&img);
        assert_eq!(
            detect_orientation(&frame, &rect, Site::ChessCom),
            Some(Orientation::White),
            "{name}"
        );
        // Through board detection as the session does it.
        let found = detect_board(&frame, Site::ChessCom).expect("board");
        assert_eq!(
            detect_orientation(&frame, &found, Site::ChessCom),
            Some(Orientation::White),
            "{name} detected"
        );
        // Every glyph at its corner, on light and dark squares (the second image is independent
        // of the one the masks were learned from).
        for (r, c, corner, expect) in [
            (0, 0, Corner::TopLeft, '8'),
            (7, 0, Corner::TopLeft, '1'),
            (7, 7, Corner::BottomRight, 'h'),
            (7, 0, Corner::BottomRight, 'a'),
        ] {
            let read = classify_builtin_corner(&frame, &rect, r, c, corner).expect("readable");
            println!("{name} {r}{c} {corner:?} -> {read:?}");
            assert_eq!(read.glyph, expect, "{name} {r}{c} {corner:?}");
            assert!(read.score >= 0.6 && read.margin >= 0.2, "{name} {read:?}");
        }
    }
    if seen == 0 {
        eprintln!("SKIP: no screenshot present");
    }
}

#[test]
fn labels_work_at_any_scale_and_with_brightness_changes() {
    let mut cases = Vec::new();
    if let Some(img) = fixture() {
        cases.push(("midgame", img, RECT));
    }
    if let Some(img) = start_fixture() {
        cases.push(("start", img, START_RECT));
    }
    for (name, img, rect) in cases {
        for k in [1.3f32, 1.0, 0.85, 0.7, 0.55, 0.45, 0.4] {
            let (im, r) = scale_to(&img, rect, k);
            let frame = frame_of_image(&im);
            assert_eq!(
                detect_orientation(&frame, &r, Site::ChessCom),
                Some(Orientation::White),
                "{name} at {k}"
            );
            // Flipped, labels included.
            let flipped = flipped_view(&im, &r, true);
            assert_eq!(
                detect_orientation(&frame_of_image(&flipped), &r, Site::ChessCom),
                Some(Orientation::Black),
                "{name} flipped at {k}"
            );
        }
        for b in [0.8f32, 0.9, 1.1] {
            let mut im = img.clone();
            for p in im.pixels_mut() {
                for c in 0..3 {
                    p[c] = (p[c] as f32 * b).round().clamp(0.0, 255.0) as u8;
                }
            }
            assert_eq!(
                detect_orientation(&frame_of_image(&im), &rect, Site::ChessCom),
                Some(Orientation::White),
                "{name} brightness {b}"
            );
        }
    }
}

#[test]
fn labels_survive_a_slightly_wrong_board_rectangle() {
    let Some(img) = fixture() else { return };
    let flipped = flipped_view(&img, &RECT, true);
    for (dx, dy, ds) in [
        (2.0, 0.0, 0.0),
        (-2.0, 2.0, 0.0),
        (0.0, -3.0, 4.0),
        (3.0, 3.0, -6.0),
    ] {
        let rect = BoardRect {
            x: RECT.x + dx,
            y: RECT.y + dy,
            size: RECT.size + ds,
        };
        assert_eq!(
            detect_orientation(&frame_of_image(&img), &rect, Site::ChessCom),
            Some(Orientation::White),
            "{rect:?}"
        );
        assert_eq!(
            detect_orientation(&frame_of_image(&flipped), &rect, Site::ChessCom),
            Some(Orientation::Black),
            "{rect:?}"
        );
    }
}

#[test]
fn highlighted_corner_squares_still_read() {
    let Some(img) = fixture() else { return };
    for flipped in [false, true] {
        let base = if flipped {
            flipped_view(&img, &RECT, true)
        } else {
            img.clone()
        };
        let want = if flipped {
            Orientation::Black
        } else {
            Orientation::White
        };
        for over_label in [true, false] {
            // Both label corners highlighted, then just one at a time.
            for squares in [vec![(0, 0), (7, 7)], vec![(0, 0)], vec![(7, 7)]] {
                let mut im = base.clone();
                for (r, c) in &squares {
                    highlight_square(&mut im, &RECT, *r, *c, over_label);
                }
                assert_eq!(
                    detect_orientation(&frame_of_image(&im), &RECT, Site::ChessCom),
                    Some(want),
                    "flipped={flipped} over_label={over_label} {squares:?}"
                );
            }
        }
    }
}

#[test]
fn hidden_labels_give_none_and_nothing_wrong() {
    let Some(img) = fixture() else { return };
    let bare = erase_labels(&img, &RECT);
    assert_eq!(
        detect_orientation(&frame_of_image(&bare), &RECT, Site::ChessCom),
        None
    );
    let bare_flipped = flipped_view(&img, &RECT, false);
    assert_eq!(
        detect_orientation(&frame_of_image(&bare_flipped), &RECT, Site::ChessCom),
        None
    );
    // Labels drawn outside the board (the rectangle covers only the squares) are not in it.
    let wide = BoardRect {
        x: RECT.x + 40.0,
        y: RECT.y + 40.0,
        size: RECT.size - 80.0,
    };
    let r = detect_orientation(&frame_of_image(&img), &wide, Site::ChessCom);
    assert!(r.is_none() || r == Some(Orientation::White), "{r:?}");
    // Only one corner readable: the top-left square's digit erased -> the bottom letter decides.
    let mut half = img.clone();
    let s = RECT.size / 8.0;
    for y in 0..(0.4 * s) as u32 {
        for x in 0..(0.4 * s) as u32 {
            let p = *half.get_pixel(RECT.x as u32 + 150, RECT.y as u32 + 8);
            half.put_pixel(RECT.x as u32 + x, RECT.y as u32 + y, p);
        }
    }
    assert_eq!(
        detect_orientation(&frame_of_image(&half), &RECT, Site::ChessCom),
        Some(Orientation::White)
    );
    // Contradicting corners: an upside-down mix (white's top-left digit, black's letter) is None.
    let flipped = flipped_view(&img, &RECT, true);
    let mut mixed = img.clone();
    let q = s.round() as u32;
    let (bx, by) = (
        (RECT.x + 7.0 * s).round() as u32,
        (RECT.y + 7.0 * s).round() as u32,
    );
    for y in 0..q {
        for x in 0..q {
            mixed.put_pixel(bx + x, by + y, *flipped.get_pixel(bx + x, by + y));
        }
    }
    assert_eq!(
        detect_orientation(&frame_of_image(&mixed), &RECT, Site::ChessCom),
        None
    );
}

#[test]
fn synthetic_boards_never_read_wrong_labels() {
    use crate::test_support::{GREEN, Scene, render};
    for white_bottom in [true, false] {
        let mut sc = Scene::new(placement_from_fen(PLACEMENT));
        sc.white_bottom = white_bottom;
        sc.theme = GREEN;
        let frame = render(&sc);
        let rect = detect_board(&frame, Site::ChessCom).expect("board");
        // The synthetic labels are not chess.com glyphs: None, or at worst the right answer.
        let want = if white_bottom {
            Orientation::White
        } else {
            Orientation::Black
        };
        let got = detect_orientation(&frame, &rect, Site::ChessCom);
        assert!(got.is_none() || got == Some(want), "{got:?}");
    }
}

// ---------------------------------------------------------------------------
// structure
// ---------------------------------------------------------------------------

fn flip_placement(p: &Placement) -> Placement {
    let mut r = *p;
    for (i, c) in p.iter().enumerate() {
        r[63 - i] = *c;
    }
    r
}

fn verdict(
    fen_placement: &str,
    black_at_bottom: bool,
    mover: Option<char>,
    last: Option<(u8, u8)>,
) -> Structural {
    let truth = placement_from_fen(fen_placement);
    // What a reader that assumes white at the bottom sees.
    let (seen, last) = if black_at_bottom {
        (flip_placement(&truth), last.map(|(a, b)| (63 - a, 63 - b)))
    } else {
        (truth, last)
    };
    structural_orientation(&seen, mover, last)
}

#[test]
fn structure_decides_openings_and_middlegames_either_way_round() {
    use Orientation::{Black, White};
    for (fen, last, mover) in [
        ("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR", None, None),
        (PLACEMENT, Some((12u8, 20u8)), Some('b')),
        (PLACEMENT, None, None),
        (
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R",
            None,
            Some('w'),
        ),
        (
            "r2q1rk1/ppp2ppp/2np1n2/2b1p1B1/2B1P1b1/2NP1N2/PPP2PPP/R2Q1RK1",
            None,
            None,
        ),
    ] {
        assert_eq!(
            verdict(fen, false, mover, last),
            Structural::Decided(White),
            "{fen}"
        );
        assert_eq!(
            verdict(fen, true, mover, last),
            Structural::Decided(Black),
            "{fen}"
        );
    }
}

#[test]
fn structure_refuses_to_guess_when_both_fit() {
    for fen in [
        "8/8/4k3/8/4P3/4K3/8/8",   // king and pawn endgame
        "8/5k2/8/8/8/8/6K1/8",     // bare kings
        "8/5k2/8/8/8/8/6K1/7R",    // lone rook
        "8/2p5/3k4/8/8/3K4/2P5/8", // symmetric pawns
    ] {
        assert_eq!(
            verdict(fen, false, None, None),
            Structural::Ambiguous,
            "{fen}"
        );
        assert_eq!(
            verdict(fen, true, None, None),
            Structural::Ambiguous,
            "{fen}"
        );
    }
}

#[test]
fn structure_uses_legality_and_the_direction_pawns_move() {
    use Orientation::White;
    // A white pawn that just went a2-a4 cannot be a pawn going backwards: only one view fits,
    // even though the bare position is ambiguous.
    let fen = "4k3/8/8/8/P7/8/8/4K3";
    assert_eq!(verdict(fen, false, None, None), Structural::Ambiguous);
    // a2 = index 48, a4 = index 32 in the view with white at the bottom.
    assert_eq!(
        verdict(fen, false, Some('w'), Some((48, 32))),
        Structural::Decided(White)
    );
    assert_eq!(
        verdict(fen, true, Some('w'), Some((48, 32))),
        Structural::Decided(Orientation::Black)
    );
    // The side that just moved cannot be left in check: a black pawn on d2 attacks the white king
    // on e1, so with white having just moved that view is impossible and the other one wins.
    assert_eq!(
        structural_orientation(
            &placement_from_fen("4k3/8/8/8/8/8/3p4/4K3"),
            Some('w'),
            None
        ),
        Structural::Decided(Orientation::Black)
    );
    // Garbage (two white kings) is unreadable either way round.
    assert_eq!(
        structural_orientation(&placement_from_fen("4k3/8/8/8/8/8/8/K3K3"), None, None),
        Structural::Unreadable
    );
    // Too many unknown squares are not judged.
    let mut p = placement_from_fen(PLACEMENT);
    for c in p.iter_mut().take(12) {
        *c = '?';
    }
    assert_eq!(
        structural_orientation(&p, None, None),
        Structural::Unreadable
    );
}

// ---------------------------------------------------------------------------
// pipeline on the real screenshot
// ---------------------------------------------------------------------------

fn run(
    pipe: &mut BoardPipeline,
    rec: &Recognizer,
    img: &RgbaImage,
    rect: &BoardRect,
    n: usize,
) -> PipelineOutput {
    let frame = frame_of_image(img);
    let mut out = None;
    for i in 0..n {
        out = Some(pipe.process(rec, &frame, rect, Site::ChessCom, i == 0));
    }
    out.unwrap()
}

fn placement_field(out: &PipelineOutput) -> String {
    out.tracked
        .fen
        .clone()
        .unwrap_or_default()
        .split(' ')
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn white_bottom_screenshots_track_with_white_without_calibration() {
    let rec = builtin();
    for (name, img, rect, want) in [
        ("midgame", fixture(), RECT, Some(PLACEMENT)),
        ("start", start_fixture(), START_RECT, None),
    ] {
        let Some(img) = img else { continue };
        let mut pipe = BoardPipeline::new(OrientationMode::Auto);
        let out = run(&mut pipe, &rec, &img, &rect, 3);
        assert_eq!(out.orientation, Some(Orientation::White), "{name}");
        assert_eq!(
            pipe.resolver.source(),
            Some(OrientationSource::Labels),
            "{name}"
        );
        if let Some(want) = want {
            assert_eq!(out.tracked.state, TrackState::Tracking, "{name}");
            assert_eq!(placement_field(&out), want, "{name}");
        } else {
            // The start position needs no calibration either.
            assert_eq!(out.tracked.state, TrackState::Tracking, "{name}");
            assert!(
                placement_field(&out).starts_with("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR"),
                "{name}"
            );
        }
    }
}

#[test]
fn faithful_flipped_view_reads_the_same_real_position() {
    let Some(img) = fixture() else { return };
    let rec = builtin();
    let flipped = flipped_view(&img, &RECT, true);
    // The old default path (white assumed) reads the rotated position confidently: this is the bug.
    let wrong = rec.read(&frame_of_image(&flipped), &RECT, Orientation::White);
    let text: String = wrong.placement.iter().collect();
    assert_ne!(
        text,
        crate::real_tests::text(&placement_from_fen(PLACEMENT))
    );
    assert!(text.starts_with("RNBKQBNR"), "{text}");
    // The pipeline never takes that path.
    for scale in [1.0f32, 0.7, 0.5] {
        let (im, rect) = scale_to(&flipped, RECT, scale);
        let mut pipe = BoardPipeline::new(OrientationMode::Auto);
        let frame = frame_of_image(&im);
        for i in 0..4 {
            let out = pipe.process(&rec, &frame, &rect, Site::ChessCom, i == 0);
            if let Some(f) = &out.tracked.fen {
                assert!(
                    f.starts_with(PLACEMENT),
                    "scale {scale} frame {i}: confidently wrong {f}"
                );
            }
            if i == 3 {
                assert_eq!(out.orientation, Some(Orientation::Black), "scale {scale}");
                assert_eq!(out.tracked.state, TrackState::Tracking, "scale {scale}");
                assert_eq!(out.tracked.orientation, Orientation::Black);
                assert_eq!(placement_field(&out), PLACEMENT, "scale {scale}");
                assert_eq!(
                    out.tracked.fen.as_deref().and_then(|f| f.split(' ').nth(1)),
                    Some("w")
                );
            }
        }
        assert_eq!(pipe.resolver.source(), Some(OrientationSource::Labels));
    }
}

#[test]
fn without_labels_the_structure_decides_and_never_guesses() {
    let Some(img) = fixture() else { return };
    let rec = builtin();
    for (name, im, want) in [
        ("white", erase_labels(&img, &RECT), Orientation::White),
        (
            "black",
            flipped_view(&img, &RECT, false),
            Orientation::Black,
        ),
    ] {
        let mut pipe = BoardPipeline::new(OrientationMode::Auto);
        let out = run(&mut pipe, &rec, &im, &RECT, 4);
        assert_eq!(out.orientation, Some(want), "{name}");
        assert_eq!(
            pipe.resolver.source(),
            Some(OrientationSource::Structure),
            "{name}"
        );
        assert_eq!(placement_field(&out), PLACEMENT, "{name}");
    }
}

#[test]
fn undecidable_position_is_not_tracked_and_says_why() {
    // A king and pawn ending drawn with chess.com glyph-less synthetic pieces: learn the pieces
    // from a synthetic start position, then show the ending with black at the bottom.
    use crate::test_support::{GREEN, Scene, render};
    let mut start = Scene::new(crate::test_support::start_placement());
    start.theme = GREEN;
    let frame = render(&start);
    let rect = detect_board(&frame, Site::ChessCom).expect("board");
    let (templates, _) = learn_from_start(&frame, &rect).expect("learn");
    let rec = Recognizer::new(&templates).unwrap();
    let mut sc = Scene::new(placement_from_fen("8/8/4k3/8/4P3/4K3/8/8"));
    sc.theme = GREEN;
    sc.white_bottom = false;
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::ChessCom).expect("board");
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    for i in 0..6 {
        let out = pipe.process(&rec, &frame, &rect, Site::ChessCom, i == 0);
        assert_eq!(out.tracked.state, TrackState::Lost);
        assert!(
            out.tracked.fen.is_none(),
            "must not guess: {:?}",
            out.tracked.fen
        );
        assert_eq!(out.orientation, None);
        if i > 0 {
            assert_eq!(out.status.as_deref(), Some(MSG_CANT_TELL));
        }
    }
    assert!(
        MSG_CANT_TELL
            .to_lowercase()
            .contains("can't tell which side is at the bottom")
    );
    // Choosing the orientation fixes it.
    assert!(pipe.resolver.set_mode(OrientationMode::Black));
    let mut out = None;
    for _ in 0..4 {
        out = Some(pipe.process(&rec, &frame, &rect, Site::ChessCom, false));
    }
    let out = out.unwrap();
    assert_eq!(out.tracked.state, TrackState::Tracking);
    assert_eq!(placement_field(&out), "8/8/4k3/8/4P3/4K3/8/8");
}

#[test]
fn manual_orientation_is_final_but_reports_a_contradiction() {
    let Some(img) = fixture() else { return };
    let rec = builtin();
    let flipped = flipped_view(&img, &RECT, true);
    // Correct choice on the flipped board: tracked without any detection.
    let mut pipe = BoardPipeline::new(OrientationMode::Black);
    let out = run(&mut pipe, &rec, &flipped, &RECT, 3);
    assert_eq!(out.orientation, Some(Orientation::Black));
    assert_eq!(pipe.resolver.source(), Some(OrientationSource::Manual));
    assert_eq!(placement_field(&out), PLACEMENT);
    assert!(out.status.is_none());
    // A choice the labels contradict is obeyed, and flagged.
    let mut pipe = BoardPipeline::new(OrientationMode::White);
    let out = run(&mut pipe, &rec, &flipped, &RECT, 3);
    assert_eq!(out.orientation, Some(Orientation::White));
    let status = out.status.expect("conflict reported");
    assert!(
        status.contains("white") && status.contains("black"),
        "{status}"
    );
    // Back to auto: the labels take over and the tracker restarts.
    assert!(pipe.resolver.set_mode(OrientationMode::Auto));
    pipe.reset_tracker();
    let out = run(&mut pipe, &rec, &flipped, &RECT, 4);
    assert_eq!(out.orientation, Some(Orientation::Black));
    assert_eq!(placement_field(&out), PLACEMENT);
    // The mode parses like the Tauri command argument.
    assert_eq!(OrientationMode::parse("auto"), Some(OrientationMode::Auto));
    assert_eq!(
        OrientationMode::parse("black"),
        Some(OrientationMode::Black)
    );
    assert_eq!(OrientationMode::parse("Black"), None);
}

#[test]
fn a_new_game_as_the_other_colour_flips_without_a_wrong_position() {
    let Some(img) = fixture() else { return };
    let rec = builtin();
    let flipped = flipped_view(&img, &RECT, true);
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    let out = run(&mut pipe, &rec, &img, &RECT, 4);
    assert_eq!(out.orientation, Some(Orientation::White));
    // The board is turned around (same position seen from the other side).
    let frame = frame_of_image(&flipped);
    let mut flipped_at = None;
    for i in 0..(RECHECK_FRAMES as usize * 2 + 4) {
        let out = pipe.process(&rec, &frame, &RECT, Site::ChessCom, false);
        if let Some(f) = &out.tracked.fen {
            assert!(f.starts_with(PLACEMENT), "frame {i}: confidently wrong {f}");
        }
        if out.orientation == Some(Orientation::Black) && out.tracked.state == TrackState::Tracking
        {
            flipped_at = Some(i);
            break;
        }
    }
    let at = flipped_at.expect("flipped to black");
    assert!(at <= 12, "took {at} frames");
    assert_eq!(pipe.resolver.source(), Some(OrientationSource::Labels));
}

#[test]
fn the_tracker_rotation_logic_is_off_inside_the_pipeline_and_on_alone() {
    // Alone, the tracker joins a start position seen from black's side by itself.
    let mut tr = PositionTracker::new();
    let rotated_start = flip_placement(&crate::test_support::start_placement());
    let reading = Reading {
        placement: rotated_start,
        orientation: Orientation::White,
        confidence: 0.9,
        mover_hint: None,
        last_move_squares: None,
    };
    tr.update(&reading);
    let out = tr.update(&reading);
    assert_eq!(out.state, TrackState::Tracking);
    assert_eq!(out.orientation, Orientation::Black);
    // Locked, the same reading is just an odd position that is adopted as read: the orientation
    // stays what the caller said.
    let mut tr = PositionTracker::new();
    tr.lock_orientation(true);
    tr.update(&reading);
    let out = tr.update(&reading);
    assert_eq!(out.orientation, Orientation::White);
}

#[test]
fn known_position_decides_an_unknown_orientation() {
    let Some(img) = fixture() else { return };
    let rec = builtin();
    let flipped = flipped_view(&img, &RECT, false); // no labels, so nothing is known yet
    let frame = frame_of_image(&flipped);
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    let fen = format!("{PLACEMENT} w KQkq - 0 3");
    pipe.set_known_position(&rec, &frame, &RECT, &fen)
        .expect("matches one view");
    assert_eq!(pipe.resolver.current(), Some(Orientation::Black));
    // A position that matches neither view is refused.
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    let err = pipe
        .set_known_position(&rec, &frame, &RECT, "4k3/8/8/8/8/8/8/4K3 w - - 0 1")
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("can't tell which side"),
        "{err}"
    );
}

/// Labels decided the orientation, then the board turns around while the labels become
/// unreadable: the stale label decision must not survive, and nothing rotated may be published.
fn label_decision_goes_stale(
    held_img: &RgbaImage,
    held: Orientation,
    next_img: &RgbaImage,
    rect: &BoardRect,
    want_placement: &str,
) {
    let rec = builtin();
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    let out = run(&mut pipe, &rec, held_img, rect, 4);
    assert_eq!(out.orientation, Some(held));
    assert_eq!(pipe.resolver.source(), Some(OrientationSource::Labels));
    let other = if held == Orientation::White {
        Orientation::Black
    } else {
        Orientation::White
    };
    let frame = frame_of_image(next_img);
    let mut settled = None;
    for i in 0..40 {
        let out = pipe.process(&rec, &frame, rect, Site::ChessCom, false);
        if let Some(f) = &out.tracked.fen {
            assert!(
                f.starts_with(want_placement),
                "frame {i}: confidently wrong {f}"
            );
            // The overlay mirrors arrows from this: it never lags the decision.
            assert_eq!(Some(out.tracked.orientation), out.orientation, "frame {i}");
        }
        if out.tracked.state == TrackState::Tracking {
            if out.orientation != Some(other) {
                continue; // the last good position, still shown while the flip is confirmed
            }
            settled = Some(i);
            break;
        }
    }
    let at = settled.expect("recovered to the new orientation");
    assert!(at <= 12, "took {at} frames");
    // And it stays right, across re-detection too.
    for i in 0..40 {
        let out = pipe.process(&rec, &frame, rect, Site::ChessCom, i == 20);
        assert_eq!(out.tracked.state, TrackState::Tracking, "frame {i}");
        assert!(
            out.tracked
                .fen
                .as_deref()
                .unwrap()
                .starts_with(want_placement)
        );
        assert_eq!(out.tracked.orientation, other);
    }
}

#[test]
fn stale_labels_white_then_black_without_labels() {
    let Some(img) = fixture() else { return };
    label_decision_goes_stale(
        &img,
        Orientation::White,
        &flipped_view(&img, &RECT, false),
        &RECT,
        PLACEMENT,
    );
}

#[test]
fn stale_labels_black_then_white_without_labels() {
    let Some(img) = fixture() else { return };
    label_decision_goes_stale(
        &flipped_view(&img, &RECT, true),
        Orientation::Black,
        &erase_labels(&img, &RECT),
        &RECT,
        PLACEMENT,
    );
}

#[test]
fn stale_labels_on_the_start_screenshot_both_directions() {
    let Some(img) = start_fixture() else { return };
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
    label_decision_goes_stale(
        &img,
        Orientation::White,
        &flipped_view(&img, &START_RECT, false),
        &START_RECT,
        start,
    );
    label_decision_goes_stale(
        &flipped_view(&img, &START_RECT, true),
        Orientation::Black,
        &erase_labels(&img, &START_RECT),
        &START_RECT,
        start,
    );
}
