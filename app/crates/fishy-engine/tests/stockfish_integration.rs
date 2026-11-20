//! Spawns the real Stockfish sidecar. Skips (with a message) when it has not been built:
//! run `app/scripts/build-stockfish.sh` first, or set `FISHY_STOCKFISH` to a binary.

use fishy_engine::{Engine, EngineConfig, EngineUpdate, MoveClass, TacticTag, build_frame};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const MATE_IN_1: &str = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
const CHECKMATED: &str = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";

fn find_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FISHY_STOCKFISH") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/binaries");
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

fn engine(cfg: EngineConfig) -> Option<(Engine, Receiver<EngineUpdate>)> {
    let Some(bin) = find_binary() else {
        eprintln!(
            "SKIP: no Stockfish binary in app/src-tauri/binaries (run app/scripts/build-stockfish.sh)"
        );
        return None;
    };
    let (tx, rx) = mpsc::channel();
    let e = Engine::spawn(&bin, cfg, move |u| {
        let _ = tx.send(u);
    })
    .expect("engine should start");
    Some((e, rx))
}

fn wait_for(
    rx: &Receiver<EngineUpdate>,
    what: &str,
    mut ok: impl FnMut(&EngineUpdate) -> bool,
) -> EngineUpdate {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let u = rx
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
        if ok(&u) {
            return u;
        }
    }
}

#[test]
fn start_position_returns_multipv_frames() {
    let cfg = EngineConfig {
        multi_pv: 3,
        depth_limit: Some(12),
        threads: 1,
        hash_mb: 16,
        ..EngineConfig::default()
    };
    let Some((e, rx)) = engine(cfg) else { return };
    assert!(e.is_alive());
    e.analyze(START).unwrap();
    let mut seen = Vec::new();
    let done = wait_for(&rx, "the final update", |u| {
        seen.push(u.clone());
        u.done
    });
    assert!(seen.iter().all(|u| u.fen == START));
    assert!(
        seen.iter().all(|u| u.lines.len() == 3),
        "updates are only emitted when every MultiPV line is in"
    );
    assert!(seen.iter().filter(|u| u.done).count() == 1);
    assert!(done.depth >= 12);
    let frame = build_frame(&done, 1).unwrap();
    assert_eq!(frame.candidates.len(), 3);
    assert!(
        frame
            .candidates
            .iter()
            .all(|c| !c.san.is_empty() && !c.pv_san.is_empty())
    );
    assert!(
        frame.eval_cp.is_some_and(|cp| cp.abs() < 150),
        "start position is roughly equal: {:?}",
        frame.eval_cp
    );
    assert!(!frame.thinking);
}

#[test]
fn finds_mate_in_one_and_stale_output_is_dropped() {
    let cfg = EngineConfig {
        multi_pv: 3,
        threads: 1,
        hash_mb: 16,
        ..EngineConfig::default()
    };
    let Some((e, rx)) = engine(cfg) else { return };
    // Start a long (infinite) search, then switch positions mid-flight.
    e.analyze(START).unwrap();
    wait_for(&rx, "first update", |u| u.depth >= 4);
    e.analyze(MATE_IN_1).unwrap();
    // After analyze() returns, nothing for the old position may be delivered.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got_new = false;
    while Instant::now() < deadline {
        if let Ok(u) = rx.recv_timeout(Duration::from_millis(100)) {
            assert_eq!(u.fen, MATE_IN_1, "stale update leaked through");
            got_new |= u.lines.first().is_some_and(|l| l.depth >= 8);
        }
        if got_new {
            break;
        }
    }
    assert!(got_new, "no update for the new position");
    let u = wait_for(&rx, "mate score", |u| {
        u.lines
            .first()
            .is_some_and(|l| matches!(l.score, fishy_engine::Score::Mate(1)))
    });
    let frame = build_frame(&u, 3).unwrap();
    let best = &frame.candidates[0];
    assert_eq!(best.uci, "h5f7");
    assert_eq!(best.mate, Some(1));
    assert!(best.tags.contains(&TacticTag::Mate));
    assert_eq!(
        best.cls,
        MoveClass::Great,
        "mate in one while alternatives only win a little"
    );
    assert_eq!(best.explanation.as_deref(), Some("Delivers checkmate."));
    e.stop();
    wait_for(&rx, "done after stop", |u| u.done);
}

#[test]
fn terminal_positions_do_not_hang_and_multipv_is_clamped() {
    let cfg = EngineConfig {
        multi_pv: 5,
        depth_limit: Some(8),
        threads: 1,
        hash_mb: 16,
        ..EngineConfig::default()
    };
    let Some((e, rx)) = engine(cfg) else { return };
    e.analyze(CHECKMATED).unwrap();
    let u = wait_for(&rx, "terminal update", |u| u.done);
    assert!(u.lines.is_empty());
    // Only one legal reply: MultiPV must clamp to 1 and still finish.
    e.analyze("k7/8/1K6/8/8/8/8/7R b - - 0 1").unwrap();
    let u = wait_for(&rx, "single-move update", |u| u.done);
    assert_eq!(u.lines.len(), 1);
    assert!(e.analyze("not a fen").is_err());
}

