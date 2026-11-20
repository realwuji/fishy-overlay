//! Pure session logic that needs no Tauri handle: when a position's analysis may be shown,
//! how often frames are emitted, which phase the session is in and when the engine must be
//! reset. Kept separate so it can be unit-tested and exercised by the real-pipeline test.

use crate::model::SessionPhase;
use fishy_engine::{EngineUpdate, StabilityConfig, StabilityTracker};
use std::time::{Duration, Instant};

/// After this long without reaching the target depth the best lines so far are shown, flagged
/// as preliminary, so the overlay never stays blank.
pub const PRELIMINARY_AFTER: Duration = Duration::from_millis(2500);
/// Minimum gap between analysis frames (~15 per second).
pub const MIN_FRAME_GAP: Duration = Duration::from_millis(66);

/// How far the search of the current position has come (shown in the session status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub depth: u32,
    /// Target before any preliminary lowering.
    pub target: u32,
    pub done: bool,
    pub nps: u64,
}

impl Progress {
    pub fn reached(&self) -> bool {
        self.done || self.depth >= self.target
    }
}

/// What the emitter should do with the newest update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacing {
    /// Value for `AnalysisFrame.target_depth`.
    pub target_depth: u32,
    /// The target was lowered because the search took too long.
    pub preliminary: bool,
}

/// Pacing of ONE position: target depth, preliminary fallback and frame throttle.
pub struct PositionPacer {
    stability: StabilityTracker,
    started: Instant,
    /// Target before the preliminary fallback (lowered by the stability tracker).
    stable_target: u32,
    /// A frame whose depth reached the target (or a finished search) has been emitted.
    shown: bool,
    last_sent: Option<Instant>,
}

impl PositionPacer {
    pub fn new(min_display_depth: u32, now: Instant) -> PositionPacer {
        let cfg = StabilityConfig {
            min_depth: min_display_depth,
            ..StabilityConfig::default()
        };
        PositionPacer {
            stability: StabilityTracker::new(cfg),
            started: now,
            stable_target: min_display_depth,
            shown: false,
            last_sent: None,
        }
    }

    /// Feed every snapshot of the position in order.
    pub fn observe(&mut self, update: &EngineUpdate) {
        let s = self.stability.feed(update);
        self.stable_target = s.target_depth;
    }

    /// Target depth for a frame of `depth`, with the preliminary fallback applied.
    pub fn pacing(&self, depth: u32, done: bool, now: Instant) -> Pacing {
        let late = now.saturating_duration_since(self.started) >= PRELIMINARY_AFTER;
        if late && !done && depth < self.stable_target {
            Pacing {
                target_depth: depth,
                preliminary: true,
            }
        } else {
            Pacing {
                target_depth: self.stable_target,
                preliminary: false,
            }
        }
    }

    pub fn progress(&self, update: &EngineUpdate) -> Progress {
        Progress {
            depth: update.depth,
            target: self.stable_target,
            done: update.done,
            nps: update.nps,
        }
    }

    /// May a frame be emitted now? The first frame of a position, the frame that reaches the
    /// target and the final frame are never delayed; everything else is rate-limited.
    pub fn allow(&self, done: bool, depth: u32, target: u32, now: Instant) -> bool {
        self.last_sent.is_none()
            || done
            || (!self.shown && depth >= target)
            || now.saturating_duration_since(self.last_sent.unwrap_or(now)) >= MIN_FRAME_GAP
    }

    pub fn sent(&mut self, done: bool, depth: u32, target: u32, now: Instant) {
        self.last_sent = Some(now);
        if done || depth >= target {
            self.shown = true;
        }
    }

    pub fn shown(&self) -> bool {
        self.shown
    }
}

/// Everything that decides the coarse phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseInputs {
    pub engine_ready: bool,
    pub board_found: bool,
    pub tracking: bool,
    /// Progress of the current position, `None` before its first frame.
    pub search: Option<Progress>,
}

