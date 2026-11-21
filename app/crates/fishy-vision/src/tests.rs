use crate::detect::*;
use crate::test_support::*;
use std::time::Instant;

fn check_rect(r: BoardRect, sc: &Scene, tol: f32) {
    let want = (sc.board_x as f32, sc.board_y as f32, (sc.square * 8) as f32);
    assert!(
        (r.x - want.0).abs() <= tol
            && (r.y - want.1).abs() <= tol
            && (r.size - want.2).abs() <= tol * 2.0,
        "detected {r:?} expected {want:?}"
    );
}

#[test]
fn detects_themes_scales_offsets() {
    for (name, theme) in [
        ("green", GREEN),
        ("brown", BROWN),
        ("blue", BLUE),
        ("custom", CUSTOM),
    ] {
        for k in [1u32, 2] {
            for (ox, oy, sq) in [(232, 72, 80), (301, 113, 67), (140, 60, 90)] {
                let mut sc = Scene::new(start_placement());
                sc.theme = theme;
                sc.board_x = ox;
                sc.board_y = oy;
                sc.square = sq;
                sc.highlights = vec![52, 36];
                sc.dots = vec![20, 28];
                let sc = sc.at_scale(k);
                let frame = render(&sc);
                for site in [Site::ChessCom, Site::Generic] {
                    let r = detect_board(&frame, site)
                        .unwrap_or_else(|| panic!("{name} k={k} sq={sq} {site:?}: no board"));
                    check_rect(r, &sc, 1.0);
                    assert!(verify_board(&frame, &r, site), "{name} verify");
                }
            }
        }
    }
}

#[test]
fn detects_with_arrows_flipped_and_mid_game() {
    let mut sc = Scene::new(placement_from_fen(
        "r1bqk2r/ppp2ppp/2n2n2/3pp3/1b1PP3/2N2N2/PPP2PPP/R1BQKB1R",
    ));
    sc.white_bottom = false;
    sc.theme = BROWN;
    sc.arrows = vec![(60, 45), (4, 20), (57, 42)];
    sc.rings = vec![27];
    sc.noise = 4;
    let frame = render(&sc);
    let r = detect_board(&frame, Site::Lichess).expect("board");
    check_rect(r, &sc, 1.0);
}

#[test]
fn no_board_no_detection() {
    let mut sc = Scene::new(['.'; 64]);
    sc.square = 8; // tiny "board" below the minimum size
    sc.board_x = 400;
    let frame = render(&sc);
    assert!(detect_board(&frame, Site::ChessCom).is_none());
}

#[test]
fn verify_rejects_moved_board() {
    let sc = Scene::new(start_placement());
    let frame = render(&sc);
    let r = detect_board(&frame, Site::ChessCom).unwrap();
    assert!(verify_board(&frame, &r, Site::ChessCom));
    let moved = BoardRect { x: r.x + 14.0, ..r };
    assert!(!verify_board(&frame, &moved, Site::ChessCom));
    let bigger = BoardRect {
        size: r.size + 24.0,
        ..r
    };
    assert!(!verify_board(&frame, &bigger, Site::ChessCom));
}

#[test]
fn detection_performance_2560x1440() {
    let mut sc = Scene::new(start_placement());
    sc.width = 2560;
    sc.height = 1440;
    sc.board_x = 640;
    sc.board_y = 160;
    sc.square = 128;
    sc.highlights = vec![52, 36];
    let frame = render(&sc);
    let _ = detect_board(&frame, Site::ChessCom);
    let t = Instant::now();
    let r = detect_board(&frame, Site::ChessCom).expect("board");
    let detect_ms = t.elapsed().as_secs_f64() * 1000.0;
    check_rect(r, &sc, 1.0);
    let t = Instant::now();
    for _ in 0..20 {
        assert!(verify_board(&frame, &r, Site::ChessCom));
    }
    let verify_ms = t.elapsed().as_secs_f64() * 1000.0 / 20.0;
    eprintln!("detect {detect_ms:.2} ms, verify {verify_ms:.3} ms");
    if !cfg!(debug_assertions) {
        assert!(detect_ms < 100.0, "detect too slow: {detect_ms}");
        assert!(verify_ms < 5.0, "verify too slow: {verify_ms}");
    }
}

// ---------------------------------------------------------------------------
// recognition
// ---------------------------------------------------------------------------

use crate::recognize::*;

const MID_FENS: &[&str] = &[
    "r1bqk2r/ppp2ppp/2n2n2/3pp3/1b1PP3/2N2N2/PPP2PPP/R1BQKB1R",
    "r4rk1/pp3ppp/2n1bn2/q2pp3/3P4/2PBBN2/PP1Q1PPP/R3K2R",
    "2kr3r/ppp2ppp/2n5/3Qn3/8/2N2N2/PPP2PPP/R3K2R",
    "8/5Q2/8/3k4/8/2K5/8/1q5Q",
    "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR",
    "3Q4/1P6/8/8/2k5/8/5K2/q2Q4",
];

