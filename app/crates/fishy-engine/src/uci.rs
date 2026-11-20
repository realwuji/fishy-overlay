//! Minimal, dependency-free UCI driver for Stockfish.
//!
//! Design
//! ------
//! * One child process, three threads: a *reader* that only moves stdout lines
//!   into a channel (so the engine can never block on a full pipe), a
//!   *processor* that parses lines, maintains the search state and delivers
//!   [`EngineUpdate`]s, and the caller's thread(s) that issue commands.
//! * **Stale output suppression.** Every `go` is answered by exactly one
//!   `bestmove`. When a new search is requested while one is running we send
//!   `stop` and remember that one extra `bestmove` has to be swallowed
//!   (`State::skip`). The processor discards *everything* until that
//!   `bestmove` has passed, so nothing from the old search can reach
//!   `on_update` for the new position. Because Stockfish handles commands
//!   strictly in order (its `position`/`go` handlers wait for a stopped search
//!   to finish), the new `position`/`go` can be written immediately.
//! * **Throttling.** Complete MultiPV snapshots are delivered at most every
//!   [`MIN_UPDATE_INTERVAL`]; a snapshot held back by the throttle is flushed by
//!   the processor's timer so the last update of a burst is never lost.
//! * `on_update` runs on the processor thread. It must not block for long and,
//!   although re-entrant calls into [`Engine`] from the callback are supported,
//!   they should be avoided.

use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::{CastlingMode, Chess, Position};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

