//! The analysis session: a worker thread that turns screen frames into overlay data.
//!
//! ```text
//! grab(target) -> verify/detect board -> read_board -> PositionTracker -> FEN
//!                                                                          |
//!      EVT.board  <-  BoardState (overlay-local geometry)                  v
//!      EVT.analysis <- build_frame <- EngineUpdate <- Stockfish (MultiPV) <- analyze(fen)
//! ```
//!
//! One thread owns all mutable state (tracker, templates, engine handle,
//! cached board rectangle). The rest of the app talks to it through a command
//! channel, so there is no shared mutable state to lock around. Captured
//! frames live only inside one loop iteration and are never persisted or sent
//! anywhere.

use crate::engine_path;
use crate::geometry::{FrameSpace, board_to_overlay, overlay_rect_for};
use crate::model::{
    BoardState, EVT_ANALYSIS, EVT_BOARD, EVT_STATUS, EngineStatus, LogicalRect, SessionPhase,
    SessionStatus,
};
use crate::overlay::Overlay;
use crate::progress::{self, PhaseInputs, PositionPacer, Progress};
use crate::settings::{EngineSettings, Settings, SettingsStore};
use crate::templates::{self, Source};
use crate::util::{guard, guard_result};
use fishy_engine::{
    Engine, EngineConfig, EngineError, EngineInfo, EngineUpdate, Hardware, build_frame,
    recommended_config,
};
use fishy_vision::{
    BoardPipeline, BoardRect, CaptureTarget, Frame, Orientation, OrientationSource, Reading,
    Recognizer, Templates, TrackState, VisionError, detect_board, grab, has_permission,
    learn_from_start, list_targets, verify_board,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// How often the engine facts (nodes/s) in the status may change.
const NPS_PUBLISH_EVERY: Duration = Duration::from_millis(500);
/// How often the worker wakes to service commands and flush pending analysis.
const POLL: Duration = Duration::from_millis(40);
/// Do not re-run full board detection more often than this while no board is known.
const DETECT_RETRY: Duration = Duration::from_millis(400);
const WINDOW_TARGET_POLL: Duration = Duration::from_millis(500);
const MONITOR_TARGET_POLL: Duration = Duration::from_millis(1500);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const NOTE_TTL: Duration = Duration::from_secs(10);
const MAX_PANIC_STREAK: u32 = 3;
/// Automatic engine restarts allowed per session before giving up.
const ENGINE_RESTARTS: u32 = 1;

const MSG_PERMISSION: &str = "Screen Recording permission is required. Grant it in System Settings, then restart Fishy if macOS asks you to.";

// ---------------------------------------------------------------- pure helpers

/// The monitor to use when no target is chosen: the one at the desktop origin,
/// otherwise the first monitor.
/// Frames of failed reading with built-in templates before suggesting calibration.
const UNREADABLE_FRAMES: u32 = 12;

pub fn pick_primary(targets: &[CaptureTarget]) -> Option<&CaptureTarget> {
    let monitors = || targets.iter().filter(|t| t.kind == "monitor");
    monitors()
        .find(|t| t.x == 0 && t.y == 0)
        .or_else(|| monitors().next())
}

/// Resolve the configured target id (None = primary monitor) against a listing.
pub fn resolve_target<'a>(
    targets: &'a [CaptureTarget],
    wanted: Option<&str>,
) -> Option<&'a CaptureTarget> {
    match wanted {
        Some(id) => targets.iter().find(|t| t.id == id),
        None => pick_primary(targets),
    }
}

/// Strictly increasing timestamp source: never returns a value <= `last`.
pub fn next_ts(last: u64, now: u64) -> u64 {
    now.max(last.saturating_add(1))
}

fn frame_interval(fps: u32) -> Duration {
    Duration::from_millis(1000 / u64::from(fps.max(1)))
}

/// The engine options to use: with `auto`, threads and hash come from the machine (measured
/// policy in `fishy_engine::recommended_config`); everything else stays under user control.
fn engine_config(e: &EngineSettings, hw: &Hardware) -> EngineConfig {
    let (threads, hash_mb) = if e.auto {
        let rec = recommended_config(hw);
        (rec.threads, rec.hash_mb)
    } else {
        (e.threads, e.hash_mb)
    };
    EngineConfig {
        threads,
        hash_mb,
        multi_pv: e.multi_pv,
        depth_limit: e.depth_limit,
        move_time_ms: e.move_time_ms,
        skill_level: e.skill_level,
    }
}

fn describe_vision_error(e: &VisionError) -> String {
    match e {
        VisionError::PermissionDenied => MSG_PERMISSION.to_string(),
        VisionError::TargetNotFound(t) => {
            format!("The capture target ({t}) is no longer available. Waiting for it to come back.")
        }
        VisionError::Capture(m) => format!("Screen capture failed: {m}"),
        VisionError::Other(m) => m.clone(),
    }
}

// ------------------------------------------------------------ shared status

/// State visible to the rest of the app. Emits events whenever it changes.
struct Shared {
    app: AppHandle,
    status: Mutex<SessionStatus>,
    board: Mutex<BoardState>,
}

impl Shared {
    fn update_status(&self, f: impl FnOnce(&mut SessionStatus)) -> SessionStatus {
        let (snapshot, changed) = {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            let before = s.clone();
            f(&mut s);
            (s.clone(), *s != before)
        };
        if changed {
            let _ = self.app.emit(EVT_STATUS, &snapshot);
        }
        snapshot
    }

    fn set_board(&self, board: BoardState) {
        let changed = {
            let mut b = self.board.lock().unwrap_or_else(|e| e.into_inner());
            if *b == board {
                false
            } else {
                *b = board.clone();
                true
            }
        };
        if changed {
            let _ = self.app.emit(EVT_BOARD, &board);
        }
    }
}

// ------------------------------------------------------- analysis forwarding