fn learn(
    theme: Theme,
    k: u32,
    white_bottom: bool,
) -> (Scene, crate::detect::BoardRect, Templates, Orientation) {
    let mut sc = Scene::new(start_placement());
    sc.theme = theme;
    sc.white_bottom = white_bottom;
    let sc = sc.at_scale(k);
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::ChessCom).expect("board");
    let (t, o) = learn_from_start(&frame, &rect).expect("learn");
    (sc, rect, t, o)
}

fn assert_reads(
    frame: &crate::capture::Frame,
    rect: &crate::detect::BoardRect,
    t: &Templates,
    o: Orientation,
    truth: &Placement,
    what: &str,
) {
    let r = read_board(frame, rect, t, o);
    let wrong: Vec<usize> = (0..64).filter(|&i| r.placement[i] != truth[i]).collect();
    assert!(
        wrong.is_empty(),
        "{what}: wrong squares {wrong:?}: got {:?}",
        r.placement.iter().collect::<String>()
    );
    assert!(
        r.confidence > 0.7,
        "{what}: low confidence {}",
        r.confidence
    );
}

#[test]
fn learn_and_read_round_trip_all_themes() {
    for (name, theme) in [("green", GREEN), ("brown", BROWN), ("blue", BLUE)] {
        for k in [1u32, 2] {
            for white_bottom in [true, false] {
                let (sc, rect, t, o) = learn(theme, k, white_bottom);
                assert_eq!(
                    o,
                    if white_bottom {
                        Orientation::White
                    } else {
                        Orientation::Black
                    },
                    "{name} orientation"
                );
                for (n, fen) in MID_FENS.iter().enumerate() {
                    let truth = placement_from_fen(fen);
                    let mut s = sc.clone();
                    s.placement = truth;
                    s.seed = 100 + n as u64;
                    let frame = render(&s);
                    assert_reads(
                        &frame,
                        &rect,
                        &t,
                        o,
                        &truth,
                        &format!("{name} k={k} wb={white_bottom} fen#{n}"),
                    );
                }
            }
        }
    }
}

#[test]
fn small_boards_on_large_frames_are_found_at_every_size() {
    // Sizes that used to alias in the downscaled lattice (4K frame, centred, no noise).
    let cases: [(Theme, &[u32]); 2] = [
        (GREEN, &[24, 25, 27, 29, 31, 33, 35, 37, 39]),
        (BLUE, &[24, 25, 27, 31, 33, 37, 43, 55, 61]),
    ];
    for (theme, sizes) in cases {
        for &sq in sizes {
            let mut sc = Scene::new(start_placement());
            sc.theme = theme;
            sc.width = 3840;
            sc.height = 2160;
            sc.square = sq;
            sc.board_x = ((3840 - 8 * sq) / 2) as i32;
            sc.board_y = ((2160 - 8 * sq) / 2) as i32;
            sc.labels = false;
            sc.noise = 0;
            let frame = render(&sc);
            let rect = detect_board(&frame, Site::ChessCom)
                .unwrap_or_else(|| panic!("no board at square size {sq}"));
            assert!(
                (rect.x - sc.board_x as f32).abs() <= 2.0
                    && (rect.size - 8.0 * sq as f32).abs() <= 3.0,
                "size {sq}: {rect:?}"
            );
        }
    }
}

#[test]
fn reading_survives_global_brightness_and_tint_shifts() {
    for (name, theme) in [("green", GREEN), ("brown", BROWN), ("blue", BLUE)] {
        let (sc, rect, t, o) = learn(theme, 1, true);
        let truth = placement_from_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R");
        let mut s = sc.clone();
        s.placement = truth;
        let base = render(&s);
        for gain in [
            [0.97, 0.97, 0.97],
            [0.92; 3],
            [0.90; 3],
            [0.85; 3],
            [1.0, 0.93, 0.8],
        ] {
            let mut frame = base.clone();
            for px in frame.rgba.chunks_exact_mut(4) {
                for k in 0..3 {
                    px[k] = (px[k] as f32 * gain[k]).round().min(255.0) as u8;
                }
            }
            assert_reads(
                &frame,
                &rect,
                &t,
                o,
                &truth,
                &format!("{name} gain {gain:?}"),
            );
        }
    }
}

#[test]
fn reading_ignores_highlights_dots_arrows_labels() {
    let (sc, rect, t, o) = learn(GREEN, 1, true);
    let truth = placement_from_fen(MID_FENS[0]);
    let mut s = sc.clone();
    s.placement = truth;
    s.highlights = vec![52, 36, 45];
    s.dots = vec![20, 21, 29, 37, 11];
    s.rings = vec![35, 27];
    s.arrows = vec![(60, 45), (59, 32), (4, 20)];
    s.noise = 4;
    assert_reads(&render(&s), &rect, &t, o, &truth, "decorated");
    // Heavy arrow pass over pieces may produce '?' but never a wrong piece.
    s.arrows = vec![(61, 34), (58, 37), (49, 17), (3, 31), (57, 18)];
    let r = read_board(&render(&s), &rect, &t, o);
    for (i, (got, want)) in r.placement.iter().zip(truth.iter()).enumerate() {
        assert!(got == want || *got == '?', "square {i}: {got} vs {want}");
    }
}

