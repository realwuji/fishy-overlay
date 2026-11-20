//! Real pipeline on the user's own chess.com screenshot of a game in progress (no calibration):
//! detect -> built-in templates -> read -> tracker -> FEN -> real Stockfish -> AnalysisFrame.
//!
//! The screenshot is third-party artwork kept in the git-ignored `local/` fixtures dir of
//! fishy-vision; the test skips (notice + pass) when it or the Stockfish sidecar is absent.

use fishy_engine::{Engine, EngineConfig, EngineUpdate, build_frame};
use fishy_vision::recognize::Recognizer;
use fishy_vision::{
    Frame, Orientation, PositionTracker, Site, Templates, TrackState, detect_board,
};
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Position};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

const PLACEMENT: &str = "rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR";

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

fn legal_move(fen: &str, uci: &str) -> bool {
    let setup: Fen = fen.parse().expect("fen");
    let pos: Chess = setup
        .into_position(CastlingMode::Standard)
        .expect("position");
    uci.parse::<UciMove>()
        .ok()
        .and_then(|m| m.to_move(&pos).ok())
        .is_some_and(|m| pos.is_legal(m))
}

fn square_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && (b'a'..=b'h').contains(&b[0]) && (b'1'..=b'8').contains(&b[1])
}

#[test]
fn midgame_screenshot_to_arrows_without_calibration() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/fishy-vision/tests/fixtures/local/chesscom-midgame.png");
    if !path.exists() {
        eprintln!(
            "SKIP: {} is absent (third-party screenshot, not committed)",
            path.display()
        );
        return;
    }
    let img = image::open(&path).expect("readable png").to_rgba8();
    let frame = Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    };

    // Vision: no saved calibration, only the bundled chess.com pieces.
    let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
    let templates = Templates::builtin(Site::ChessCom).expect("built-in chess.com templates");
    let rec = Recognizer::new(&templates).expect("templates parse");
    let reading = rec.read(&frame, &rect, Orientation::White);
    let mut tracker = PositionTracker::new();
    let mut tracked = tracker.update(&reading);
    for _ in 0..3 {
        tracked = tracker.update(&reading);
    }
    assert_eq!(tracked.state, TrackState::Tracking, "{tracked:?}");
    let fen = tracked.fen.expect("fen");
    let mut parts = fen.split(' ');
    assert_eq!(parts.next(), Some(PLACEMENT), "{fen}");
    assert_eq!(parts.next(), Some("w"), "white to move: {fen}");

    // Engine.
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
    let out = build_frame(&done, 1).expect("analysis frame");
    assert_eq!(out.side_to_move, "w");
    assert!(out.fen.starts_with(PLACEMENT), "{}", out.fen);
    assert_eq!(out.candidates.len(), 3);
    for c in &out.candidates {
        assert!(legal_move(&fen, &c.uci), "illegal candidate {}", c.uci);
        assert!(
            square_ok(&c.uci[0..2]) && square_ok(&c.uci[2..4]),
            "arrow needs from/to squares: {}",
            c.uci
        );
    }
}

/// The user plays black: the board is flipped (squares rotated, pieces upright, coordinates
/// reversed). The session path (`BoardPipeline`) must read the same position with Black at the
/// bottom and must never publish the rotated one the old White default produced.
#[test]
fn flipped_midgame_screenshot_is_read_as_black_without_calibration() {
    use fishy_vision::test_support::{flipped_view, frame_of_image};
    use fishy_vision::{BoardPipeline, OrientationMode};

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/fishy-vision/tests/fixtures/local/chesscom-midgame.png");
    if !path.exists() {
        eprintln!(
            "SKIP: {} is absent (third-party screenshot, not committed)",
            path.display()
        );
        return;
    }
    let img = image::open(&path).expect("readable png").to_rgba8();
    let rect = detect_board(&frame_of_image(&img), Site::ChessCom).expect("board detected");
    let flipped = flipped_view(&img, &rect, true);
    let frame = frame_of_image(&flipped);
    let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
    let rec = Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap();

    // The old default path is confidently wrong...
    let wrong = rec.read(&frame, &rect, Orientation::White);
    assert!(
        wrong
            .placement
            .iter()
            .collect::<String>()
            .starts_with("RNBKQBNR"),
        "the white-assuming read is the rotated position"
    );

    // ...the session path is not.
    let mut pipe = BoardPipeline::new(OrientationMode::Auto);
    let mut last = None;
    for i in 0..4 {
        let out = pipe.process(&rec, &frame, &rect, Site::ChessCom, i == 0);
        if let Some(f) = &out.tracked.fen {
            assert!(f.starts_with(PLACEMENT), "frame {i}: {f}");
        }
        last = Some(out);
    }
    let out = last.unwrap();
    assert_eq!(out.orientation, Some(Orientation::Black));
    assert_eq!(out.tracked.state, TrackState::Tracking);
    let fen = out.tracked.fen.expect("fen");
    let mut parts = fen.split(' ');
    assert_eq!(parts.next(), Some(PLACEMENT), "{fen}");
    assert_eq!(parts.next(), Some("w"), "{fen}");

    // Manual override: black is final even where nothing could be detected.
    let mut pipe = BoardPipeline::new(OrientationMode::Black);
    let mut out = None;
    for i in 0..3 {
        out = Some(pipe.process(&rec, &frame, &rect, Site::ChessCom, i == 0));
    }
    assert!(out.unwrap().tracked.fen.unwrap().starts_with(PLACEMENT));
}

