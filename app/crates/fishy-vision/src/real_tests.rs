//! Tests on the user's own chess.com screenshot (third-party artwork, kept in the git-ignored
//! `tests/fixtures/local/`). Every test skips (prints a notice and passes) when it is absent.

use crate::tracker::*;
use crate::*;
use image::{RgbaImage, imageops};
use std::path::PathBuf;

pub(crate) const PLACEMENT: &str = "rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR";
pub(crate) const RECT: BoardRect = BoardRect {
    x: 68.0,
    y: 22.0,
    size: 1056.0,
};

pub(crate) fn fixture() -> Option<RgbaImage> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/local/chesscom-midgame.png");
    if !path.exists() {
        eprintln!(
            "SKIP: {} is absent (third-party screenshot, not committed)",
            path.display()
        );
        return None;
    }
    Some(image::open(path).expect("readable png").to_rgba8())
}

pub(crate) fn frame_of(img: &RgbaImage) -> Frame {
    Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.clone().into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    }
}

pub(crate) fn truth() -> Placement {
    crate::test_support::placement_from_fen(PLACEMENT)
}

pub(crate) fn text(p: &Placement) -> String {
    p.iter().collect()
}

pub(crate) fn scaled(img: &RgbaImage, k: f32) -> (RgbaImage, BoardRect) {
    let (w, h) = (
        (img.width() as f32 * k).round() as u32,
        (img.height() as f32 * k).round() as u32,
    );
    (
        imageops::resize(img, w, h, imageops::FilterType::Lanczos3),
        BoardRect {
            x: RECT.x * k,
            y: RECT.y * k,
            size: RECT.size * k,
        },
    )
}