#[test]
fn drag_ghost_is_not_read_as_extra_piece() {
    let (sc, rect, t, o) = learn(BROWN, 1, true);
    let fen = placement_from_fen(MID_FENS[0]);
    let mut s = sc.clone();
    s.placement = fen;
    // lift the knight on f3 (index 45) and hold it between squares
    s.hide = vec![45];
    s.drag = Some(('N', 232.0 + 80.0 * 4.5 + 40.0, 72.0 + 80.0 * 5.0 + 20.0));
    let r = read_board(&render(&s), &rect, &t, o);
    // origin must read empty
    assert_eq!(r.placement[45], '.');
}

#[test]
fn learn_rejects_non_start_position() {
    let mut sc = Scene::new(placement_from_fen(MID_FENS[0]));
    sc.theme = GREEN;
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::ChessCom).unwrap();
    assert!(learn_from_start(&frame, &rect).is_err());
}

#[test]
fn templates_survive_json_round_trip() {
    let (sc, rect, t, o) = learn(BLUE, 1, true);
    let json = serde_json::to_string(&t).unwrap();
    let t2: Templates = serde_json::from_str(&json).unwrap();
    let truth = placement_from_fen(MID_FENS[1]);
    let mut s = sc.clone();
    s.placement = truth;
    assert_reads(&render(&s), &rect, &t2, o, &truth, "json");
    let empty = read_board(&render(&s), &rect, &Templates::default(), o);
    assert_eq!(empty.confidence, 0.0);
}

// ---------------------------------------------------------------------------
// tracker
// ---------------------------------------------------------------------------

use crate::tracker::*;
use shakmaty::{CastlingMode, Chess, EnPassantMode, Position, fen::Fen, uci::UciMove};

/// 22 plies: double pawn captures, promotion with capture, en passant, both castles.
const GAME: &[&str] = &[
    "a2a4", "b7b5", "a4b5", "a7a6", "b5a6", "d7d6", "a6a7", "e7e6", "a7b8q", "a8b8", "e2e4",
    "g8h6", "e4e5", "f7f5", "e5f6", "g7f6", "g1f3", "d8d7", "f1e2", "f8g7", "e1g1", "e8g8",
];

fn fen_string(pos: &Chess) -> String {
    Fen::from_position(pos, EnPassantMode::Legal).to_string()
}

fn reading_of(pos: &Chess) -> Reading {
    let mut placement = ['.'; 64];
    let fen = fen_string(pos);
    placement.copy_from_slice(&placement_from_fen(&fen));
    Reading {
        placement,
        orientation: Orientation::White,
        confidence: 1.0,
        mover_hint: None,
        last_move_squares: None,
    }
}

fn game_positions() -> Vec<(Chess, String)> {
    let mut pos = Chess::default();
    let mut out = vec![(pos.clone(), String::new())];
    for m in GAME {
        let mv = m
            .parse::<UciMove>()
            .unwrap()
            .to_move(&pos)
            .unwrap_or_else(|e| panic!("illegal {m}: {e}"));
        pos.play_unchecked(mv);
        out.push((pos.clone(), m.to_string()));
    }
    out
}

fn feed(t: &mut PositionTracker, r: &Reading, n: usize) -> Tracked {
    let mut last = None;
    for _ in 0..n {
        last = Some(t.update(r));
    }
    last.unwrap()
}

#[test]
fn tracks_scripted_game_with_special_moves() {
    let positions = game_positions();
    let mut t = PositionTracker::new();
    assert_eq!(
        t.update(&reading_of(&positions[0].0)).state,
        TrackState::NeedsCalibration
    );
    t.reset_to_start();
    let mut prev = reading_of(&positions[0].0);
    for (ply, (pos, mv)) in positions.iter().enumerate().skip(1) {
        let r = reading_of(pos);
        // First frame of a new position is not trusted yet.
        let first = t.update(&r);
        assert_ne!(
            first.last_move.as_deref(),
            Some(mv.as_str()),
            "ply {ply} accepted after one frame"
        );
        let out = t.update(&r);
        assert_eq!(out.state, TrackState::Tracking);
        assert_eq!(
            out.fen.as_deref(),
            Some(fen_string(pos).as_str()),
            "ply {ply} {mv}"
        );
        assert_eq!(out.last_move.as_deref(), Some(mv.as_str()));
        prev = r;
    }
    let _ = prev;
    // Castling rights are gone and the en-passant square is only reported when legal.
    let fen = fen_string(&positions[22].0);
    assert!(fen.contains(" w - - "), "{fen}");
}