/// Converts engine snapshots into `AnalysisFrame` events: monotonic timestamps, a target depth
/// per position, a preliminary fallback and a ~15 Hz throttle with a trailing edge so the last
/// update of a burst is never lost (the first frame of a position is never delayed).
struct AnalysisEmitter {
    app: AppHandle,
    start: Instant,
    last_ts: AtomicU64,
    min_display_depth: AtomicU32,
    slot: Mutex<Slot>,
}

/// Emitter state guarded by one lock so that "is this update still current?" and "emit it"
/// cannot be separated by a position change.
#[derive(Default)]
struct Slot {
    /// Position the session currently expects; `None` accepts any update.
    expected: Option<String>,
    pacer: Option<PositionPacer>,
    /// Update held back by the throttle.
    pending: Option<EngineUpdate>,
    /// Newest update of the position (for the preliminary fallback).
    last: Option<EngineUpdate>,
    progress: Option<Progress>,
}

impl AnalysisEmitter {
    fn new(app: AppHandle, min_display_depth: u32) -> AnalysisEmitter {
        AnalysisEmitter {
            app,
            start: Instant::now(),
            last_ts: AtomicU64::new(0),
            min_display_depth: AtomicU32::new(min_display_depth),
            slot: Mutex::new(Slot::default()),
        }
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn set_min_display_depth(&self, depth: u32) {
        self.min_display_depth.store(depth, Ordering::Relaxed);
    }

    fn emit(&self, slot: &mut Slot, update: &EngineUpdate, now: Instant) {
        let Some(pacer) = slot.pacer.as_mut() else {
            return;
        };
        let pacing = pacer.pacing(update.depth, update.done, now);
        pacer.sent(update.done, update.depth, pacing.target_depth, now);
        let wall = self.now_ms();
        let prev = self
            .last_ts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                Some(next_ts(last, wall))
            })
            .unwrap_or(0);
        let ts = next_ts(prev, wall);
        match build_frame(update, ts) {
            Ok(mut frame) => {
                frame.target_depth = Some(pacing.target_depth);
                let _ = self.app.emit(EVT_ANALYSIS, &frame);
            }
            Err(e) => eprintln!("fishy: could not build analysis frame: {e}"),
        }
    }

    fn lock_slot(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called from the engine's reader thread.
    fn handle(&self, update: EngineUpdate) {
        let now = Instant::now();
        let mut slot = self.lock_slot();
        if slot.expected.as_deref().is_some_and(|f| f != update.fen) {
            return;
        }
        let depth_cfg = self.min_display_depth.load(Ordering::Relaxed);
        let pacer = slot
            .pacer
            .get_or_insert_with(|| PositionPacer::new(depth_cfg, now));
        pacer.observe(&update);
        let pacing = pacer.pacing(update.depth, update.done, now);
        let allowed = pacer.allow(update.done, update.depth, pacing.target_depth, now);
        let progress = pacer.progress(&update);
        slot.progress = Some(progress);
        slot.last = Some(update.clone());
        if allowed {
            slot.pending = None;
            self.emit(&mut slot, &update, now);
        } else {
            slot.pending = Some(update);
        }
    }

    /// Emit a held-back update once the throttle window has passed, or show preliminary lines
    /// when the target depth is taking too long.
    fn flush(&self) {
        let now = Instant::now();
        let mut slot = self.lock_slot();
        let Some(pacer) = slot.pacer.as_ref() else {
            return;
        };
        let from_pending = slot.pending.is_some();
        let candidate = slot.pending.clone().or_else(|| {
            // Nothing held back: only a position that was never shown may still need the
            // preliminary fallback.
            (!pacer.shown()).then(|| slot.last.clone()).flatten()
        });
        let Some(update) = candidate else { return };
        let pacing = pacer.pacing(update.depth, update.done, now);
        if pacer.allow(update.done, update.depth, pacing.target_depth, now)
            && (from_pending || pacing.preliminary || update.depth >= pacing.target_depth)
        {
            slot.pending = None;
            self.emit(&mut slot, &update, now);
        }
    }

    /// Search progress of the position currently expected.
    fn progress(&self) -> Option<Progress> {
        self.lock_slot().progress
    }

    /// Drop anything held back and accept updates for any position again.
    fn reset(&self) {
        *self.lock_slot() = Slot::default();
    }

    /// A new position is about to be analysed: drop held-back output and reject every update
    /// (pending or fresh) that is not for `fen`.
    fn expect(&self, fen: &str) {
        let depth = self.min_display_depth.load(Ordering::Relaxed);
        *self.lock_slot() = Slot {
            expected: Some(fen.to_owned()),
            pacer: Some(PositionPacer::new(depth, Instant::now())),
            ..Slot::default()
        };
    }
}

// -------------------------------------------------------------- worker thread

type Reply = mpsc::Sender<Result<BoardState, String>>;

enum Cmd {
    Calibrate(Reply),
    Recalibrate(Reply),
    SetFen(String, Reply),
    SettingsChanged,
    Stop,
}

struct WorkerContext {
    shared: Arc<Shared>,
    settings: Arc<SettingsStore>,
    overlay: Arc<Overlay>,
    data_dir: PathBuf,
    app: AppHandle,
}

struct Worker {
    shared: Arc<Shared>,
    store: Arc<SettingsStore>,
    overlay: Arc<Overlay>,
    data_dir: PathBuf,
    cfg: Settings,

    emitter: Arc<AnalysisEmitter>,
    engine: Option<Engine>,
    /// The engine is being spawned and warmed up on its own thread (started with the session so
    /// capture and recognition run meanwhile).
    engine_boot: Option<mpsc::Receiver<Result<(Engine, EngineInfo), String>>>,
    engine_info: Option<EngineInfo>,
    /// Options changed while the engine was still starting.
    pending_config: Option<EngineConfig>,
    hardware: Hardware,
    engine_path: PathBuf,
    restarts_left: u32,
    /// The next analysis starts a new game (`ucinewgame`): a position was set explicitly.
    force_new_game: bool,
    /// Phase publishing state.
    phase: SessionPhase,
    phase_msg: Option<String>,
    board_found: bool,
    tracking: bool,
    shown_nps: u64,
    nps_at: Instant,