#[test]
fn configure_and_quit() {
    let cfg = EngineConfig {
        multi_pv: 2,
        threads: 1,
        hash_mb: 16,
        ..EngineConfig::default()
    };
    let Some((e, rx)) = engine(cfg.clone()) else {
        return;
    };
    e.analyze(START).unwrap();
    wait_for(&rx, "update", |u| u.lines.len() == 2);
    e.configure(EngineConfig {
        multi_pv: 4,
        skill_level: Some(5),
        ..cfg.clone()
    })
    .unwrap();
    wait_for(&rx, "4-line update after configure", |u| u.lines.len() == 4);
    e.configure(EngineConfig {
        multi_pv: 4,
        skill_level: None,
        ..cfg
    })
    .unwrap();
    e.quit();
    assert!(!e.is_alive());
    assert!(e.analyze(START).is_err());
}

#[test]
fn mated_position_after_active_search_still_gets_final_frame() {
    let Some((e, rx)) = engine(EngineConfig {
        threads: 1,
        hash_mb: 16,
        ..EngineConfig::default()
    }) else {
        return;
    };
    e.analyze(START).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    e.analyze(CHECKMATED).unwrap();
    let u = wait_for(&rx, "terminal frame", |u| u.fen == CHECKMATED);
    assert!(u.done && u.lines.is_empty());
}

const MIDGAME: &str = "rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR w KQkq - 0 3";
const MIDGAME_BLACK: &str = "rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR b KQkq - 0 3";

fn fast_cfg() -> EngineConfig {
    EngineConfig {
        threads: 2,
        hash_mb: 64,
        multi_pv: 3,
        ..EngineConfig::default()
    }
}

#[test]
fn wdl_is_reported_and_white_pov() {
    let Some((e, rx)) = engine(fast_cfg()) else {
        return;
    };
    e.analyze(MIDGAME).unwrap();
    let u = wait_for(&rx, "depth 12 with wdl", |u| u.depth >= 12);
    let w = u.lines[0].wdl.expect("UCI_ShowWDL should be on");
    assert_eq!(w.iter().sum::<u32>(), 1000);
    let frame = build_frame(&u, 0).unwrap();
    assert_eq!(frame.wdl, Some(w), "white to move: same orientation");
    // Black to move: the frame's wdl is mirrored relative to the raw line.
    e.analyze(MIDGAME_BLACK).unwrap();
    let u = wait_for(&rx, "black depth 12", |u| {
        u.fen == MIDGAME_BLACK && u.depth >= 12
    });
    let raw = u.lines[0].wdl.unwrap();
    let frame = build_frame(&u, 0).unwrap();
    assert_eq!(frame.wdl, Some([raw[2], raw[1], raw[0]]));
}

#[test]
fn info_new_game_and_warm_up() {
    let Some((e, rx)) = engine(fast_cfg()) else {
        return;
    };
    let info = e.info();
    assert!(info.ready);
    assert!(
        info.name
            .as_deref()
            .is_some_and(|n| n.contains("Stockfish"))
    );
    assert_eq!((info.threads, info.hash_mb, info.multi_pv), (2, 64, 3));

    let took = e.warm_up().unwrap();
    eprintln!("warm_up took {took:?}");
    assert!(
        rx.try_recv().is_err(),
        "warm-up must not deliver updates to the callback"
    );

    // The warm engine answers the first real position at once.
    let t = Instant::now();
    e.analyze(MIDGAME).unwrap();
    wait_for(&rx, "first update", |u| u.fen == MIDGAME && u.depth >= 1);
    eprintln!("warm: first update after {:?}", t.elapsed());

    e.new_game().unwrap();
    e.analyze(START).unwrap();
    let u = wait_for(&rx, "update after new_game", |u| u.depth >= 6);
    assert_eq!(u.fen, START, "nothing stale after new_game");
    assert!(e.is_alive());
}

#[test]
fn hash_stays_warm_between_analyses_and_new_game_resets_it() {
    let Some((e, rx)) = engine(EngineConfig {
        threads: 1,
        hash_mb: 64,
        multi_pv: 1,
        depth_limit: Some(20),
        ..EngineConfig::default()
    }) else {
        return;
    };
    let nodes_to_finish = |e: &Engine| {
        e.analyze(MIDGAME).unwrap();
        let u = wait_for(&rx, "finish", |u| u.done);
        u.lines[0].nodes
    };
    let cold = nodes_to_finish(&e);
    let warm = nodes_to_finish(&e);
    eprintln!("nodes cold={cold} warm(repeat, hash kept)={warm}");
    assert!(
        warm * 10 < cold * 8,
        "analyze() must keep the hash: {warm} vs {cold}"
    );
    e.new_game().unwrap();
    let after_reset = nodes_to_finish(&e);
    assert!(
        after_reset * 10 > warm * 12,
        "new_game() clears the hash: {after_reset} vs {warm}"
    );
}