#[test]
fn survives_mid_drag_and_unknown_squares() {
    let positions = game_positions();
    let mut t = PositionTracker::new();
    t.reset_to_start();
    for (pos, _) in positions.iter().take(11).skip(1) {
        feed(&mut t, &reading_of(pos), 2);
    }
    let before = t.update(&reading_of(&positions[10].0));
    // The e-pawn is lifted and held for many frames: still the same position.
    let mut lifted = reading_of(&positions[10].0);
    lifted.placement[52] = '.'; // e2 on the board (pawn still on e2 at ply 10)
    for _ in 0..30 {
        let o = t.update(&lifted);
        assert_eq!(o.state, TrackState::Tracking);
        assert_eq!(o.fen, before.fen);
    }
    // Dragged piece drawn over another square.
    let mut ghost = lifted.clone();
    ghost.placement[13] = 'P';
    for _ in 0..30 {
        assert_eq!(t.update(&ghost).fen, before.fen);
    }
    // Move completes, destination square hidden by an arrow head ('?').
    let mut hidden = reading_of(&positions[11].0);
    hidden.placement[36] = '?';
    let o = feed(&mut t, &hidden, 2);
    assert_eq!(o.last_move.as_deref(), Some("e2e4"));
    // Low confidence frames are ignored.
    let mut junk = reading_of(&positions[5].0);
    junk.confidence = 0.2;
    assert_eq!(feed(&mut t, &junk, 5).last_move.as_deref(), Some("e2e4"));
}

fn play(pos: &mut Chess, ucis: &[&str]) {
    for m in ucis {
        let mv = m.parse::<UciMove>().unwrap().to_move(pos).unwrap();
        pos.play_unchecked(mv);
    }
}

/// Feed a board that is physically shown from black's side, like the session does: every frame
/// is read with the orientation the tracker reported last.
fn feed_black_side(
    t: &mut PositionTracker,
    pos: &Chess,
    n: usize,
    mut orient: Orientation,
) -> Tracked {
    let mut last = None;
    for _ in 0..n {
        let mut r = reading_of(pos);
        if orient == Orientation::White {
            let src = r.placement;
            for (i, c) in src.iter().enumerate() {
                r.placement[63 - i] = *c;
            }
        }
        r.orientation = orient;
        let o = t.update(&r);
        orient = o.orientation;
        last = Some(o);
    }
    last.unwrap()
}

#[test]
fn flipped_board_is_detected_instead_of_adopting_garbage() {
    let mut t = PositionTracker::new();
    t.reset_to_start();
    let mut pos = Chess::default();
    for m in ["e2e4", "e7e5", "g1f3"] {
        play(&mut pos, &[m]);
        feed(&mut t, &reading_of(&pos), 3);
    }
    // New game as black: chess.com shows the start position from the other side.
    let start = Chess::default();
    let o = feed_black_side(&mut t, &start, 12, Orientation::White);
    assert_eq!(o.state, TrackState::Tracking);
    assert_eq!(o.orientation, Orientation::Black);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&start).as_str()));
    // And moves keep tracking from the flipped side.
    let mut pos = Chess::default();
    play(&mut pos, &["e2e4"]);
    let o = feed_black_side(&mut t, &pos, 3, Orientation::Black);
    assert_eq!(o.orientation, Orientation::Black);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&pos).as_str()));
    // An uncalibrated tracker also recognises the rotated start.
    let mut t = PositionTracker::new();
    let o = feed_black_side(&mut t, &start, 3, Orientation::White);
    assert_eq!(o.orientation, Orientation::Black);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&start).as_str()));
}

#[test]
fn flipped_board_mid_game_is_explained_by_legal_play() {
    let mut t = PositionTracker::new();
    t.reset_to_start();
    let mut pos = Chess::default();
    play(&mut pos, &["e2e4", "e7e5"]);
    feed(&mut t, &reading_of(&pos), 3);
    play(&mut pos, &["g1f3"]);
    let o = feed_black_side(&mut t, &pos, 3, Orientation::White);
    assert_eq!(o.orientation, Orientation::Black);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&pos).as_str()));
}

#[test]
fn unexplained_piece_in_hand_look_alike_does_not_stick_forever() {
    // Joined mid-game, then the user steps to a position the history cannot explain.
    let mut pos = Chess::default();
    play(&mut pos, &["e2e4", "e7e5"]);
    let mut t = PositionTracker::new();
    feed(&mut t, &reading_of(&pos), 3);
    let mut target = Chess::default();
    play(&mut target, &["e2e4"]);
    let o = feed(&mut t, &reading_of(&target), 200);
    assert_eq!(o.state, TrackState::Tracking);
    // The side to move cannot be known from a bare placement; the placement itself must follow.
    let board = |f: &str| f.split(' ').next().unwrap().to_owned();
    assert_eq!(
        board(o.fen.as_deref().unwrap()),
        board(&fen_string(&target))
    );
}