/// Minimum spacing between progress updates (~20 per second).
const MIN_UPDATE_INTERVAL: Duration = Duration::from_millis(50);
const UCI_TIMEOUT: Duration = Duration::from_secs(10);
/// Loading the NNUE network and allocating the hash can take a while.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SKILL_LEVEL: u32 = 20;
/// Depth of the throw-away warm-up search.
const WARM_UP_DEPTH: u32 = 14;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine binary not found at {0}")]
    NotFound(String),
    #[error("engine io error: {0}")]
    Io(String),
    #[error("engine protocol error: {0}")]
    Protocol(String),
    #[error("invalid fen: {0}")]
    InvalidFen(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    pub threads: u32,
    pub hash_mb: u32,
    pub multi_pv: u32,
    /// Stop each search at this depth (None = infinite until stop/new position).
    pub depth_limit: Option<u32>,
    pub move_time_ms: Option<u32>,
    /// 0..=20, None = full strength (never set `Skill Level` then).
    pub skill_level: Option<u32>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            threads: 2,
            hash_mb: 128,
            multi_pv: 3,
            depth_limit: None,
            move_time_ms: None,
            skill_level: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Score {
    /// Centipawns from the side-to-move's point of view (raw UCI).
    Cp(i32),
    /// Mate in N plies-moves from the side-to-move's point of view (negative = being mated).
    Mate(i32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawLine {
    pub multipv: u32,
    pub depth: u32,
    pub seldepth: u32,
    pub score: Score,
    pub nodes: u64,
    pub nps: u64,
    /// Principal variation in UCI long algebraic ("e2e4", "e7e8q").
    pub pv: Vec<String>,
    /// Win/draw/loss chances in per-mille from the side-to-move's point of view
    /// (`UCI_ShowWDL`); `None` when the engine did not report them.
    #[serde(default)]
    pub wdl: Option<[u32; 3]>,
}

/// One snapshot of the current search. Emitted every time all MultiPV lines at
/// the newest completed depth are available, and once more with `done = true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineUpdate {
    pub fen: String,
    pub lines: Vec<RawLine>,
    pub depth: u32,
    pub nps: u64,
    pub done: bool,
}

/// Whether a reported score is exact or only a search-window bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Exact,
    /// `lowerbound`: the true score is at least this.
    Lower,
    /// `upperbound`: the true score is at most this.
    Upper,
}

/// A parsed `info ... pv ...` line together with its score bound.
#[derive(Debug, Clone)]
pub struct ParsedInfo {
    pub line: RawLine,
    pub bound: Bound,
}

/// Parse a UCI `info` line. Returns `None` for lines that do not describe a
/// scored principal variation (`info string`, `info currmove`, `info depth 5`
/// without score, ...).
pub fn parse_info_line(text: &str) -> Option<ParsedInfo> {
    let mut it = text.split_ascii_whitespace();
    if it.next()? != "info" {
        return None;
    }
    let mut line = RawLine {
        multipv: 1,
        depth: 0,
        seldepth: 0,
        score: Score::Cp(0),
        nodes: 0,
        nps: 0,
        pv: Vec::new(),
        wdl: None,
    };
    let mut bound = Bound::Exact;
    let mut have_score = false;
    let mut have_depth = false;
    while let Some(tok) = it.next() {
        match tok {
            "string" => return None,
            "depth" => {
                line.depth = it.next()?.parse().ok()?;
                have_depth = true;
            }
            "seldepth" => line.seldepth = it.next()?.parse().ok()?,
            "multipv" => line.multipv = it.next()?.parse().ok()?,
            "nodes" => line.nodes = it.next()?.parse().ok()?,
            "nps" => line.nps = it.next()?.parse().ok()?,
            "score" => {
                let kind = it.next()?;
                let value: i32 = it.next()?.parse().ok()?;
                line.score = match kind {
                    "cp" => Score::Cp(value),
                    "mate" => Score::Mate(value),
                    _ => return None,
                };
                have_score = true;
            }
            "wdl" => {
                let w: u32 = it.next()?.parse().ok()?;
                let d: u32 = it.next()?.parse().ok()?;
                let l: u32 = it.next()?.parse().ok()?;
                line.wdl = Some([w, d, l]);
            }
            "lowerbound" => bound = Bound::Lower,
            "upperbound" => bound = Bound::Upper,
            "pv" => {
                line.pv = it.by_ref().map(str::to_owned).collect();
                break;
            }
            // Fields we do not need; their value is a single token (`refutation`/`currline` lists are not scored lines).
            "currmove" | "currmovenumber" | "hashfull" | "tbhits" | "time" | "cpuload" => {
                it.next();
            }
            "refutation" | "currline" => return None,
            _ => {}
        }
    }
    if !have_score || !have_depth || line.pv.is_empty() || line.multipv == 0 {
        return None;
    }
    Some(ParsedInfo { line, bound })
}

/// Parse and validate a FEN into a position. Tolerates a FEN whose en-passant
/// square is not actually capturable (trackers often emit those) and positions
/// with unusual material, but rejects anything that is not a playable position.
pub(crate) fn parse_position(fen: &str) -> Result<Chess, EngineError> {
    let parsed: Fen = fen
        .trim()
        .parse()
        .map_err(|e| EngineError::InvalidFen(format!("{fen}: {e}")))?;
    let reject = |e: &dyn std::fmt::Display| EngineError::InvalidFen(format!("{fen}: {e}"));
    match parsed.into_position::<Chess>(CastlingMode::Standard) {
        Ok(pos) => Ok(pos),
        Err(e) => {
            // Trackers often emit an en-passant square nobody can capture on; that is harmless.
            let e = match e.ignore_invalid_ep_square() {
                Ok(pos) => return Ok(pos),
                Err(e) => e,
            };
            match e.ignore_too_much_material() {
                Ok(pos) => Ok(pos),
                Err(e) => Err(reject(&e)),
            }
        }
    }
}

/// Mutable search bookkeeping shared by the command side and the processor.
struct State {
    cfg: EngineConfig,
    /// Position of the current/last search.
    fen: Option<String>,
    /// MultiPV value the engine is currently set to (0 = unknown, force a resend).
    sent_multi_pv: u32,
    sent_threads: u32,
    sent_hash: u32,
    sent_skill: Option<u32>,
    sent_wdl: bool,
    /// MultiPV lines expected for the current search (clamped to legal moves).
    expected_lines: u32,
    /// A `go` has been issued and its `bestmove` has not been seen yet.
    active: bool,
    /// Number of stale `bestmove`s still to be swallowed.
    skip: u32,
    lines: BTreeMap<u32, RawLine>,
    /// A complete snapshot is waiting for the throttle.
    pending: bool,
    last_emit: Instant,
    /// Engine name from `id name`.
    name: Option<String>,
}

type Callback = Box<dyn Fn(EngineUpdate) + Send>;

struct Shared {
    state: Mutex<State>,
    stdin: Mutex<ChildStdin>,
    cb: Mutex<Callback>,
    alive: AtomicBool,
    processor: OnceLock<ThreadId>,
    /// Counters bumped by the processor: `readyok`s seen, stale `bestmove`s swallowed.
    signals: Mutex<Signals>,
    signals_cv: Condvar,
}

#[derive(Default, Clone, Copy)]
struct Signals {
    ready: u64,
    swallowed: u64,
}

/// Static facts about the running engine, for status displays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub name: Option<String>,
    pub threads: u32,
    pub hash_mb: u32,
    pub multi_pv: u32,
    pub ready: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn send(&self, cmd: &str) -> Result<(), EngineError> {
        let mut w = lock(&self.stdin);
        let res = w
            .write_all(cmd.as_bytes())
            .and_then(|_| w.write_all(b"\n"))
            .and_then(|_| w.flush());
        res.map_err(|e| {
            self.alive.store(false, Ordering::SeqCst);
            EngineError::Io(format!("write to engine failed: {e}"))
        })
    }

    /// Run `f` against the state and deliver the resulting update, if any.
    /// The callback lock is taken first so deliveries are serialised and so
    /// [`Shared::barrier`] can wait for an in-flight delivery.
    fn deliver(&self, f: impl FnOnce(&mut State) -> Option<EngineUpdate>) {
        let cb = lock(&self.cb);
        let update = f(&mut lock(&self.state));
        if let Some(u) = update {
            cb(u);
        }
    }

    fn bump(&self, f: impl FnOnce(&mut Signals)) {
        f(&mut lock(&self.signals));
        self.signals_cv.notify_all();
    }

    /// Block until `pred` holds for the signal counters or `timeout` passes.
    fn wait_signals(&self, timeout: Duration, pred: impl Fn(&Signals) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = lock(&self.signals);
        while !pred(&g) {
            if !self.alive.load(Ordering::SeqCst) {
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let (ng, _) = self
                .signals_cv
                .wait_timeout(g, left.min(Duration::from_millis(200)))
                .unwrap_or_else(|p| p.into_inner());
            g = ng;
        }
        true
    }

    /// Wait until any delivery that started before the caller changed the state has finished.
    fn barrier(&self) {
        if self.processor.get() != Some(&thread::current().id()) {
            drop(lock(&self.cb));
        }
    }
}

fn snapshot(st: &State, done: bool) -> Option<EngineUpdate> {
    let fen = st.fen.clone()?;
    let lines: Vec<RawLine> = st.lines.values().cloned().collect();
    let depth = lines.iter().map(|l| l.depth).max().unwrap_or(0);
    let nps = lines.iter().map(|l| l.nps).max().unwrap_or(0);
    Some(EngineUpdate {
        fen,
        lines,
        depth,
        nps,
        done,
    })
}

fn handle_info(shared: &Shared, text: &str) {
    let Some(info) = parse_info_line(text) else {
        return;
    };
    if info.bound != Bound::Exact {
        // Aspiration-window fail-high/low: the line is not a settled result yet.
        return;
    }
    shared.deliver(|st| {
        if st.skip > 0 || !st.active || info.line.multipv > st.expected_lines {
            return None;
        }
        let multipv = info.line.multipv;
        st.lines.insert(multipv, info.line);
        if multipv == st.expected_lines {
            let depth = st.lines[&multipv].depth;
            let complete =
                (1..=st.expected_lines).all(|k| st.lines.get(&k).is_some_and(|l| l.depth == depth));
            st.pending = complete;
        }
        take_pending(st)
    });
}

fn take_pending(st: &mut State) -> Option<EngineUpdate> {
    if st.pending && st.last_emit.elapsed() >= MIN_UPDATE_INTERVAL {
        st.pending = false;
        st.last_emit = Instant::now();
        return snapshot(st, false);
    }
    None
}

fn handle_bestmove(shared: &Shared) {
    let mut swallowed = false;
    shared.deliver(|st| {
        if st.skip > 0 {
            st.skip -= 1;
            swallowed = true;
            return None;
        }
        if !st.active {
            return None;
        }
        st.active = false;
        st.pending = false;
        st.last_emit = Instant::now();
        snapshot(st, true)
    });
    if swallowed {
        shared.bump(|s| s.swallowed += 1);
    }
}

fn processor_loop(shared: Arc<Shared>, rx: Receiver<String>) {
    let _ = shared.processor.set(thread::current().id());
    loop {
        match rx.recv_timeout(MIN_UPDATE_INTERVAL / 2) {
            Ok(line) => {
                if line.starts_with("info") {
                    handle_info(&shared, &line);
                } else if line.starts_with("bestmove") {
                    handle_bestmove(&shared);
                } else if line.trim() == "readyok" {
                    shared.bump(|s| s.ready += 1);
                }
            }
            Err(RecvTimeoutError::Timeout) => shared.deliver(take_pending),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    shared.alive.store(false, Ordering::SeqCst);
}

fn wait_for(rx: &Receiver<String>, token: &str, timeout: Duration) -> Result<(), EngineError> {
    wait_for_collect(rx, token, timeout, |_| {})
}

/// Like [`wait_for`], handing every line seen before `token` to `on_line`.
fn wait_for_collect(
    rx: &Receiver<String>,
    token: &str,
    timeout: Duration,
    mut on_line: impl FnMut(&str),
) -> Result<(), EngineError> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(l) if l.trim() == token => return Ok(()),
            Ok(l) => on_line(&l),
            Err(RecvTimeoutError::Timeout) => {
                return Err(EngineError::Protocol(format!(
                    "timed out waiting for `{token}`"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(EngineError::Protocol(format!(
                    "engine exited before sending `{token}`"
                )));
            }
        }
    }
}

fn spawn_reader(stdout: std::process::ChildStdout) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("fishy-uci-reader".into())
        .spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut buf = Vec::with_capacity(512);
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let s = String::from_utf8_lossy(&buf);
                        if tx.send(s.trim_end().to_owned()).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .expect("spawning the reader thread");
    rx
}

/// Handle to a running Stockfish process.
pub struct Engine {
    shared: Arc<Shared>,
    child: Mutex<Option<Child>>,
}

impl Engine {
    /// Spawn the binary, perform the UCI handshake, apply `cfg`.
    /// `on_update` is called from an internal processor thread.
    pub fn spawn(
        path: &Path,
        cfg: EngineConfig,
        on_update: impl Fn(EngineUpdate) + Send + 'static,
    ) -> Result<Engine, EngineError> {
        if !path.exists() {
            return Err(EngineError::NotFound(path.display().to_string()));
        }
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: do not flash a console window for the sidecar.
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| EngineError::Io(format!("cannot start {}: {e}", path.display())))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EngineError::Io("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Io("no stdout".into()))?;
        let rx = spawn_reader(stdout);

        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                cfg: cfg.clone(),
                fen: None,
                sent_multi_pv: 0,
                sent_threads: 0,
                sent_hash: 0,
                sent_skill: None,
                sent_wdl: false,
                expected_lines: 1,
                active: false,
                skip: 0,
                lines: BTreeMap::new(),
                pending: false,
                last_emit: Instant::now(),
                name: None,
            }),
            stdin: Mutex::new(stdin),
            cb: Mutex::new(Box::new(on_update)),
            alive: AtomicBool::new(true),
            processor: OnceLock::new(),
            signals: Mutex::new(Signals::default()),
            signals_cv: Condvar::new(),
        });
        let engine = Engine {
            shared: shared.clone(),
            child: Mutex::new(Some(child)),
        };

        // Handshake (errors drop `engine`, which kills the child).
        shared.send("uci")?;
        let mut name = None;
        wait_for_collect(&rx, "uciok", UCI_TIMEOUT, |l| {
            if let Some(n) = l.strip_prefix("id name ") {
                name = Some(n.trim().to_owned());
            }
        })?;
        {
            let mut st = lock(&shared.state);
            st.name = name;
            apply_options(&shared, &mut st, &cfg)?;
        }
        shared.send("isready")?;
        wait_for(&rx, "readyok", READY_TIMEOUT)?;

        let proc_shared = shared.clone();
        thread::Builder::new()
            .name("fishy-uci-processor".into())
            .spawn(move || processor_loop(proc_shared, rx))
            .map_err(|e| EngineError::Io(e.to_string()))?;
        Ok(engine)
    }

    /// Name, effective options and liveness of the engine (for status displays).
    pub fn info(&self) -> EngineInfo {
        let ready = self.is_alive();
        let st = lock(&self.shared.state);
        EngineInfo {
            name: st.name.clone(),
            threads: st.sent_threads.max(1),
            hash_mb: st.sent_hash.max(1),
            multi_pv: st.cfg.multi_pv,
            ready,
        }
    }

    /// Reset for a genuinely new game: `ucinewgame` (clears the hash table and
    /// history heuristics) followed by `isready`. Blocks until `readyok`.
    /// [`Engine::analyze`] deliberately never does this, so moves within one game
    /// keep reusing the transposition table.
    pub fn new_game(&self) -> Result<(), EngineError> {
        self.ensure_alive()?;
        let target = {
            let mut st = lock(&self.shared.state);
            if st.active {
                st.skip += 1;
                st.active = false;
                st.pending = false;
                self.shared.send("stop")?;
            }
            self.shared.send("ucinewgame")?;
            self.shared.send("isready")?;
            lock(&self.shared.signals).ready + 1
        };
        self.shared.barrier();
        if self
            .shared
            .wait_signals(READY_TIMEOUT, |s| s.ready >= target)
        {
            Ok(())
        } else {
            Err(EngineError::Protocol(
                "timed out waiting for `readyok`".into(),
            ))
        }
    }

    /// Run a short throw-away search so threads, the network and the hash pages
    /// are hot before the first real position. Silent (no updates are delivered)
    /// and a no-op while a search is running. Returns how long it took.
    pub fn warm_up(&self) -> Result<Duration, EngineError> {
        self.ensure_alive()?;
        let t0 = Instant::now();
        let target = {
            let mut st = lock(&self.shared.state);
            if st.active {
                return Ok(Duration::ZERO);
            }
            // The throw-away search's output is discarded exactly like a stale search's.
            st.skip += 1;
            self.shared.send("position startpos")?;
            self.shared.send(&format!("go depth {WARM_UP_DEPTH}"))?;
            // Wait for every outstanding stale `bestmove`, including ones from earlier searches.
            lock(&self.shared.signals).swallowed + u64::from(st.skip)
        };
        if self
            .shared
            .wait_signals(READY_TIMEOUT, |s| s.swallowed >= target)
        {
            Ok(t0.elapsed())
        } else {
            Err(EngineError::Protocol(
                "warm-up search did not finish".into(),
            ))
        }
    }

    /// True while the engine process is running and its pipes are open.
    pub fn is_alive(&self) -> bool {
        if !self.shared.alive.load(Ordering::SeqCst) {
            return false;
        }
        match lock(&self.child).as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Re-apply options (restarts the current search if needed).
    pub fn configure(&self, cfg: EngineConfig) -> Result<(), EngineError> {
        self.ensure_alive()?;
        let mut st = lock(&self.shared.state);
        if st.cfg == cfg {
            return Ok(());
        }
        let was_active = st.active;
        if was_active {
            st.skip += 1;
            st.active = false;
            self.shared.send("stop")?;
        }
        st.pending = false;
        st.cfg = cfg.clone();
        apply_options(&self.shared, &mut st, &cfg)?;
        self.shared.send("isready")?;
        let restart = st.fen.clone().filter(|_| was_active);
        if let Some(fen) = restart {
            let pos = parse_position(&fen)?;
            self.begin_search(&mut st, &fen, &pos)?;
        }
        drop(st);
        self.shared.barrier();
        Ok(())
    }

    /// Stop any running search and start analysing `fen`. Stale output from the
    /// previous search must never be delivered for the new position.
    pub fn analyze(&self, fen: &str) -> Result<(), EngineError> {
        let pos = parse_position(fen)?;
        self.ensure_alive()?;
        let mut st = lock(&self.shared.state);
        if st.active {
            st.skip += 1;
            st.active = false;
            self.shared.send("stop")?;
        }
        let started = self.begin_search(&mut st, fen, &pos);
        drop(st);
        // After this returns no update for the previous position can still be in flight.
        self.shared.barrier();
        let no_moves = started?;
        if no_moves {
            // Checkmate/stalemate: nothing to search; report an empty, finished snapshot.
            self.shared.deliver(|st| {
                // `skip` may still be non-zero (the stale `bestmove` of the previous search has not
                // been read yet). That is fine: stale info is already discarded by `skip`, and
                // the `fen` check keeps this snapshot from outliving a newer `analyze`.
                (st.fen.as_deref() == Some(fen))
                    .then(|| snapshot(st, true))
                    .flatten()
            });
        }
        Ok(())
    }

    pub fn stop(&self) {
        let st = lock(&self.shared.state);
        if st.active {
            // The engine answers with `bestmove`, which produces the final `done` update.
            let _ = self.shared.send("stop");
        }
    }

    /// Send `quit` and reap the child. Also invoked on Drop.
    pub fn quit(&self) {
        let Some(mut child) = lock(&self.child).take() else {
            return;
        };
        let _ = self.shared.send("quit");
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
        self.shared.alive.store(false, Ordering::SeqCst);
    }

    fn ensure_alive(&self) -> Result<(), EngineError> {
        if self.is_alive() {
            Ok(())
        } else {
            Err(EngineError::Io("engine process is not running".into()))
        }
    }

    /// Reset search bookkeeping and send `position`/`go`. Caller holds the state lock and has
    /// already stopped/accounted for any previous search. Returns `true` when the position has no
    /// legal moves (nothing was started).
    fn begin_search(&self, st: &mut State, fen: &str, pos: &Chess) -> Result<bool, EngineError> {
        st.lines.clear();
        st.pending = false;
        st.fen = Some(fen.to_owned());
        let legal = u32::try_from(pos.legal_moves().len()).unwrap_or(u32::MAX);
        if legal == 0 {
            st.expected_lines = 0;
            st.active = false;
            return Ok(true);
        }
        let multi_pv = st.cfg.multi_pv.clamp(1, legal);
        st.expected_lines = multi_pv;
        if st.sent_multi_pv != multi_pv {
            self.shared
                .send(&format!("setoption name MultiPV value {multi_pv}"))?;
            st.sent_multi_pv = multi_pv;
        }
        self.shared.send(&format!("position fen {}", fen.trim()))?;
        let go = match (st.cfg.depth_limit, st.cfg.move_time_ms) {
            (None, None) => "go infinite".to_owned(),
            (Some(d), None) => format!("go depth {d}"),
            (None, Some(t)) => format!("go movetime {t}"),
            (Some(d), Some(t)) => format!("go depth {d} movetime {t}"),
        };
        self.shared.send(&go)?;
        st.active = true;
        Ok(false)
    }
}

