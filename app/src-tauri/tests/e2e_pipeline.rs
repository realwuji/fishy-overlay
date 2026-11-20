//! Headless end-to-end check of the real pipeline, with no GUI and no screen capture:
//! synthetic chess.com-style frame -> detect -> learn -> read -> tracker -> real Stockfish -> build_frame.
//!
//! Skips (with a message) when the Stockfish sidecar has not been built.

use fishy_engine::{Engine, EngineConfig, EngineUpdate, build_frame};
use fishy_vision::recognize::Recognizer;
use fishy_vision::test_support::{Scene, placement_from_fen, render, start_placement};
use fishy_vision::{PositionTracker, Site, TrackState, detect_board, learn_from_start};
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Position};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

fn find_stockfish() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("stockfish-") && !n.ends_with(".nnue"))
        })
}

fn legal(fen: &str, uci: &str) -> bool {
    let setup: Fen = fen.parse().expect("fen");
    let pos: Chess = setup
        .into_position(CastlingMode::Standard)
        .expect("position");
    uci.parse::<UciMove>()
        .ok()
        .and_then(|m| m.to_move(&pos).ok())
        .is_some_and(|m| pos.is_legal(m))
}

#[test]
fn synthetic_frame_to_analysis_frame() {
    // 1. A start-position screenshot with a last-move-free board, as the user would calibrate on.
    let sc = Scene::new(start_placement());
    let frame = render(&sc);
    let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
    let (templates, orientation) = learn_from_start(&frame, &rect).expect("calibration");

    // 2. Read the board and follow it with the tracker (it wants two identical frames).
    let rec = Recognizer::new(&templates).expect("templates");
    let mut tracker = PositionTracker::new();
    tracker.reset_to_start();
    let reading = rec.read(&frame, &rect, orientation);
    tracker.update(&reading);
    let tracked = tracker.update(&reading);
    assert_eq!(tracked.state, TrackState::Tracking);

    // 3. The same board after 1.e4, with a highlight and a stray arrow like a real page has.
    let after = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
    let mut sc2 = Scene::new(placement_from_fen(after));
    sc2.highlights = vec![52, 36];
    sc2.arrows = vec![(57, 42)];
    let frame2 = render(&sc2);
    let reading2 = rec.read(&frame2, &rect, orientation);
    tracker.update(&reading2);
    let tracked = tracker.update(&reading2);
    assert_eq!(tracked.state, TrackState::Tracking);
    assert_eq!(tracked.last_move.as_deref(), Some("e2e4"));
    let fen = tracked.fen.expect("fen");
    assert!(
        fen.starts_with("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b "),
        "{fen}"
    );

    // 4. Real Stockfish.
    let Some(bin) = find_stockfish() else {
        eprintln!(
            "SKIP engine half: no Stockfish in src-tauri/binaries (run scripts/build-stockfish.sh)"
        );
        return;
    };
    let (tx, rx) = mpsc::channel::<EngineUpdate>();
    let engine = Engine::spawn(
        &bin,
        EngineConfig {
            multi_pv: 3,
            depth_limit: Some(12),
            threads: 1,
            hash_mb: 16,
            ..EngineConfig::default()
        },
        move |u| {
            let _ = tx.send(u);
        },
    )
    .expect("engine starts");
    engine.analyze(&fen).expect("analyze");
    let done = loop {
        let u = rx
            .recv_timeout(Duration::from_secs(60))
            .expect("engine answers");
        if u.done {
            break u;
        }
    };

    // 5. Classified frame.
    let out = build_frame(&done, 42).expect("frame");
    assert_eq!(out.side_to_move, "b");
    assert_eq!(out.fen, fen);
    assert_eq!(out.candidates.len(), 3, "three MultiPV candidates");
    for (i, c) in out.candidates.iter().enumerate() {
        assert_eq!(c.rank as usize, i + 1);
        assert!(legal(&fen, &c.uci), "illegal candidate {}", c.uci);
        assert!(!c.san.is_empty() && !c.pv_san.is_empty());
        assert!(c.loss_pct >= 0.0 && (0.0..=100.0).contains(&c.win_pct));
        assert!(c.explanation.is_some(), "every candidate is explained");
    }
    assert_eq!(out.candidates[0].loss_pct, 0.0);
    assert!(
        matches!(
            out.candidates[0].cls,
            fishy_engine::MoveClass::Best
                | fishy_engine::MoveClass::Great
                | fishy_engine::MoveClass::Brilliant
        ),
        "the engine's first choice is graded best: {:?}",
        out.candidates[0].cls
    );
    assert!(
        out.eval_cp.is_some_and(|cp| cp.abs() < 150),
        "{:?}",
        out.eval_cp
    );
    assert_eq!(out.ts, 42);
}