#[test]
fn joining_with_black_to_move_is_corrected_by_the_first_black_move() {
    let mut pos = Chess::default();
    play(&mut pos, &["e2e4"]);
    let mut t = PositionTracker::new();
    let o = feed(&mut t, &reading_of(&pos), 3);
    assert_eq!(o.state, TrackState::Tracking);
    play(&mut pos, &["d7d5"]);
    let o = feed(&mut t, &reading_of(&pos), 3);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&pos).as_str()));
    assert_eq!(o.last_move.as_deref(), Some("d7d5"));
    play(&mut pos, &["d2d4"]);
    let o = feed(&mut t, &reading_of(&pos), 3);
    assert_eq!(o.fen.as_deref(), Some(fen_string(&pos).as_str()));
}

#[test]
fn hidden_promotion_square_defaults_to_queen_and_under_promotion_is_read() {
    let mut t = PositionTracker::new();
    t.reset_to_fen("1n5k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap();
    let mut after = Chess::default();
    after = "1n5k/P7/8/8/8/8/8/K7 w - - 0 1"
        .parse::<Fen>()
        .unwrap()
        .into_position(CastlingMode::Standard)
        .unwrap_or(after);
    let mv = "a7b8q".parse::<UciMove>().unwrap().to_move(&after).unwrap();
    let mut q = after.clone();
    q.play_unchecked(mv);
    let mut r = reading_of(&q);
    r.placement[1] = '?';
    assert_eq!(feed(&mut t, &r, 2).last_move.as_deref(), Some("a7b8q"));
    // Explicit knight on screen.
    let mut t = PositionTracker::new();
    t.reset_to_fen("1n5k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap();
    let mut n = after.clone();
    n.play_unchecked("a7b8n".parse::<UciMove>().unwrap().to_move(&after).unwrap());
    assert_eq!(
        feed(&mut t, &reading_of(&n), 2).last_move.as_deref(),
        Some("a7b8n")
    );
}

#[test]
fn takeback_navigation_and_new_game() {
    let positions = game_positions();
    let mut t = PositionTracker::new();
    t.reset_to_start();
    for (pos, _) in positions.iter().take(15).skip(1) {
        feed(&mut t, &reading_of(pos), 2);
    }
    // Takeback by two plies.
    let o = feed(&mut t, &reading_of(&positions[12].0), 2);
    assert_eq!(
        o.fen.as_deref(),
        Some(fen_string(&positions[12].0).as_str())
    );
    // Step forward again along the remembered line.
    let o = feed(&mut t, &reading_of(&positions[14].0), 2);
    assert_eq!(
        o.fen.as_deref(),
        Some(fen_string(&positions[14].0).as_str())
    );
    // Back to ply 10, then a different move than the game.
    feed(&mut t, &reading_of(&positions[10].0), 2);
    let mut pos = positions[10].0.clone();
    pos.play_unchecked("g1f3".parse::<UciMove>().unwrap().to_move(&pos).unwrap());
    let o = feed(&mut t, &reading_of(&pos), 2);
    assert_eq!(o.last_move.as_deref(), Some("g1f3"));
    assert_eq!(o.fen.as_deref(), Some(fen_string(&pos).as_str()));
    // New game: placement returns to the start position.
    let o = feed(&mut t, &reading_of(&Chess::default()), 2);
    assert_eq!(
        o.fen.as_deref(),
        Some("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
    );
    assert_eq!(o.last_move, None);
}

#[test]
fn resyncs_when_the_board_jumps() {
    let positions = game_positions();
    let mut t = PositionTracker::new();
    t.reset_to_start();
    for (pos, _) in positions.iter().take(6).skip(1) {
        feed(&mut t, &reading_of(pos), 2);
    }
    // A completely different position (e.g. the user loaded another game).
    let other: Chess = "r3k2r/ppp2ppp/2n5/8/8/2N5/PPP2PPP/R3K2R b KQkq - 3 12"
        .parse::<Fen>()
        .unwrap()
        .into_position(CastlingMode::Standard)
        .unwrap();
    let r = reading_of(&other);
    let mut saw_lost = false;
    let mut last = None;
    for _ in 0..(RESYNC_FRAMES + 6) {
        let o = t.update(&r);
        saw_lost |= o.state == TrackState::Lost;
        last = Some(o);
    }
    assert!(saw_lost, "Lost state must be reported while unexplained");
    let o = last.unwrap();
    assert_eq!(o.state, TrackState::Tracking);
    let fen = o.fen.unwrap();
    assert!(
        fen.starts_with("r3k2r/ppp2ppp/2n5/8/8/2N5/PPP2PPP/R3K2R "),
        "{fen}"
    );
    // Castling rights are inferred from the home squares.
    assert!(fen.contains("KQkq"), "{fen}");
    // The adopted position is playable: castle.
    let mut pos = other.clone();
    let castle = pos
        .legal_moves()
        .into_iter()
        .find(|m| m.to_uci(CastlingMode::Standard).to_string() == "e8g8")
        .expect("legal castle");
    pos.play_unchecked(castle);
    let o = feed(&mut t, &reading_of(&pos), 2);
    assert_eq!(o.last_move.as_deref(), Some("e8g8"));
}

#[test]
fn joins_a_running_game_without_calibration() {
    let positions = game_positions();
    let mut t = PositionTracker::new();
    let o = feed(&mut t, &reading_of(&positions[9].0), 2);
    assert_eq!(o.state, TrackState::Tracking);
    assert!(o.fen.unwrap().starts_with("rQbqkbnr/"));
}

// ---------------------------------------------------------------------------
// end to end: pixels -> detection -> reading -> tracker
// ---------------------------------------------------------------------------

#[test]
fn pixel_pipeline_follows_a_game_in_every_theme() {
    let positions = game_positions();
    for (name, theme, k, white_bottom) in [
        ("green", GREEN, 1, true),
        ("brown", BROWN, 2, false),
        ("blue", BLUE, 1, false),
    ] {
        let (sc, rect, templates, orient) = learn(theme, k, white_bottom);
        let rec = Recognizer::new(&templates).unwrap();
        let mut t = PositionTracker::new();
        t.reset_to_start();
        let mut last_uci: Option<String> = None;
        for (ply, (pos, mv)) in positions.iter().enumerate().skip(1) {
            let mut s = sc.clone();
            s.placement = placement_from_fen(&fen_string(pos));
            s.seed = 1000 + ply as u64;
            // last move highlight + a Fishy-style arrow + dots for the side to move
            let m = shakmaty::uci::UciMove::from_ascii(mv.as_bytes()).unwrap();
            if let shakmaty::uci::UciMove::Normal { from, to, .. } = m {
                let idx = |sq: shakmaty::Square| (7 - sq.rank() as usize) * 8 + sq.file() as usize;
                s.highlights = vec![idx(from), idx(to)];
                last_uci = Some(mv.clone());
            }
            s.arrows = vec![(60, 45)];
            s.dots = vec![20, 28];
            let frame = render(&s);
            let reading = rec.read(&frame, &rect, orient);
            let _ = feed(&mut t, &reading, 1);
            let out = t.update(&reading);
            assert_eq!(out.state, TrackState::Tracking, "{name} ply {ply}");
            assert_eq!(
                out.fen.as_deref(),
                Some(fen_string(pos).as_str()),
                "{name} ply {ply} {mv}"
            );
        }
        assert!(last_uci.is_some());
    }
}

#[test]
fn detection_tolerates_heavy_noise_and_reading_is_fast() {
    let mut sc = Scene::new(start_placement());
    sc.noise = 12;
    sc.theme = BROWN;
    let frame = render(&sc);
    let r = detect_board(&frame, Site::Lichess).expect("board under heavy noise");
    check_rect(r, &sc, 1.5);
    let (lsc, rect, t, o) = learn(GREEN, 1, true);
    let mut s = lsc.clone();
    s.placement = placement_from_fen(MID_FENS[1]);
    let frame = render(&s);
    let rec = Recognizer::new(&t).unwrap();
    let started = Instant::now();
    for _ in 0..10 {
        let _ = rec.read(&frame, &rect, o);
    }
    let ms = started.elapsed().as_secs_f64() * 100.0;
    eprintln!("read_board {ms:.2} ms");
    if !cfg!(debug_assertions) {
        assert!(ms < 60.0, "read_board too slow: {ms} ms");
    }
}

// ---------------------------------------------------------------------------
// last-move highlight -> mover hint
// ---------------------------------------------------------------------------

fn hint_for(
    theme: Theme,
    white_bottom: bool,
    fen: &str,
    highlights: &[usize],
    dots: &[usize],
) -> Reading {
    let (sc, rect, t, o) = learn(theme, 1, white_bottom);
    let mut s = sc.clone();
    s.placement = placement_from_fen(fen);
    s.highlights = highlights.to_vec();
    s.dots = dots.to_vec();
    read_board(&render(&s), &rect, &t, o)
}

#[test]
fn mover_hint_comes_from_a_highlighted_pair_not_from_a_lone_selected_piece() {
    let after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR";
    let after_e4_e5 = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR";
    let after_e4_e6 = "rnbqkbnr/pppp1ppp/4p3/8/4P3/8/PPPP1PPP/RNBQKBNR";
    for (name, theme) in [("green", GREEN), ("brown", BROWN), ("blue", BLUE)] {
        for wb in [true, false] {
            let what = format!("{name} white_bottom={wb}");
            // e2-e4 (e2 empty, e4 white pawn): white moved.
            let r = hint_for(theme, wb, after_e4, &[52, 36], &[]);
            assert_eq!(
                (r.mover_hint, r.last_move_squares),
                (Some('w'), Some((52, 36))),
                "{what}"
            );
            // e7-e5: black moved.
            let r = hint_for(theme, wb, after_e4_e5, &[12, 28], &[]);
            assert_eq!(
                (r.mover_hint, r.last_move_squares),
                (Some('b'), Some((12, 28))),
                "{what}"
            );
            // A lone highlighted piece is a selection: no hint (dots on its targets change nothing).
            let r = hint_for(theme, wb, after_e4, &[62], &[45, 47]);
            assert_eq!(
                (r.mover_hint, r.last_move_squares),
                (None, None),
                "{what} lone"
            );
            // No highlight at all.
            assert_eq!(
                hint_for(theme, wb, after_e4, &[], &[]).mover_hint,
                None,
                "{what} none"
            );
            // The pair plus a selected piece elsewhere (a queen on d1 that cannot have come from e7).
            let r = hint_for(theme, wb, after_e4_e6, &[12, 20, 59], &[51, 52, 45]);
            assert_eq!(
                (r.mover_hint, r.last_move_squares),
                (Some('b'), Some((12, 20))),
                "{what} + selected"
            );
            // Two occupied or two empty tinted squares are not a move.
            assert_eq!(
                hint_for(theme, wb, after_e4, &[62, 63], &[]).mover_hint,
                None,
                "{what} two pieces"
            );
            assert_eq!(
                hint_for(theme, wb, after_e4, &[20, 21], &[]).mover_hint,
                None,
                "{what} two empty"
            );
        }
    }
}

#[test]
fn tracker_uses_the_mover_hint_when_joining_a_game() {
    // Black just played ...e6: white to move.
    let mid = placement_from_fen("rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR");
    for (hint, side) in [(Some('b'), "w"), (Some('w'), "b"), (None, "w")] {
        let mut t = PositionTracker::new();
        let r = Reading {
            placement: mid,
            orientation: Orientation::White,
            confidence: 1.0,
            mover_hint: hint,
            last_move_squares: None,
        };
        assert_eq!(t.update(&r).state, TrackState::NeedsCalibration);
        let out = t.update(&r);
        assert_eq!(out.state, TrackState::Tracking, "{hint:?}");
        let fen = out.fen.unwrap();
        assert_eq!(fen.split(' ').nth(1), Some(side), "{hint:?}: {fen}");
    }
}

// ---------------------------------------------------------------------------
// mid-game calibration
// ---------------------------------------------------------------------------

use crate::{learn_from_placement_auto, learned_kinds, merge_templates, missing_kind_names, placement_is_legal};

const THEMES: [(&str, Theme); 4] = [
    ("green", GREEN),
    ("brown", BROWN),
    ("blue", BLUE),
    ("custom", CUSTOM),
];

fn scene_of(fen: &str, theme: Theme, white_bottom: bool) -> Scene {
    let mut sc = Scene::new(placement_from_fen(fen));
    sc.theme = theme;
    sc.white_bottom = white_bottom;
    sc
}

#[test]
fn auto_learning_finds_orientation_and_reads_back() {
    for (name, theme) in THEMES {
        for white_bottom in [true, false] {
            for fen in [MID_FENS[0], MID_FENS[1], MID_FENS[2]] {
                let sc = scene_of(fen, theme, white_bottom);
                let frame = render(&sc);
                let rect = detect_board(&frame, Site::Generic).expect("board");
                let truth = placement_from_fen(fen);
                let a = learn_from_placement_auto(&frame, &rect, &truth, None)
                    .unwrap_or_else(|e| panic!("{name} white_bottom={white_bottom} {fen}: {e}"));
                let want = if white_bottom {
                    Orientation::White
                } else {
                    Orientation::Black
                };
                assert_eq!(a.orientation, want, "{name} {fen}");
                assert_reads(&frame, &rect, &a.templates, want, &truth, name);
                // With a hint the answer is the same.
                let h = learn_from_placement_auto(&frame, &rect, &truth, Some(want)).unwrap();
                assert_eq!(h.orientation, want);
            }
        }
    }
}

#[test]
fn auto_learning_rejects_a_wrong_fen_and_tolerates_arrows() {
    let fen = MID_FENS[0];
    let mut sc = scene_of(fen, BROWN, true);
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::Generic).unwrap();
    // Two pieces moved to empty squares: occupancy no longer fits.
    let mut bad = placement_from_fen(fen);
    bad.swap(0, 24); // a8 rook <-> empty a5
    bad.swap(7, 31);
    bad.swap(4, 28);
    let err = learn_from_placement_auto(&frame, &rect, &bad, None).unwrap_err();
    assert!(
        err.to_string().contains("pieces seen") || err.to_string().contains("do not reproduce"),
        "unexpected error: {err}"
    );
    // Arrows over two pieces are fine.
    sc.arrows = vec![(62, 45), (57, 42)];
    let frame = render(&sc);
    let truth = placement_from_fen(fen);
    let a = learn_from_placement_auto(&frame, &rect, &truth, None).expect("arrows tolerated");
    assert_eq!(a.orientation, Orientation::White);
}

#[test]
fn auto_learning_a_symmetric_occupancy_uses_colours_or_refuses() {
    // Occupancy is identical upside down: only the piece colours tell the sides apart.
    let fen = "8/2p5/3k4/8/8/4K3/5P2/8";
    for white_bottom in [true, false] {
        let sc = scene_of(fen, GREEN, white_bottom);
        let frame = render(&sc);
        let rect = detect_board(&frame, Site::Generic).expect("board");
        let truth = placement_from_fen(fen);
        let want = if white_bottom {
            Orientation::White
        } else {
            Orientation::Black
        };
        match learn_from_placement_auto(&frame, &rect, &truth, None) {
            Ok(a) => assert_eq!(a.orientation, want, "never the wrong side"),
            Err(e) => {
                let m = e.to_string();
                assert!(
                    m.contains("which side") || m.contains("piece colours"),
                    "{m}"
                );
            }
        }
    }
}

#[test]
fn learned_kinds_masks_missing_pieces() {
    // No queens and no black knights.
    let fen = "r1b1kb1r/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/R1B1KB1R";
    let sc = scene_of(fen, GREEN, true);
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::Generic).unwrap();
    let a = learn_from_placement_auto(&frame, &rect, &placement_from_fen(fen), None).unwrap();
    // P N B R Q K p n b r q k
    assert_eq!(
        a.kinds_seen,
        [
            true, false, true, true, false, true, true, false, true, true, false, true
        ]
    );
    assert_eq!(learned_kinds(&a.templates), a.kinds_seen);
    assert_eq!(missing_kind_names(&a.kinds_seen), ["knight", "queen"]);
    assert_eq!(learned_kinds(&Templates::default()), [false; 12]);
}

#[test]
fn placement_legality() {
    let p = |fen: &str| placement_from_fen(fen);
    assert!(placement_is_legal(&p(MID_FENS[0])));
    assert!(placement_is_legal(&p(MID_FENS[3])));
    // no black king
    assert!(!placement_is_legal(&placement_from_fen("8/8/8/8/8/8/8/4K3")));
    // nine white pawns
    assert!(!placement_is_legal(&placement_from_fen(
        "k7/8/8/8/8/PPPPPPPP/P7/K7"
    )));
    // pawn on the last rank
    assert!(!placement_is_legal(&placement_from_fen("k6P/8/8/8/8/8/8/K7")));
    // both kings attacked: black rook gives check to white, white rook gives check to black
    assert!(!placement_is_legal(&placement_from_fen("k3R3/8/8/8/8/8/8/K3r3")));
}

#[test]
fn merging_adds_the_missing_kinds_and_refuses_foreign_sets() {
    // A has no queens; B shows them. Same theme: merged reads both.
    let fen_a = "r1b1kb1r/pppp1ppp/2n2n2/4p3/4P3/2N2N2/PPPP1PPP/R1B1KB1R";
    let learn_in = |fen: &str, theme: Theme| {
        let frame = render(&scene_of(fen, theme, true));
        let rect = detect_board(&frame, Site::Generic).unwrap();
        let a = learn_from_placement_auto(&frame, &rect, &placement_from_fen(fen), None).unwrap();
        (frame, rect, a)
    };
    let (_, _, a) = learn_in(fen_a, GREEN);
    assert!(!a.kinds_seen[4] && !a.kinds_seen[10]);
    // B contains queens but, learned alone from a position with few pieces, misses rooks.
    let fen_b = "3qk3/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/3QK3";
    let (frame_b, rect_b, b) = learn_in(fen_b, GREEN);
    assert!(b.kinds_seen[4] && b.kinds_seen[10] && !b.kinds_seen[3]);
    let truth_b = placement_from_fen(fen_b);
    let out = merge_templates(
        &a.templates,
        &b.templates,
        &frame_b,
        &rect_b,
        &truth_b,
        Orientation::White,
    );
    assert!(out.merged, "same theme merges");
    let k = learned_kinds(&out.templates);
    assert!(k[3], "rooks carried over");
    assert!(k[4] && k[10], "queens kept");
    for fen in [fen_a, fen_b, MID_FENS[0]] {
        let truth = placement_from_fen(fen);
        assert_reads(
            &render(&scene_of(fen, GREEN, true)),
            &rect_b,
            &out.templates,
            Orientation::White,
            &truth,
            "merged",
        );
    }
    // A different piece set (other paint colours) is not mixed in.
    let (frame_c, rect_c, c) = learn_in(fen_b, BLUE);
    let mut foreign = c.templates.clone();
    for col in foreign.data["palette"].as_array_mut().unwrap() {
        for ch in col[0].as_array_mut().unwrap() {
            *ch = serde_json::json!((ch.as_u64().unwrap() as u8).wrapping_add(90));
        }
    }
    let out = merge_templates(
        &a.templates,
        &foreign,
        &frame_c,
        &rect_c,
        &truth_b,
        Orientation::White,
    );
    assert!(!out.merged);
    assert_eq!(out.templates.data, foreign.data);
}