pub(crate) fn brightness(img: &RgbaImage, k: f32) -> RgbaImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in 0..3 {
            p[c] = (p[c] as f32 * k).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

pub(crate) type Rgb = [f32; 3];

pub(crate) fn hl(c: Rgb) -> Rgb {
    // chess.com-style highlight: the square under a 50% yellow overlay.
    [0.5 * c[0] + 127.5, 0.5 * c[1] + 127.5, 0.5 * c[2]]
}

/// Copy of the board with the two square colours (and their highlight tints) replaced.
/// Anti-aliased piece edges are re-blended against the new square colour, dots are re-shaded.
pub(crate) fn recolour(img: &RgbaImage, light: Rgb, dark: Rgb) -> RgbaImage {
    let s = RECT.size / 8.0;
    let mut out = img.clone();
    let mut old_bg = [[0.0f32; 3]; 64];
    for (sq, bg) in old_bg.iter_mut().enumerate() {
        let (r, c) = (sq / 8, sq % 8);
        let px = img.get_pixel(
            (RECT.x + c as f32 * s + 4.0) as u32,
            (RECT.y + r as f32 * s + s * 0.5) as u32,
        );
        *bg = [px[0] as f32, px[1] as f32, px[2] as f32];
    }
    let (ol, od) = (old_bg[0], old_bg[1]);
    let dist = |a: Rgb, b: Rgb| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    for (sq, &bo) in old_bg.iter().enumerate() {
        let (r, c) = (sq / 8, sq % 8);
        let is_light = (r + c) % 2 == 0;
        let (ref_old, ref_new) = if is_light { (ol, light) } else { (od, dark) };
        let tinted = dist(bo, ref_old) > 20.0;
        let bn = if tinted { hl(ref_new) } else { ref_new };
        let (x0, y0) = (
            (RECT.x + c as f32 * s) as u32,
            (RECT.y + r as f32 * s) as u32,
        );
        for y in y0..y0 + s as u32 {
            for x in x0..x0 + s as u32 {
                let p = img.get_pixel(x, y);
                let pv = [p[0] as f32, p[1] as f32, p[2] as f32];
                let new: Rgb = if dist(pv, bo) <= 6.0 {
                    bn
                } else {
                    let bb: f32 = bo.iter().map(|v| v * v).sum();
                    let lam = (0..3).map(|k| pv[k] * bo[k]).sum::<f32>() / bb;
                    let res = dist(pv, [bo[0] * lam, bo[1] * lam, bo[2] * lam]);
                    if (0.6..=1.03).contains(&lam) && res <= 10.0 {
                        [bn[0] * lam, bn[1] * lam, bn[2] * lam]
                    } else {
                        // Blend of a piece colour (white or near black) and the old square colour.
                        let mut best: Option<(f32, f32)> = None;
                        for cc in [255.0f32, 40.0] {
                            let cv = [cc; 3];
                            let d: Rgb = [bo[0] - cv[0], bo[1] - cv[1], bo[2] - cv[2]];
                            let dd: f32 = d.iter().map(|v| v * v).sum();
                            let beta = (0..3).map(|k| (pv[k] - cv[k]) * d[k]).sum::<f32>() / dd;
                            let fit = [
                                cv[0] + beta * d[0],
                                cv[1] + beta * d[1],
                                cv[2] + beta * d[2],
                            ];
                            let res = dist(pv, fit);
                            if (0.0..=1.0).contains(&beta)
                                && res <= 14.0
                                && best.is_none_or(|b| res < b.0)
                            {
                                best = Some((res, beta));
                            }
                        }
                        match best {
                            Some((_, beta)) => [
                                pv[0] + beta * (bn[0] - bo[0]),
                                pv[1] + beta * (bn[1] - bo[1]),
                                pv[2] + beta * (bn[2] - bo[2]),
                            ],
                            None => pv,
                        }
                    }
                };
                let o = out.get_pixel_mut(x, y);
                for k in 0..3 {
                    o[k] = new[k].round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

fn variants(img: &RgbaImage) -> Vec<(String, RgbaImage, BoardRect)> {
    let mut v = vec![("original".to_string(), img.clone(), RECT)];
    for k in [0.40f32, 0.55, 0.75, 1.30] {
        let (i, r) = scaled(img, k);
        v.push((format!("scale {k}"), i, r));
    }
    for k in [0.88f32, 1.12] {
        v.push((format!("brightness {k}"), brightness(img, k), RECT));
    }
    v.push((
        "brown theme".into(),
        recolour(img, [240.0, 217.0, 181.0], [181.0, 136.0, 99.0]),
        RECT,
    ));
    v.push((
        "blue theme".into(),
        recolour(img, [222.0, 227.0, 230.0], [140.0, 162.0, 173.0]),
        RECT,
    ));
    let (i, r) = scaled(
        &recolour(img, [240.0, 217.0, 181.0], [181.0, 136.0, 99.0]),
        0.55,
    );
    v.push(("brown theme at 55%".into(), i, r));
    let (i, r) = scaled(
        &recolour(img, [222.0, 227.0, 230.0], [140.0, 162.0, 173.0]),
        0.75,
    );
    v.push(("blue theme at 75%".into(), i, r));
    v
}

fn correct(r: &Reading) -> usize {
    let t = truth();
    (0..64).filter(|&i| r.placement[i] == t[i]).count()
}

#[test]
fn detect_learn_from_placement_read_round_trip() {
    let Some(img) = fixture() else { return };
    let frame = frame_of(&img);
    let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
    assert!(
        (rect.x - RECT.x).abs() < 3.0
            && (rect.y - RECT.y).abs() < 3.0
            && (rect.size - RECT.size).abs() < 4.0,
        "{rect:?}"
    );
    let t = learn_from_placement(&frame, &rect, &truth(), Orientation::White).expect("learn");
    let r = read_board(&frame, &rect, &t, Orientation::White);
    assert_eq!(text(&r.placement), text(&truth()));
    assert!(r.confidence > 0.8, "confidence {}", r.confidence);
    // e7-e6 was the last move; the selected queen on d1 must not confuse the hint.
    assert_eq!(r.mover_hint, Some('b'));
    assert_eq!(r.last_move_squares, Some((12, 20)));
    // Kinds that do not occur get no template, others still read.
    let mut only_white = truth();
    for c in only_white.iter_mut() {
        if c.is_ascii_lowercase() {
            *c = '.';
        }
    }
    // (the screenshot still shows black pieces, so the self check must refuse this lie)
    assert!(learn_from_placement(&frame, &rect, &only_white, Orientation::White).is_err());
}

#[test]
fn learning_rejects_a_wrong_placement() {
    let Some(img) = fixture() else { return };
    let frame = frame_of(&img);
    let start = crate::test_support::start_placement();
    assert!(learn_from_placement(&frame, &RECT, &start, Orientation::White).is_err());
}

#[test]
fn builtin_templates_are_small_and_only_for_chesscom_and_lichess() {
    let t = Templates::builtin(Site::ChessCom).expect("chess.com builtin");
    let len = serde_json::to_string(&t.data).unwrap().len();
    assert!(len < 100_000, "{len} bytes");
    assert!(Recognizer::new(&t).is_some());
    let li = Templates::builtin(Site::Lichess).expect("lichess builtin");
    assert!(serde_json::to_string(&li.data).unwrap().len() < 100_000);
    assert!(Recognizer::new(&li).is_some());
    assert!(Templates::builtin(Site::Generic).is_none());
}

#[test]
fn builtin_templates_read_the_real_board_in_every_variant() {
    let Some(img) = fixture() else { return };
    let rec = Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap();
    let mut failures = Vec::new();
    for (name, im, rect) in variants(&img) {
        let r = rec.read(&frame_of(&im), &rect, Orientation::White);
        let ok = correct(&r);
        println!(
            "{name:28} {ok}/64 confidence {:.2} mover {:?} last {:?}",
            r.confidence, r.mover_hint, r.last_move_squares
        );
        if ok != 64 || r.mover_hint != Some('b') {
            failures.push(format!(
                "{name}: {ok}/64 got {} mover {:?}",
                text(&r.placement),
                r.mover_hint
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn builtin_templates_after_detection_at_other_sizes() {
    let Some(img) = fixture() else { return };
    let rec = Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap();
    for k in [0.55f32, 1.3] {
        let (im, _) = scaled(&img, k);
        let frame = frame_of(&im);
        let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
        let r = rec.read(&frame, &rect, Orientation::White);
        assert_eq!(correct(&r), 64, "scale {k}: {}", text(&r.placement));
    }
}

#[test]
fn tracker_joins_the_real_game_in_progress() {
    let Some(img) = fixture() else { return };
    let rec = Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap();
    for (name, im, rect) in variants(&img) {
        let frame = frame_of(&im);
        let mut tr = PositionTracker::new();
        let mut states = Vec::new();
        for _ in 0..3 {
            let r = rec.read(&frame, &rect, Orientation::White);
            states.push(tr.update(&r));
        }
        assert_eq!(states[0].state, TrackState::NeedsCalibration, "{name}");
        let last = &states[2];
        assert_eq!(last.state, TrackState::Tracking, "{name}");
        let fen = last.fen.clone().expect("fen");
        let mut parts = fen.split(' ');
        assert_eq!(parts.next(), Some(PLACEMENT), "{name}");
        assert_eq!(parts.next(), Some("w"), "{name}: {fen}");
        assert_eq!(
            states[1].state,
            TrackState::Tracking,
            "{name}: tracking after two readings"
        );
    }
}

// ---------------------------------------------------------------------------
// mid-game calibration on private Lichess screenshots (local validation only)
// ---------------------------------------------------------------------------

const LICHESS_DIR: &str = "/private/tmp/claude-501/-Users-ahmed-fishy-engine/8ec90d43-9535-4602-8da4-c1b3ececeb14/images";
/// Brown board, black at the bottom, arrows over a few pieces.
const LICHESS_1: &str = "r1bqkbnr/ppp1pppp/2n5/3p4/3P4/2N2N2/PPP1PPPP/R1BQKB1R";
/// Same set, clean, an endgame (no rooks, one knight).
const LICHESS_2: &str = "3N4/5pb1/3Q3p/4pk2/1P6/8/b1P2PPP/2K5";

fn lichess(n: u8) -> Option<(Frame, BoardRect)> {
    let path = PathBuf::from(LICHESS_DIR).join(format!("{n}.png"));
    if !path.exists() {
        eprintln!("SKIP: lichess screenshot {n} is absent (private, not committed)");
        return None;
    }
    let img = image::open(path).ok()?.to_rgba8();
    let f = frame_of(&img);
    let rect = detect_board(&f, Site::Lichess)?;
    Some((f, rect))
}

fn wrong(t: &Templates, f: &Frame, r: &BoardRect, o: Orientation, want: &Placement) -> usize {
    let got = read_board(f, r, t, o);
    (0..64).filter(|&i| got.placement[i] != want[i]).count()
}

#[test]
fn midgame_calibration_on_real_lichess_screenshots() {
    let (Some((f1, r1)), Some((f2, r2))) = (lichess(1), lichess(2)) else { return };
    let p1 = placement_of_fen(LICHESS_1).unwrap();
    let p2 = placement_of_fen(LICHESS_2).unwrap();
    let b = Orientation::Black;

    // 1: arrows over a few pieces, a middlegame with every kind present but one knight pair.
    let a1 = learn_from_placement_auto(&f1, &r1, &p1, None).expect("learn from 1");
    assert_eq!(a1.orientation, b);
    let w11 = wrong(&a1.templates, &f1, &r1, b, &p1);
    eprintln!("real: 1->1 wrong {w11}, missing {:?}", missing_kind_names(&a1.kinds_seen));
    assert!(w11 <= 4);

    // 2: an endgame; either learns with the right orientation or refuses, never guesses wrong.
    match learn_from_placement_auto(&f2, &r2, &p2, None) {
        Ok(a2) => {
            assert_eq!(a2.orientation, b);
            let w22 = wrong(&a2.templates, &f2, &r2, b, &p2);
            eprintln!("real: 2->2 wrong {w22}, missing {:?}", missing_kind_names(&a2.kinds_seen));
            let cross = wrong(&a2.templates, &f1, &r1, b, &p1);
            let cross_back = wrong(&a1.templates, &f2, &r2, b, &p2);
            eprintln!("real: 2->1 wrong {cross}, 1->2 wrong {cross_back}");
            let m = merge_templates(&a1.templates, &a2.templates, &f2, &r2, &p2, b);
            assert!(m.merged, "same set from the same site merges");
            assert!(wrong(&m.templates, &f2, &r2, b, &p2) <= 4);
            assert!(wrong(&m.templates, &f1, &r1, b, &p1) <= 4);
            eprintln!(
                "real: merged={} reads 1: {}, reads 2: {}",
                m.merged,
                wrong(&m.templates, &f1, &r1, b, &p1),
                wrong(&m.templates, &f2, &r2, b, &p2)
            );
        }
        Err(e) => {
            let m = e.to_string();
            eprintln!("real: 2 refused: {m}");
            assert!(m.contains("which side") || m.contains("not recognised"), "{m}");
        }
    }
}