    /// Orientation decision (manual / labels / structure / calibration) plus the tracker.
    pipeline: BoardPipeline,
    /// Why the orientation is unknown or contradicted (shown above the generic hint).
    orient_status: Option<String>,
    templates: Option<Templates>,
    /// Parsed form of `templates`, kept so frames are not re-parsed every tick.
    recognizer: Option<Recognizer>,
    template_source: Option<Source>,
    /// Consecutive frames the built-in templates could not make sense of the board.
    unreadable_frames: u32,
    /// Persistent, lowest-priority status line (which pieces are in use, or how to fix reading).
    hint: Option<String>,

    targets_polled: Option<Instant>,
    target: Option<CaptureTarget>,
    overlay_rect: Option<LogicalRect>,
    rect: Option<BoardRect>,
    last_detect: Option<Instant>,
    last_fen: Option<String>,
    /// Last position handed to the engine (survives `last_fen` resets, which only re-issue it).
    analysed_fen: Option<String>,

    /// Capture-side problem (permission, vanished target, grab error); cleared by a good capture.
    capture_problem: Option<String>,
    /// Engine/internal notice with the time it was raised; non-fatal ones expire.
    note: Option<(String, Instant)>,
    /// Last message pushed into the status (to publish only on change).
    published: Option<String>,
    panic_streak: u32,
    fatal: bool,
}

impl Worker {
    fn new(ctx: WorkerContext) -> Result<Worker, String> {
        let cfg = ctx.settings.get();
        let orientation_mode = cfg.capture.orientation;
        let engine_path = engine_path::resolve()?;
        let emitter = Arc::new(AnalysisEmitter::new(ctx.app, cfg.engine.min_display_depth));
        let hardware = Hardware::detect();
        let engine_boot = Some(Self::boot_engine(
            engine_path.clone(),
            engine_config(&cfg.engine, &hardware),
            Arc::clone(&emitter),
        ));
        let mut worker = Worker {
            templates: None,
            recognizer: None,
            template_source: None,
            unreadable_frames: 0,
            hint: None,
            shared: ctx.shared,
            store: ctx.settings,
            overlay: ctx.overlay,
            data_dir: ctx.data_dir,
            cfg,
            emitter,
            engine: None,
            engine_boot,
            engine_info: None,
            pending_config: None,
            hardware,
            engine_path,
            restarts_left: ENGINE_RESTARTS,
            force_new_game: false,
            phase: SessionPhase::StartingEngine,
            phase_msg: progress::phase_message(SessionPhase::StartingEngine).map(String::from),
            board_found: false,
            tracking: false,
            shown_nps: 0,
            nps_at: Instant::now(),
            pipeline: BoardPipeline::new(orientation_mode),
            orient_status: None,
            targets_polled: None,
            target: None,
            overlay_rect: None,
            rect: None,
            last_detect: None,
            last_fen: None,
            analysed_fen: None,
            capture_problem: None,
            note: None,
            published: None,
            panic_streak: 0,
            fatal: false,
        };
        worker.reload_templates();
        Ok(worker)
    }

    /// Pick the templates for the configured site: saved calibration first, else built-in.
    fn reload_templates(&mut self) {
        match templates::resolve(&self.data_dir, self.cfg.site) {
            Some((t, source)) => self.install_templates(t, source),
            None => {
                self.templates = None;
                self.recognizer = None;
                self.template_source = None;
                self.unreadable_frames = 0;
                self.set_hint(Some(format!(
                    "No built-in pieces for {} yet. Show the starting position, then press Calibrate in Fishy > Overview.",
                    templates::site_name(self.cfg.site)
                )));
            }
        }
    }

    fn install_templates(&mut self, t: Templates, source: Source) {
        self.recognizer = Recognizer::new(&t);
        self.templates = Some(t);
        self.template_source = Some(source);
        self.unreadable_frames = 0;
        let label = source.label(self.cfg.site);
        self.set_hint(Some(format!("Reading the board with {label}.")));
    }

    fn set_hint(&mut self, hint: Option<String>) {
        self.hint = hint;
        self.refresh_message();
    }

    fn spawn_engine(
        path: &std::path::Path,
        cfg: EngineConfig,
        emitter: &Arc<AnalysisEmitter>,
    ) -> Result<Engine, String> {
        let em = Arc::clone(emitter);
        guard_result(|| {
            Engine::spawn(path, cfg, move |u| em.handle(u))
                .map_err(|e| format!("Could not start the chess engine: {e}"))
        })
    }

    /// Spawn and warm up the engine on its own thread; the session keeps capturing meanwhile.
    fn boot_engine(
        path: PathBuf,
        cfg: EngineConfig,
        emitter: Arc<AnalysisEmitter>,
    ) -> mpsc::Receiver<Result<(Engine, EngineInfo), String>> {
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("fishy-engine-boot".into())
            .spawn(move || {
                let r = Self::spawn_engine(&path, cfg, &emitter).map(|engine| {
                    // Warm-up output is discarded by the engine; failing here is harmless.
                    let _ = guard(|| engine.warm_up());
                    let info = engine.info();
                    (engine, info)
                });
                let _ = tx.send(r);
            });
        if let Err(e) = spawned {
            let (tx2, rx2) = mpsc::channel();
            let _ = tx2.send(Err(format!("Could not start the engine thread: {e}")));
            return rx2;
        }
        rx
    }