/// The session's engine path on the real screenshot: the machine-chosen (auto) configuration,
/// warm-up, pacing of frames (target depth, throttle) and the content of every frame. Prints
/// the time to the first frame and to depth 12 on this machine.
#[test]
fn midgame_auto_config_frames_are_fast_and_sane() {
    use app_lib::progress::{MIN_FRAME_GAP, PositionPacer};
    use fishy_engine::{Hardware, recommended_config};
    use std::time::Instant;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/fishy-vision/tests/fixtures/local/chesscom-midgame.png");
    if !path.exists() {
        eprintln!("SKIP: {} is absent", path.display());
        return;
    }
    let Some(bin) = find_stockfish() else {
        eprintln!("SKIP: no Stockfish in src-tauri/binaries");
        return;
    };
    let img = image::open(&path).expect("readable png").to_rgba8();
    let frame = Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    };
    let rect = detect_board(&frame, Site::ChessCom).expect("board detected");
    let rec = Recognizer::new(&Templates::builtin(Site::ChessCom).unwrap()).unwrap();
    let reading = rec.read(&frame, &rect, Orientation::White);
    let mut tracker = PositionTracker::new();
    let mut tracked = tracker.update(&reading);
    for _ in 0..3 {
        tracked = tracker.update(&reading);
    }
    let fen = tracked.fen.expect("fen");

    let hw = Hardware::detect();
    let cfg = recommended_config(&hw);
    eprintln!(
        "auto config: {} threads, {} MB hash (hardware: {hw:?})",
        cfg.threads, cfg.hash_mb
    );
    assert!(cfg.threads >= 1 && cfg.hash_mb.is_power_of_two());

    let (tx, rx) = mpsc::channel::<EngineUpdate>();
    let t_spawn = Instant::now();
    let engine = Engine::spawn(&bin, cfg, move |u| {
        let _ = tx.send(u);
    })
    .expect("engine starts");
    let warm = engine.warm_up().expect("warm-up");
    let startup = t_spawn.elapsed();
    let info = engine.info();
    assert!(info.ready);
    assert!(
        info.name
            .as_deref()
            .is_some_and(|n| n.contains("Stockfish"))
    );
    eprintln!(
        "engine ready in {startup:?} (warm-up {warm:?}): {:?} {}T {} MB",
        info.name, info.threads, info.hash_mb
    );

    const MIN_DEPTH: u32 = 12;
    let t0 = Instant::now();
    let mut pacer = PositionPacer::new(MIN_DEPTH, t0);
    engine.analyze(&fen).expect("analyze");

    let mut first_frame = None;
    let mut depth12 = None;
    let mut frames = Vec::new();
    let mut last_sent: Option<Instant> = None;
    let mut last_depth = 0;
    loop {
        let u = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("engine answers");
        let now = Instant::now();
        assert!(u.depth >= last_depth, "depth never goes back");
        last_depth = u.depth;
        pacer.observe(&u);
        let pacing = pacer.pacing(u.depth, u.done, now);
        if pacer.allow(u.done, u.depth, pacing.target_depth, now) {
            if let Some(prev) = last_sent {
                let gap = now.duration_since(prev);
                // Frames after the first are rate-limited unless they cross the target or end.
                assert!(
                    gap >= MIN_FRAME_GAP || u.done || u.depth >= pacing.target_depth,
                    "frame gap {gap:?}"
                );
            }
            pacer.sent(u.done, u.depth, pacing.target_depth, now);
            last_sent = Some(now);
            let mut f = build_frame(&u, frames.len() as u64 + 1).expect("frame");
            f.target_depth = Some(pacing.target_depth);
            first_frame.get_or_insert(t0.elapsed());
            if f.depth >= MIN_DEPTH && depth12.is_none() {
                depth12 = Some(t0.elapsed());
            }
            frames.push(f);
        }
        if depth12.is_some() || u.done {
            break;
        }
    }
    engine.stop();
    let first_frame = first_frame.expect("a frame arrived");
    let depth12 = depth12.expect("depth 12 reached");
    eprintln!("time to first frame: {first_frame:?}, time to depth 12: {depth12:?}");
    assert!(first_frame < Duration::from_secs(1), "{first_frame:?}");
    assert!(depth12 < Duration::from_millis(2500), "{depth12:?}");

    assert!(!frames.is_empty());
    let mut depth = 0;
    for f in &frames {
        assert!(f.depth >= depth, "depth progression");
        depth = f.depth;
        assert_eq!(f.target_depth, Some(MIN_DEPTH), "target set on every frame");
        let w = f.wdl.expect("wdl present");
        assert_eq!(w.iter().sum::<u32>(), 1000, "wdl sums to per-mille: {w:?}");
        assert!(f.nps > 0);
    }
    let last = frames.last().unwrap();
    assert!(last.depth >= MIN_DEPTH);
    assert_eq!(last.candidates.len(), 3, "MultiPV 3");
    // The standard-ish opening position is balanced: white does not lose, and is not winning big.
    let w = last.wdl.unwrap();
    assert!(
        w[2] < 300 && w[0] < 700,
        "sane wdl for a balanced position: {w:?}"
    );
    for c in &last.candidates {
        assert!(legal_move(&fen, &c.uci), "illegal candidate {}", c.uci);
        assert!(
            square_ok(&c.from) && square_ok(&c.to),
            "from/to: {} {}",
            c.from,
            c.to
        );
        assert_eq!(format!("{}{}", c.from, c.to), c.uci[..4]);
    }
    assert_eq!(last.side_to_move, "w");
}