pub fn compute_phase(i: PhaseInputs) -> SessionPhase {
    if !i.engine_ready {
        SessionPhase::StartingEngine
    } else if !i.board_found {
        SessionPhase::FindingBoard
    } else if !i.tracking {
        SessionPhase::Recognising
    } else if i.search.is_some_and(|p| p.reached()) {
        SessionPhase::Ready
    } else {
        SessionPhase::Analysing
    }
}

/// Friendly status line for the phases that are about waiting.
pub fn phase_message(phase: SessionPhase) -> Option<&'static str> {
    match phase {
        SessionPhase::StartingEngine => Some("Starting the chess engine..."),
        SessionPhase::FindingBoard => Some("Looking for the chess board on your screen..."),
        SessionPhase::Recognising => Some("Reading the pieces..."),
        SessionPhase::Idle | SessionPhase::Analysing | SessionPhase::Ready => None,
    }
}

const START_PLACEMENT: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";

/// The standard starting position, White to move (move counters are ignored).
pub fn is_start_position(fen: &str) -> bool {
    let mut parts = fen.split_whitespace();
    parts.next() == Some(START_PLACEMENT) && parts.next() == Some("w")
}

/// Should the engine be reset (`ucinewgame`) before analysing `fen`? Only for a genuinely new
/// game: the board went back to the start position, or the position was set explicitly. Plain
/// moves keep the hash table warm.
pub fn needs_new_game(previous: Option<&str>, fen: &str, forced: bool) -> bool {
    if previous.is_none() {
        return false; // a freshly started engine has nothing to clear
    }
    forced || (is_start_position(fen) && !previous.is_some_and(is_start_position))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fishy_engine::{RawLine, Score};

    const FEN: &str = "rnbqkbnr/ppp2ppp/4p3/3p4/3P4/4P3/PPP2PPP/RNBQKBNR w KQkq - 0 3";

    fn update(depth: u32, best: &str, cp: i32, done: bool) -> EngineUpdate {
        EngineUpdate {
            fen: FEN.into(),
            lines: vec![RawLine {
                multipv: 1,
                depth,
                seldepth: depth,
                score: Score::Cp(cp),
                nodes: 1000,
                nps: 1_000_000,
                pv: vec![best.into()],
                wdl: None,
            }],
            depth,
            nps: 1_000_000,
            done,
        }
    }

    #[test]
    fn target_is_the_configured_depth_until_the_position_settles() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(12, t0);
        for d in 1..=11 {
            let u = update(d, if d % 2 == 0 { "g1f3" } else { "b1c3" }, 20, false);
            p.observe(&u);
            assert_eq!(p.pacing(d, false, t0).target_depth, 12, "depth {d}");
        }
    }

    #[test]
    fn a_clear_stable_move_lowers_the_target() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(30, t0);
        let mut last = 30;
        for d in 1..=14 {
            let u = update(d, "g1f3", 20, false);
            p.observe(&u);
            last = p.pacing(d, false, t0).target_depth;
        }
        assert!((8..30).contains(&last), "target {last}");
        assert!(last <= 14);
    }

    #[test]
    fn preliminary_after_two_and_a_half_seconds() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(30, t0);
        let u = update(9, "g1f3", 20, false);
        p.observe(&u);
        let early = p.pacing(9, false, t0 + Duration::from_millis(2000));
        assert_eq!((early.target_depth, early.preliminary), (30, false));
        let late = p.pacing(9, false, t0 + Duration::from_millis(2600));
        assert_eq!((late.target_depth, late.preliminary), (9, true));
        // A finished search is never preliminary.
        let done = p.pacing(9, true, t0 + Duration::from_millis(2600));
        assert!(!done.preliminary);
    }

    #[test]
    fn min_display_depth_zero_shows_immediately() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(0, t0);
        let u = update(1, "g1f3", 20, false);
        p.observe(&u);
        let pc = p.pacing(1, false, t0);
        assert_eq!(pc.target_depth, 0);
        assert!(p.allow(false, 1, pc.target_depth, t0));
    }

    #[test]
    fn forced_move_needs_no_depth() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(12, t0);
        // Black king in the corner, only one legal reply.
        let mut u = update(1, "h8g8", 0, false);
        u.fen = "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1".into();
        p.observe(&u);
        assert_eq!(p.pacing(1, false, t0).target_depth, 1);
    }

    #[test]
    fn throttle_never_delays_first_crossing_or_final_frame() {
        let t0 = Instant::now();
        let mut p = PositionPacer::new(12, t0);
        // First frame of a position goes out at once.
        assert!(p.allow(false, 1, 12, t0));
        p.sent(false, 1, 12, t0);
        // Within the gap: held back...
        let t1 = t0 + Duration::from_millis(10);
        assert!(!p.allow(false, 5, 12, t1));
        // ...unless it reaches the target for the first time...
        assert!(p.allow(false, 12, 12, t1));
        p.sent(false, 12, 12, t1);
        assert!(p.shown());
        // ...or is the final one.
        let t2 = t1 + Duration::from_millis(5);
        assert!(!p.allow(false, 13, 12, t2));
        assert!(p.allow(true, 20, 12, t2));
        // After the gap everything flows again (<= ~15 frames/s).
        assert!(p.allow(false, 14, 12, t1 + MIN_FRAME_GAP));
        assert!(MIN_FRAME_GAP >= Duration::from_millis(66));
    }

    #[test]
    fn phases_follow_the_pipeline() {
        let base = PhaseInputs {
            engine_ready: false,
            board_found: false,
            tracking: false,
            search: None,
        };
        assert_eq!(compute_phase(base), SessionPhase::StartingEngine);
        // The engine boots in parallel with capture: no board yet still means "starting".
        let b = PhaseInputs {
            engine_ready: true,
            ..base
        };
        assert_eq!(compute_phase(b), SessionPhase::FindingBoard);
        let b = PhaseInputs {
            board_found: true,
            ..b
        };
        assert_eq!(compute_phase(b), SessionPhase::Recognising);
        let b = PhaseInputs {
            tracking: true,
            ..b
        };
        assert_eq!(compute_phase(b), SessionPhase::Analysing);
        let mut prog = Progress {
            depth: 7,
            target: 12,
            done: false,
            nps: 1,
        };
        let b = PhaseInputs {
            search: Some(prog),
            ..b
        };
        assert_eq!(compute_phase(b), SessionPhase::Analysing);
        prog.depth = 12;
        let b = PhaseInputs {
            search: Some(prog),
            ..b
        };
        assert_eq!(compute_phase(b), SessionPhase::Ready);
        // Losing the board drops back to recognising.
        let lost = PhaseInputs {
            tracking: false,
            ..b
        };
        assert_eq!(compute_phase(lost), SessionPhase::Recognising);
        assert!(phase_message(SessionPhase::Ready).is_none());
        assert!(phase_message(SessionPhase::StartingEngine).is_some());
    }

    #[test]
    fn new_game_only_for_a_genuine_restart() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(is_start_position(start));
        assert!(!is_start_position(FEN));
        assert!(!is_start_position(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
        ));
        // Fresh engine: nothing to clear.
        assert!(!needs_new_game(None, start, false));
        // An ordinary move keeps the hash warm.
        assert!(!needs_new_game(Some(FEN), FEN, false));
        assert!(!needs_new_game(
            Some(start),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            false
        ));
        // The board went back to the start position: a new game.
        assert!(needs_new_game(Some(FEN), start, false));
        // Re-reading the start position is not.
        assert!(!needs_new_game(Some(start), start, false));
        // An explicitly set position is.
        assert!(needs_new_game(Some(FEN), FEN, true));
    }
}