    /// Pick up the engine once it has finished starting.
    fn poll_engine_boot(&mut self) {
        let Some(rx) = self.engine_boot.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((engine, info))) => {
                self.engine_boot = None;
                let mut info = info;
                if let Some(cfg) = self.pending_config.take()
                    && guard_result(|| engine.configure(cfg).map_err(|e| e.to_string())).is_ok()
                {
                    info = engine.info();
                }
                self.engine = Some(engine);
                self.engine_info = Some(info);
                self.last_fen = None; // analyse whatever is on the board right away
            }
            Ok(Err(e)) => {
                self.engine_boot = None;
                self.fail(e);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.engine_boot = None;
                self.fail("The chess engine could not be started.".to_string());
            }
        }
    }

    fn engine_ready(&self) -> bool {
        self.engine.is_some()
    }

    // ---- phase -----------------------------------------------------------

    /// Work out the phase and engine facts and publish them when they changed.
    fn update_phase(&mut self) {
        if self.fatal {
            return;
        }
        let search = if self.last_fen.is_some() {
            self.emitter.progress()
        } else {
            None
        };
        let phase = progress::compute_phase(PhaseInputs {
            engine_ready: self.engine_ready(),
            board_found: self.board_found,
            tracking: self.tracking,
            search,
        });
        if self.nps_at.elapsed() >= NPS_PUBLISH_EVERY {
            self.nps_at = Instant::now();
            if let Some(p) = search {
                self.shown_nps = p.nps;
            }
        }
        let engine = self.engine_info.as_ref().map(|i| EngineStatus {
            name: i.name.clone(),
            threads: i.threads,
            hash_mb: i.hash_mb,
            nps: self.shown_nps,
            ready: self.engine_ready(),
        });
        if phase != self.phase {
            self.phase = phase;
            self.phase_msg = progress::phase_message(phase).map(String::from);
            self.refresh_message();
        }
        self.shared.update_status(|s| {
            s.phase = phase;
            s.engine = engine;
        });
    }

    // ---- main loop -------------------------------------------------------

    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        let mut next_frame = Instant::now();
        loop {
            let wait = next_frame
                .saturating_duration_since(Instant::now())
                .min(POLL);
            match rx.recv_timeout(wait) {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle_cmd(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= next_frame {
                next_frame = Instant::now() + frame_interval(self.cfg.capture.fps);
                self.guarded_tick();
            }
            self.poll_engine_boot();
            self.emitter.flush();
            self.update_phase();
            if self.fatal {
                break;
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(engine) = self.engine.take() {
            let _ = guard(|| {
                engine.stop();
                engine.quit();
            });
        }
        self.emitter.reset();
        self.overlay.set_session_active(false);
        self.shared.set_board(BoardState::idle());
        let msg = if self.fatal {
            self.note.as_ref().map(|(m, _)| m.clone())
        } else {
            None
        };
        self.shared.update_status(|s| {
            s.running = false;
            s.message = msg;
            s.phase = SessionPhase::Idle;
            s.engine = None;
        });
    }

    fn guarded_tick(&mut self) {
        match guard(|| self.tick()) {
            Ok(()) => self.panic_streak = 0,
            Err(msg) => {
                self.panic_streak += 1;
                eprintln!("fishy: session tick panicked: {msg}");
                if self.panic_streak >= MAX_PANIC_STREAK {
                    self.fail(format!(
                        "The session stopped after repeated internal errors ({msg})."
                    ));
                } else {
                    self.set_note(Some(format!("Internal error: {msg}")));
                }
            }
        }
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Calibrate(reply) => {
                let r = guard_result(|| self.calibrate());
                let _ = reply.send(r);
            }
            Cmd::Recalibrate(reply) => {
                let r = guard_result(|| self.recalibrate());
                let _ = reply.send(r);
            }
            Cmd::SetFen(fen, reply) => {
                let r = guard_result(|| self.set_known_position(&fen));
                let _ = reply.send(r);
            }
            Cmd::SettingsChanged => {
                if let Err(e) = guard(|| self.apply_settings()) {
                    self.set_note(Some(format!("Internal error applying settings: {e}")));
                }
            }
            Cmd::Stop => {}
        }
    }

    // ---- status helpers --------------------------------------------------

    /// Push the highest-priority message into the status when it changed.
    fn refresh_message(&mut self) {
        let msg = self
            .capture_problem
            .clone()
            .or_else(|| self.note.as_ref().map(|(m, _)| m.clone()))
            .or_else(|| self.orient_status.clone())
            .or_else(|| self.phase_msg.clone())
            .or_else(|| self.hint.clone());
        if self.published != msg {
            self.published = msg.clone();
            self.shared.update_status(|s| s.message = msg);
        }
    }

    fn set_orient_status(&mut self, status: Option<String>) {
        if self.orient_status != status {
            self.orient_status = status;
            self.refresh_message();
        }
    }

    fn set_problem(&mut self, problem: Option<String>) {
        self.capture_problem = problem;
        self.refresh_message();
    }

    fn set_note(&mut self, note: Option<String>) {
        self.note = note.map(|m| (m, Instant::now()));
        self.refresh_message();
    }

    fn expire_note(&mut self) {
        if !self.fatal
            && self
                .note
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() >= NOTE_TTL)
        {
            self.set_note(None);
        }
    }

    fn fail(&mut self, message: String) {
        self.fatal = true;
        self.set_note(Some(message));
    }

    // ---- target tracking -------------------------------------------------

    fn poll_due(&self) -> bool {
        let interval = match &self.target {
            Some(t) if t.kind == "window" => WINDOW_TARGET_POLL,
            _ => MONITOR_TARGET_POLL,
        };
        self.target.is_none() || self.targets_polled.is_none_or(|t| t.elapsed() >= interval)
    }

    /// Re-list capture targets and (re)select the configured one.
    fn refresh_target(&mut self) {
        self.targets_polled = Some(Instant::now());
        let targets = match list_targets() {
            Ok(t) => t,
            Err(e) => {
                self.set_problem(Some(describe_vision_error(&e)));
                return;
            }
        };
        let wanted = self.cfg.capture.target_id.clone();
        match resolve_target(&targets, wanted.as_deref()) {
            Some(t) => {
                let changed = self.target.as_ref().is_none_or(|old| {
                    old.id != t.id || old.width != t.width || old.height != t.height
                });
                if changed {
                    self.rect = None;
                    self.last_detect = None;
                }
                let rect = overlay_rect_for(t);
                self.overlay_rect = Some(rect);
                self.overlay.apply_rect(rect);
                self.target = Some(t.clone());
            }
            None => {
                self.target = None;
                self.overlay_rect = None;
                self.rect = None;
                self.overlay.clear_rect();
                let what = wanted.map_or_else(|| "No monitor was found to capture.".to_string(), |id| {
                    format!("The capture target ({id}) is not available. Waiting for it to come back, or pick another in Capture settings.")
                });
                self.set_problem(Some(what));
                self.shared.set_board(self.no_board_state());
            }
        }
    }

    /// Make sure there is a live target and permission, then capture one frame.
    fn capture(&mut self) -> Result<Frame, String> {
        if self.poll_due() {
            self.refresh_target();
        }
        let Some(target) = self.target.clone() else {
            return Err(self
                .capture_problem
                .clone()
                .unwrap_or_else(|| "No capture target available.".to_string()));
        };
        if !has_permission() {
            self.shared.update_status(|s| s.permission = false);
            self.set_problem(Some(MSG_PERMISSION.to_string()));
            return Err(MSG_PERMISSION.to_string());
        }
        self.shared.update_status(|s| s.permission = true);
        match grab(&target.id) {
            Ok(frame) => {
                self.set_problem(None);
                Ok(frame)
            }
            Err(e) => {
                let msg = describe_vision_error(&e);
                match e {
                    VisionError::PermissionDenied => {
                        self.shared.update_status(|s| s.permission = false);
                    }
                    VisionError::TargetNotFound(_) => {
                        // Force a re-list next tick; the window may have closed.
                        self.target = None;
                        self.rect = None;
                        self.overlay.clear_rect();
                        self.shared.set_board(self.no_board_state());
                    }
                    _ => {}
                }
                self.set_problem(Some(msg.clone()));
                Err(msg)
            }
        }
    }

    // ---- per-frame pipeline ---------------------------------------------

    fn tick(&mut self) {
        let Ok(frame) = self.capture() else { return };
        self.expire_note();
        let board = self.process_frame(&frame, false);
        self.shared.set_board(board);
    }

    fn no_board_state(&self) -> BoardState {
        BoardState {
            geometry: None,
            fen: None,
            last_move: None,
            state: if self.templates.is_some() {
                TrackState::Lost
            } else {
                TrackState::NeedsCalibration
            },
            confidence: 0.0,
            calibrated: self.templates.is_some(),
        }
    }

    /// Locate the board (cached rect first), read it, track it, kick off analysis.
    /// `force_detect` bypasses the cache and the retry back-off.
    fn process_frame(&mut self, frame: &Frame, force_detect: bool) -> BoardState {
        let site = self.cfg.site;
        if force_detect {
            self.rect = None;
        }
        if let Some(r) = self.rect
            && !verify_board(frame, &r, site)
        {
            self.rect = None;
        }
        let mut redetected = false;
        if self.rect.is_none()
            && (force_detect || self.last_detect.is_none_or(|t| t.elapsed() >= DETECT_RETRY))
        {
            self.last_detect = Some(Instant::now());
            self.rect = detect_board(frame, site);
            redetected = self.rect.is_some();
        }
        let Some(rect) = self.rect else {
            self.board_found = false;
            self.tracking = false;
            return self.no_board_state();
        };
        self.board_found = true;

        let overlay = self.overlay_rect.unwrap_or(LogicalRect {
            x: frame.origin_x as f64,
            y: frame.origin_y as f64,
            width: frame.width as f64 / f64::from(frame.scale.max(0.01)),
            height: frame.height as f64 / f64::from(frame.scale.max(0.01)),
        });
        let space = FrameSpace::from(frame);
        let Some(recognizer) = self.recognizer.as_ref() else {
            self.tracking = false;
            return BoardState {
                geometry: Some(board_to_overlay(
                    &rect,
                    space,
                    &overlay,
                    self.pipeline
                        .resolver
                        .current()
                        .unwrap_or(Orientation::White),
                )),
                fen: None,
                last_move: None,
                state: TrackState::NeedsCalibration,
                confidence: 0.0,
                calibrated: false,
            };
        };

        // Orientation is decided, never defaulted: manual > coordinate labels > structure >
        // calibration (see fishy_vision::orient). While it is unknown nothing is tracked.
        let out = self
            .pipeline
            .process(recognizer, frame, &rect, site, redetected);
        let tracked = out.tracked;
        if out.changed {
            self.last_fen = None;
            if let Some(o) = out.orientation {
                let how = match self.pipeline.resolver.source() {
                    Some(OrientationSource::Manual) => "as set",
                    Some(OrientationSource::Labels) => "from the board's coordinates",
                    Some(OrientationSource::Structure) => "from the position",
                    _ => "from calibration",
                };
                self.set_note(Some(format!(
                    "{} is at the bottom of the board ({how}).",
                    if o == Orientation::White {
                        "White"
                    } else {
                        "Black"
                    }
                )));
            }
        }
        self.set_orient_status(out.status);
        if let Some(reading) = &out.reading {
            self.note_readability(reading, &tracked.state);
        }

        self.tracking = tracked.state == TrackState::Tracking;
        // While the engine is still starting the position simply waits (it is picked up as soon
        // as the engine is ready); it is not an engine failure.
        if self.tracking
            && self.engine_ready()
            && let Some(fen) = tracked.fen.as_deref()
            && self.last_fen.as_deref() != Some(fen)
        {
            self.start_analysis(fen.to_string());
        }

        BoardState {
            geometry: Some(board_to_overlay(
                &rect,
                space,
                &overlay,
                tracked.orientation,
            )),
            fen: tracked.fen,
            last_move: tracked.last_move,
            state: tracked.state,
            confidence: tracked.confidence,
            calibrated: true,
        }
    }

    /// Built-in templates only know the default piece set: when they cannot read the board for a
    /// while, say so and point at calibration instead of staying silent.
    fn note_readability(&mut self, reading: &Reading, state: &TrackState) {
        if self.template_source != Some(Source::BuiltIn) {
            return;
        }
        let unknown = reading.placement.iter().filter(|c| **c == '?').count();
        let unreadable =
            *state != TrackState::Tracking && (reading.confidence < 0.5 || unknown > 8);
        if !unreadable {
            if self.unreadable_frames >= UNREADABLE_FRAMES {
                let label = Source::BuiltIn.label(self.cfg.site);
                self.set_hint(Some(format!("Reading the board with {label}.")));
            }
            self.unreadable_frames = 0;
            return;
        }
        self.unreadable_frames = self.unreadable_frames.saturating_add(1);
        if self.unreadable_frames == UNREADABLE_FRAMES {
            self.set_hint(Some(
                "These pieces do not look like the built-in set. Show the starting position, then press Calibrate in Fishy > Overview."
                    .to_string(),
            ));
        }
    }

    // ---- engine ----------------------------------------------------------

    fn start_analysis(&mut self, fen: String) {
        // Only a genuinely new game clears the engine's hash; ordinary moves keep it warm.
        let new_game = progress::needs_new_game(
            self.analysed_fen.as_deref(),
            &fen,
            std::mem::take(&mut self.force_new_game),
        );
        if new_game && let Some(e) = self.engine.as_ref() {
            let _ = guard_result(|| e.new_game().map_err(|e| e.to_string()));
        }
        self.analysed_fen = Some(fen.clone());
        self.emitter.expect(&fen);
        let result = match self.engine.as_ref() {
            Some(e) => match guard(|| e.analyze(&fen)) {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => Err((err.to_string(), !matches!(err, EngineError::InvalidFen(_)))),
                Err(panic) => Err((panic, true)),
            },
            None => Err(("engine is not running".to_string(), true)),
        };
        match result {
            Ok(()) => self.last_fen = Some(fen),
            Err((e, false)) => {
                // A position the engine layer rejects is not an engine failure: keep the healthy
                // process and the previous analysis, and do not retry the same input.
                eprintln!("fishy: skipped unanalysable position ({e})");
                self.last_fen = Some(fen);
                self.emitter.reset();
            }
            Err((e, true)) => {
                self.last_fen = None;
                self.recover_engine(&e);
                if !self.fatal
                    && let Some(engine) = self.engine.as_ref()
                    && guard_result(|| engine.analyze(&fen).map_err(|e| e.to_string())).is_ok()
                {
                    self.last_fen = Some(fen);
                }
            }
        }
    }

    /// Restart the engine once; if that was already used up, end the session with a clear message.
    fn recover_engine(&mut self, cause: &str) {
        if let Some(old) = self.engine.take() {
            let _ = guard(|| old.quit());
        }
        if self.restarts_left == 0 {
            self.fail(format!("The chess engine stopped unexpectedly ({cause}). Stop and start the session to try again."));
            return;
        }
        self.restarts_left -= 1;
        let cfg = engine_config(&self.cfg.engine, &self.hardware);
        match Self::spawn_engine(&self.engine_path, cfg, &self.emitter) {
            Ok(engine) => {
                self.engine_info = Some(engine.info());
                self.engine = Some(engine);
                self.analysed_fen = None;
                self.set_note(Some(
                    "The chess engine stopped and was restarted.".to_string(),
                ));
            }
            Err(e) => self.fail(format!(
                "The chess engine stopped unexpectedly and could not be restarted. {e}"
            )),
        }
    }

    // ---- settings changes ------------------------------------------------

    fn apply_settings(&mut self) {
        let new = self.store.get();
        let old = std::mem::replace(&mut self.cfg, new.clone());

        self.emitter
            .set_min_display_depth(new.engine.min_display_depth);
        let new_cfg = engine_config(&new.engine, &self.hardware);
        let changed = new_cfg != engine_config(&old.engine, &self.hardware)
            || new.engine.min_display_depth != old.engine.min_display_depth;
        if changed {
            match self.engine.as_ref() {
                Some(e) => {
                    let result =
                        guard_result(|| e.configure(new_cfg.clone()).map_err(|e| e.to_string()));
                    match result {
                        Ok(()) => {
                            self.engine_info = Some(e.info());
                            self.last_fen = None; // re-issue analysis with the new options
                        }
                        Err(e) => self.recover_engine(&e),
                    }
                }
                // Still starting: the new options are used by the next spawn only if the boot has
                // not read them yet, so apply them right after it finishes.
                None if self.engine_boot.is_some() => self.pending_config = Some(new_cfg),
                None => self.recover_engine("engine is not running"),
            }
        }
        if new.site != old.site {
            self.reload_templates();
            self.pipeline = BoardPipeline::new(new.capture.orientation);
            self.set_orient_status(None);
            self.rect = None;
            self.last_detect = None;
            self.last_fen = None;
        }
        if new.site == old.site && new.capture.orientation != old.capture.orientation {
            if self.pipeline.resolver.set_mode(new.capture.orientation) {
                self.pipeline.reset_tracker();
            }
            self.set_orient_status(None);
            self.last_fen = None;
        }
        if new.capture.target_id != old.capture.target_id {
            self.target = None;
            self.rect = None;
            self.last_detect = None;
            self.targets_polled = None;
        }
        if new.capture.fps != old.capture.fps {
            let fps = new.capture.fps;
            self.shared.update_status(|s| s.capture_fps = fps);
        }
    }

    // ---- user-driven calibration ----------------------------------------

    fn publish(&mut self, frame: &Frame, force_detect: bool) -> BoardState {
        let board = self.process_frame(frame, force_detect);
        self.shared.set_board(board.clone());
        board
    }

    fn calibrate(&mut self) -> Result<BoardState, String> {
        let frame = self.capture()?;
        let site = self.cfg.site;
        let rect = detect_board(&frame, site).ok_or_else(no_board_message)?;
        let (learned, orientation) = learn_from_start(&frame, &rect)
            .map_err(|e| format!("Could not learn the pieces from this position: {e}"))?;
        templates::save(&self.data_dir, site, &learned)?;
        self.install_templates(learned, Source::User);
        self.rect = Some(rect);
        // The start position shows who is at the bottom: authoritative unless the user fixed it.
        self.pipeline.resolver.note_calibration(orientation);
        self.pipeline.reset_tracker();
        self.pipeline.tracker.reset_to_start();
        self.set_orient_status(None);
        self.last_fen = None;
        self.force_new_game = true;
        if self
            .pipeline
            .resolver
            .current()
            .is_some_and(|o| o != orientation)
        {
            self.set_note(Some(
                "The calibration position shows the other colour at the bottom than the orientation you set; choose Auto or the right colour."
                    .to_string(),
            ));
        }
        Ok(self.publish(&frame, false))
    }

    fn recalibrate(&mut self) -> Result<BoardState, String> {
        let frame = self.capture()?;
        self.rect = None;
        self.last_detect = None;
        let board = self.publish(&frame, true);
        if board.geometry.is_none() {
            return Err(no_board_message());
        }
        Ok(board)
    }

    fn set_known_position(&mut self, fen: &str) -> Result<BoardState, String> {
        let fen = fen.trim();
        if self.templates.is_none() {
            return Err(
                "Calibrate on a starting position once first: Fishy learns your piece set from it."
                    .to_string(),
            );
        }
        let frame = self.capture()?;
        if self.pipeline.resolver.current().is_none() {
            // Unknown orientation: the position itself says which view of the board is right.
            let site = self.cfg.site;
            let rect = self
                .rect
                .or_else(|| detect_board(&frame, site))
                .ok_or_else(no_board_message)?;
            let recognizer = self
                .recognizer
                .as_ref()
                .ok_or_else(|| "Calibrate on a starting position once first.".to_string())?;
            self.pipeline
                .set_known_position(recognizer, &frame, &rect, fen)?;
            self.rect = Some(rect);
            self.set_orient_status(None);
        } else {
            self.pipeline
                .tracker
                .reset_to_fen(fen)
                .map_err(|e| format!("That position is not valid: {e}"))?;
        }
        self.last_fen = None;
        self.force_new_game = true;
        Ok(self.publish(&frame, false))
    }
}