/// Send only the options whose value differs from what the engine already has.
fn apply_options(shared: &Shared, st: &mut State, cfg: &EngineConfig) -> Result<(), EngineError> {
    let threads = cfg.threads.max(1);
    if st.sent_threads != threads {
        shared.send(&format!("setoption name Threads value {threads}"))?;
        st.sent_threads = threads;
    }
    let hash = cfg.hash_mb.max(1);
    if st.sent_hash != hash {
        shared.send(&format!("setoption name Hash value {hash}"))?;
        st.sent_hash = hash;
    }
    let skill = cfg.skill_level.map(|l| l.min(20));
    if skill != st.sent_skill {
        // Going back to full strength must reset the option, otherwise it would stay weakened.
        shared.send(&format!(
            "setoption name Skill Level value {}",
            skill.unwrap_or(DEFAULT_SKILL_LEVEL)
        ))?;
        st.sent_skill = skill;
    }
    if !st.sent_wdl {
        shared.send("setoption name UCI_ShowWDL value true")?;
        st.sent_wdl = true;
    }
    st.sent_multi_pv = 0; // MultiPV is (re)sent per search after clamping to the legal move count
    Ok(())
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cp_line() {
        let p = parse_info_line(
            "info depth 12 seldepth 18 multipv 2 score cp -34 nodes 123456 nps 1500000 hashfull 10 tbhits 0 time 82 pv e7e5 g1f3 b8c6",
        )
        .unwrap();
        assert_eq!(p.bound, Bound::Exact);
        assert_eq!(p.line.depth, 12);
        assert_eq!(p.line.seldepth, 18);
        assert_eq!(p.line.multipv, 2);
        assert_eq!(p.line.score, Score::Cp(-34));
        assert_eq!(p.line.nodes, 123_456);
        assert_eq!(p.line.nps, 1_500_000);
        assert_eq!(p.line.pv, ["e7e5", "g1f3", "b8c6"]);
    }

    #[test]
    fn parses_mate_and_default_multipv() {
        let p = parse_info_line("info depth 5 seldepth 5 score mate -3 nodes 10 nps 100 pv a1a2")
            .unwrap();
        assert_eq!(p.line.multipv, 1);
        assert_eq!(p.line.score, Score::Mate(-3));
    }

    #[test]
    fn parses_bounds() {
        let lo =
            parse_info_line("info depth 9 multipv 1 score cp 80 lowerbound nodes 5 nps 5 pv e2e4")
                .unwrap();
        assert_eq!(lo.bound, Bound::Lower);
        let hi =
            parse_info_line("info depth 9 multipv 1 score cp -80 upperbound nodes 5 nps 5 pv e2e4")
                .unwrap();
        assert_eq!(hi.bound, Bound::Upper);
    }

    #[test]
    fn ignores_non_pv_lines() {
        assert!(parse_info_line("info string NNUE evaluation using nn.nnue").is_none());
        assert!(parse_info_line("info depth 3 currmove e2e4 currmovenumber 1").is_none());
        assert!(
            parse_info_line("info depth 3 seldepth 3 multipv 1 nodes 3 nps 3 pv e2e4").is_none(),
            "no score"
        );
        assert!(parse_info_line("bestmove e2e4").is_none());
        assert!(
            parse_info_line("info depth 3 score cp 5 nodes 3").is_none(),
            "no pv"
        );
    }

    #[test]
    fn fen_validation() {
        assert!(parse_position("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").is_ok());
        assert!(matches!(
            parse_position("not a fen"),
            Err(EngineError::InvalidFen(_))
        ));
        // No white king.
        assert!(matches!(
            parse_position("8/8/8/8/8/8/8/k7 w - - 0 1"),
            Err(EngineError::InvalidFen(_))
        ));
        // Uncapturable en-passant square is tolerated.
        assert!(
            parse_position("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1").is_ok()
        );
    }

    #[test]
    fn parses_wdl() {
        let p = parse_info_line(
            "info depth 10 seldepth 12 multipv 1 score cp 20 wdl 120 850 30 nodes 5 nps 5 pv e2e4",
        )
        .unwrap();
        assert_eq!(p.line.wdl, Some([120, 850, 30]));
        let p = parse_info_line("info depth 10 score cp 20 nodes 5 nps 5 pv e2e4").unwrap();
        assert_eq!(p.line.wdl, None);
    }

    #[test]
    fn missing_binary_is_reported() {
        let r = Engine::spawn(
            Path::new("/definitely/not/here/stockfish"),
            EngineConfig::default(),
            |_| {},
        );
        assert!(matches!(r, Err(EngineError::NotFound(_))));
    }
}