fn no_board_message() -> String {
    "No chess board was found on the captured screen. Make sure the whole board is visible and not covered.".to_string()
}

// ------------------------------------------------------------- controller

struct WorkerHandle {
    tx: mpsc::Sender<Cmd>,
    join: JoinHandle<()>,
}

/// Owner of the session worker; the app-facing API.
pub struct SessionController {
    app: AppHandle,
    settings: Arc<SettingsStore>,
    overlay: Arc<Overlay>,
    data_dir: PathBuf,
    shared: Arc<Shared>,
    worker: Mutex<Option<WorkerHandle>>,
}

impl SessionController {
    pub fn new(
        app: AppHandle,
        settings: Arc<SettingsStore>,
        overlay: Arc<Overlay>,
        data_dir: PathBuf,
    ) -> SessionController {
        let fps = settings.get().capture.fps;
        let shared = Arc::new(Shared {
            app: app.clone(),
            status: Mutex::new(SessionStatus {
                running: false,
                permission: true,
                capture_fps: fps,
                message: None,
                phase: SessionPhase::Idle,
                engine: None,
            }),
            board: Mutex::new(BoardState::idle()),
        });
        SessionController {
            app,
            settings,
            overlay,
            data_dir,
            shared,
            worker: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.shared
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running
    }

    /// Set a non-fatal informational message (e.g. a hotkey that could not be registered).
    pub fn note(&self, message: Option<String>) {
        self.shared.update_status(|s| s.message = message);
    }

    /// Current status, with the permission flag freshly checked while idle.
    pub fn status(&self) -> SessionStatus {
        let fps = self.settings.get().capture.fps;
        let running = self.is_running();
        let permission = if running {
            self.shared
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .permission
        } else {
            guard(has_permission).unwrap_or(true)
        };
        self.shared.update_status(|s| {
            s.permission = permission;
            s.capture_fps = fps;
        })
    }

    pub fn start(&self) -> Result<SessionStatus, String> {
        if !self.settings.get().accepted_fair_play {
            return Err("Please read and accept the fair-play notice before starting a session. Fishy is for study and analysis of positions you are allowed to analyse, never for live rated or competitive games.".to_string());
        }
        let mut slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|h| !h.join.is_finished()) {
            drop(slot);
            return Ok(self.status());
        }
        if let Some(finished) = slot.take() {
            let _ = finished.join.join();
        }

        let (tx, rx) = mpsc::channel::<Cmd>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let ctx = WorkerContext {
            shared: Arc::clone(&self.shared),
            settings: Arc::clone(&self.settings),
            overlay: Arc::clone(&self.overlay),
            data_dir: self.data_dir.clone(),
            app: self.app.clone(),
        };
        // The worker (and its engine/tracker) is built on its own thread so none of
        // those types need to be `Send`; start-up success is reported back here.
        let join = thread::Builder::new()
            .name("fishy-session".into())
            .spawn(move || match guard_result(|| Worker::new(ctx)) {
                Ok(worker) => {
                    let _ = ready_tx.send(Ok(()));
                    worker.run(rx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| format!("Could not start the session thread: {e}"))?;

        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {
                *slot = Some(WorkerHandle { tx, join });
                drop(slot);
                self.overlay.set_session_active(true);
                self.shared.set_board(BoardState::idle());
                let fps = self.settings.get().capture.fps;
                Ok(self.shared.update_status(|s| {
                    s.running = true;
                    s.message =
                        progress::phase_message(SessionPhase::StartingEngine).map(String::from);
                    s.capture_fps = fps;
                    s.phase = SessionPhase::StartingEngine;
                    s.engine = None;
                }))
            }
            Ok(Err(e)) => {
                let _ = join.join();
                self.shared.update_status(|s| {
                    s.running = false;
                    s.message = Some(e.clone());
                    s.phase = SessionPhase::Idle;
                    s.engine = None;
                });
                Err(e)
            }
            Err(_) => {
                let _ = tx.send(Cmd::Stop);
                let msg = "The chess engine did not respond in time.".to_string();
                self.shared.update_status(|s| s.message = Some(msg.clone()));
                Err(msg)
            }
        }
    }

    pub fn stop(&self) -> SessionStatus {
        let handle = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = handle {
            let _ = h.tx.send(Cmd::Stop);
            let _ = h.join.join();
        }
        self.overlay.set_session_active(false);
        self.shared.set_board(BoardState::idle());
        let fps = self.settings.get().capture.fps;
        self.shared.update_status(|s| {
            s.running = false;
            s.capture_fps = fps;
            s.phase = SessionPhase::Idle;
            s.engine = None;
        })
    }

    /// Tell the worker that settings changed (engine options, fps, target, site).
    pub fn settings_changed(&self) {
        let fps = self.settings.get().capture.fps;
        if let Some(h) = self
            .worker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = h.tx.send(Cmd::SettingsChanged);
        } else {
            self.shared.update_status(|s| s.capture_fps = fps);
        }
    }

    fn request(&self, make: impl FnOnce(Reply) -> Cmd) -> Result<BoardState, String> {
        let tx = {
            let slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
            match slot.as_ref() {
                Some(h) if !h.join.is_finished() => h.tx.clone(),
                _ => return Err("Start a session first.".to_string()),
            }
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(make(reply_tx))
            .map_err(|_| "The session is not running.".to_string())?;
        reply_rx
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| "The session did not answer in time.".to_string())?
    }

    pub fn calibrate_start_position(&self) -> Result<BoardState, String> {
        self.request(Cmd::Calibrate)
    }

    pub fn recalibrate_board(&self) -> Result<BoardState, String> {
        self.request(Cmd::Recalibrate)
    }

    pub fn set_known_position(&self, fen: String) -> Result<BoardState, String> {
        self.request(|r| Cmd::SetFen(fen, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, kind: &str, x: i32, y: i32) -> CaptureTarget {
        CaptureTarget {
            id: id.into(),
            kind: kind.into(),
            title: String::new(),
            app: String::new(),
            x,
            y,
            width: 100,
            height: 100,
            scale: 1.0,
        }
    }

    #[test]
    fn primary_is_the_monitor_at_the_origin() {
        let t = vec![
            target("window:1", "window", 0, 0),
            target("monitor:2", "monitor", -1920, 0),
            target("monitor:1", "monitor", 0, 0),
        ];
        assert_eq!(pick_primary(&t).unwrap().id, "monitor:1");
    }

    #[test]
    fn primary_falls_back_to_first_monitor() {
        let t = vec![
            target("window:1", "window", 0, 0),
            target("monitor:5", "monitor", 10, 10),
        ];
        assert_eq!(pick_primary(&t).unwrap().id, "monitor:5");
        assert!(pick_primary(&[target("window:1", "window", 0, 0)]).is_none());
        assert!(pick_primary(&[]).is_none());
    }

    #[test]
    fn explicit_target_is_not_replaced_by_primary() {
        let t = vec![
            target("monitor:1", "monitor", 0, 0),
            target("window:9", "window", 5, 5),
        ];
        assert_eq!(resolve_target(&t, Some("window:9")).unwrap().id, "window:9");
        assert!(resolve_target(&t, Some("window:404")).is_none());
        assert_eq!(resolve_target(&t, None).unwrap().id, "monitor:1");
    }

    #[test]
    fn timestamps_are_strictly_increasing() {
        let mut last = 0;
        for now in [0, 0, 0, 5, 5, 3, 100, 100] {
            let n = next_ts(last, now);
            assert!(n > last);
            last = n;
        }
        assert_eq!(next_ts(10, 50), 50);
        assert_eq!(next_ts(10, 10), 11);
        assert_eq!(next_ts(u64::MAX, 0), u64::MAX);
    }

    #[test]
    fn frame_interval_matches_fps() {
        assert_eq!(frame_interval(8), Duration::from_millis(125));
        assert_eq!(frame_interval(30), Duration::from_millis(33));
        assert_eq!(frame_interval(0), Duration::from_millis(1000));
    }

    #[test]
    fn vision_errors_have_friendly_messages() {
        assert!(describe_vision_error(&VisionError::PermissionDenied).contains("Screen Recording"));
        assert!(
            describe_vision_error(&VisionError::TargetNotFound("window:3".into()))
                .contains("window:3")
        );
    }

    #[test]
    fn engine_settings_map_to_config() {
        let mut s = Settings::default();
        s.engine.depth_limit = Some(20);
        s.engine.skill_level = Some(7);
        s.engine.auto = false;
        let hw = Hardware {
            logical_cores: 10,
            perf_cores: Some(4),
            ram_mb: 16 * 1024,
        };
        let c = engine_config(&s.engine, &hw);
        assert_eq!(
            (
                c.threads,
                c.hash_mb,
                c.multi_pv,
                c.depth_limit,
                c.skill_level
            ),
            (s.engine.threads, 128, 3, Some(20), Some(7))
        );
    }

    #[test]
    fn auto_picks_threads_and_hash_for_the_machine_but_keeps_user_options() {
        let hw = Hardware {
            logical_cores: 10,
            perf_cores: Some(4),
            ram_mb: 16 * 1024,
        };
        let mut s = Settings::default();
        assert!(s.engine.auto, "auto is the default");
        s.engine.threads = 1; // ignored while auto
        s.engine.hash_mb = 16;
        s.engine.multi_pv = 2;
        s.engine.skill_level = Some(5);
        let c = engine_config(&s.engine, &hw);
        assert_eq!((c.threads, c.hash_mb), (6, 512));
        assert_eq!((c.multi_pv, c.skill_level), (2, Some(5)));
        // A small machine gets a small engine.
        let small = Hardware {
            logical_cores: 2,
            perf_cores: None,
            ram_mb: 4096,
        };
        let c = engine_config(&s.engine, &small);
        assert_eq!((c.threads, c.hash_mb), (1, 128));
        // Manual numbers apply once auto is off.
        s.engine.auto = false;
        s.engine.threads = 3;
        s.engine.hash_mb = 64;
        let c = engine_config(&s.engine, &hw);
        assert_eq!((c.threads, c.hash_mb), (3, 64));
    }
}
