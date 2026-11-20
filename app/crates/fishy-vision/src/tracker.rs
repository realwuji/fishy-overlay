//! Position tracking: noisy per-frame placements in, legal game positions out.
//!
//! The tracker keeps a line of legal positions (`entries`, with a `cursor` so
//! that takebacks and analysis-board navigation can step back *and forward*
//! again) and explains every new stable reading as one of, in this order:
//!
//! 1. the current position (nothing happened);
//! 2. one legal move from it (castling, en passant and promotion included -
//!    the move is found by comparing the reading with every legal successor, so
//!    rights and en-passant state always follow from real play);
//! 3. an earlier or later position of the same line (takeback / navigation);
//! 4. the standard start position (new game);
//! 5. two legal moves (a move and the reply between two frames);
//! 6. a transient board (a piece lifted or dragged, however long): ignored,
//!    never adopted;
//! 7. otherwise it counts as *unreconciled*; after a few stable frames the
//!    state becomes `Lost` and after [`RESYNC_FRAMES`] the tracker resyncs:
//!    1-3 legal plies from the current position, 1-2 plies from any earlier
//!    position, finally adoption of the placement itself as a brand new
//!    position (both kings required; side to move from the last-move highlight
//!    (`Reading::mover_hint`) when there is one, otherwise guessed).
//!
//! Orientation: the tracker can notice a turned board by itself (a rotated start position, a reading
//! that is a legal continuation only when rotated), but inside `orient::BoardPipeline` that is locked
//! off (`lock_orientation`) and the orientation decision there takes precedence.
//!
//! Readings are only trusted after two identical consecutive frames, and
//! `'?'` squares are wildcards: they match anything, so an arrow head hiding
//! a square does not freeze tracking. A hidden promotion piece resolves to a
//! queen, the overwhelmingly common choice.

use crate::recognize::{Orientation, Placement, Reading};
use serde::{Deserialize, Serialize};
use shakmaty::{
    CastlingMode, Chess, Color, EnPassantMode, File, Move, Position, Rank, Role, Square, fen::Fen,
};

/// Stable-but-unexplained frames after which the tracker resyncs.
pub const RESYNC_FRAMES: u32 = 6;
/// Frames of an unexplained placement before the state is reported as `Lost`.
const LOST_FRAMES: u32 = 3;
/// Readings below this mean confidence are ignored.
const MIN_CONFIDENCE: f32 = 0.45;
/// Readings with more `?` squares than this are ignored.
const MAX_UNKNOWN: usize = 10;
/// Identical stable frames a two-square "piece in hand" look-alike is tolerated for. A real drag
/// leaves one square empty; a change that also fills a square is a position change.
const LIFT_GHOST_FRAMES: u32 = 40;
/// Identical stable frames a single emptied square is tolerated for (a piece held while thinking).
const LIFT_HOLD_FRAMES: u32 = 1200;
/// Maximum number of positions kept for takebacks.
const MAX_ENTRIES: usize = 800;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrackState {
    /// Waiting for a start position / templates.
    NeedsCalibration,
    /// Placement could not be reconciled with legal play; waiting for a stable reading.
    Lost,
    Tracking,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracked {
    pub state: TrackState,
    /// Full FEN with side to move, castling and en-passant inferred from move history.
    pub fen: Option<String>,
    /// UCI of the last inferred move.
    pub last_move: Option<String>,
    pub orientation: Orientation,
    pub confidence: f32,
}

#[derive(Clone)]
struct Entry {
    pos: Chess,
    placement: Placement,
    last_move: Option<String>,
}

/// Turns noisy per-frame placements into a legal game position. Rejects
/// transient readings (animations, drags), infers moves by diffing against the
/// previous legal position, and resynchronises from a stable placement when lost.
pub struct PositionTracker {
    state: TrackState,
    entries: Vec<Entry>,
    cursor: usize,
    orientation: Orientation,
    confidence: f32,
    pending: Option<Placement>,
    pending_count: u32,
    unmatched: Option<Placement>,
    unreconciled: u32,
    transient: u32,
    /// Consecutive stable frames spent in the "piece in hand" branch for `transient_key`.
    transient_key: Option<Placement>,
    /// Side to move of the last known position (hint when adopting a new one).
    turn_hint: Color,
    /// Colour that just moved according to the last-move highlight of the latest reading.
    mover_hint: Option<Color>,
    /// The orientation is decided elsewhere (see `orient.rs`); never second-guess it.
    orientation_locked: bool,
}

// ---------------------------------------------------------------------------
// placement helpers
// ---------------------------------------------------------------------------

fn square_of(index: usize) -> Square {
    Square::from_coords(
        File::new((index % 8) as u32),
        Rank::new(7 - (index / 8) as u32),
    )
}

fn placement_of(pos: &Chess) -> Placement {
    let board = pos.board();
    let mut p = ['.'; 64];
    for (i, c) in p.iter_mut().enumerate() {
        if let Some(piece) = board.piece_at(square_of(i)) {
            *c = piece.char();
        }
    }
    p
}

/// Number of squares where both sides are known and differ.
fn known_diff(a: &Placement, reading: &Placement) -> usize {
    a.iter()
        .zip(reading)
        .filter(|(x, y)| **y != '?' && x != y)
        .count()
}

fn compatible(a: &Placement, reading: &Placement) -> bool {
    known_diff(a, reading) == 0
}

/// The placement seen from the other side of the board (a 180 degree turn).
fn rotated(p: &Placement) -> Placement {
    let mut r = *p;
    for (i, c) in p.iter().enumerate() {
        r[63 - i] = *c;
    }
    r
}

fn flip(o: Orientation) -> Orientation {
    match o {
        Orientation::White => Orientation::Black,
        Orientation::Black => Orientation::White,
    }
}

fn start_placement() -> Placement {
    placement_of(&Chess::default())
}

fn fen_of(pos: &Chess) -> String {
    Fen::from_position(pos, EnPassantMode::Legal).to_string()
}

fn uci_of(m: &Move) -> String {
    m.to_uci(CastlingMode::Standard).to_string()
}

fn other(c: Color) -> Color {
    match c {
        Color::White => Color::Black,
        Color::Black => Color::White,
    }
}

/// Legal continuations of `pos` of exactly `plies` plies whose resulting
/// placement is compatible with `reading`.
fn search(
    pos: &Chess,
    reading: &Placement,
    plies: usize,
    path: &mut Vec<Move>,
    out: &mut Vec<(Vec<Move>, Chess)>,
) {
    for m in pos.legal_moves() {
        let mut next = pos.clone();
        next.play_unchecked(m);
        let pl = placement_of(&next);
        path.push(m);
        if plies == 1 {
            if compatible(&pl, reading) {
                out.push((path.clone(), next));
            }
        } else if known_diff(&pl, reading) <= 4 * (plies - 1) {
            search(&next, reading, plies - 1, path, out);
        }
        path.pop();
    }
}

/// Resolve several explanations of one reading: promotions to anything but a
/// queen are only chosen when the picture says so (i.e. they are the only match).
fn pick(mut matches: Vec<(Vec<Move>, Chess)>) -> Option<(Vec<Move>, Chess)> {
    match matches.len() {
        0 => None,
        1 => matches.pop(),
        _ => {
            let queenish = |(path, _): &(Vec<Move>, Chess)| {
                path.iter()
                    .all(|m| m.promotion().is_none_or(|r| r == Role::Queen))
            };
            let idx = matches.iter().position(queenish).unwrap_or(0);
            Some(matches.swap_remove(idx))
        }
    }
}

/// Both kings, plausible piece counts: the minimum to adopt a placement.
fn plausible(p: &Placement) -> bool {
    let count = |c: char| p.iter().filter(|&&x| x == c).count();
    count('K') == 1 && count('k') == 1 && count('P') <= 8 && count('p') <= 8 && !p.contains(&'?')
}

fn castling_field(p: &Placement) -> String {
    let mut s = String::new();
    if p[60] == 'K' && p[63] == 'R' {
        s.push('K');
    }
    if p[60] == 'K' && p[56] == 'R' {
        s.push('Q');
    }
    if p[4] == 'k' && p[7] == 'r' {
        s.push('k');
    }
    if p[4] == 'k' && p[0] == 'r' {
        s.push('q');
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

/// Build a position from a bare placement. Castling rights are assumed when
/// king and rook stand on their home squares; the side to move is `hint` unless
/// only the other side makes a legal position.
#[allow(clippy::result_large_err)]
fn position_from_placement(p: &Placement, hint: Color) -> Option<Chess> {
    let mut board = String::new();
    for rank in 0..8 {
        let mut empty = 0;
        for file in 0..8 {
            let c = p[rank * 8 + file];
            if c == '.' {
                empty += 1;
            } else {
                if empty > 0 {
                    board.push_str(&empty.to_string());
                    empty = 0;
                }
                board.push(c);
            }
        }
        if empty > 0 {
            board.push_str(&empty.to_string());
        }
        if rank < 7 {
            board.push('/');
        }
    }
    let castling = castling_field(p);
    let try_turn = |turn: Color| -> Option<Chess> {
        let fen = format!(
            "{board} {} {castling} - 0 1",
            if turn == Color::White { 'w' } else { 'b' }
        );
        let parsed: Fen = fen.parse().ok()?;
        parsed
            .into_position::<Chess>(CastlingMode::Standard)
            .or_else(|e| e.ignore_invalid_castling_rights())
            .ok()
    };
    // A side whose opponent is in check must be the one to move.
    let (first, second) = (hint, other(hint));
    match (try_turn(first), try_turn(second)) {
        (Some(a), Some(b)) => Some(if b.is_check() && !a.is_check() { b } else { a }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// tracker
// ---------------------------------------------------------------------------

impl PositionTracker {
    pub fn new() -> Self {
        PositionTracker {
            state: TrackState::NeedsCalibration,
            entries: Vec::new(),
            cursor: 0,
            orientation: Orientation::White,
            confidence: 0.0,
            pending: None,
            pending_count: 0,
            unmatched: None,
            unreconciled: 0,
            transient: 0,
            transient_key: None,
            turn_hint: Color::White,
            mover_hint: None,
            orientation_locked: false,
        }
    }

    /// Hand the orientation decision to the caller.
    ///
    /// Precedence of the two mechanisms that can notice a turned board: an authority outside the
    /// tracker (manual choice, coordinate labels, structural evidence or calibration, see
    /// `orient::BoardPipeline`) always wins. While locked, the tracker trusts `Reading::orientation`
    /// and never explains a reading by rotating it 180 degrees, so the two cannot fight. Unlocked
    /// (the default, for standalone use) the tracker keeps recognising a start position seen from
    /// black's side and a board that was turned around mid-game on its own.
    pub fn lock_orientation(&mut self, locked: bool) {
        self.orientation_locked = locked;
    }

    fn clear_counters(&mut self) {
        self.pending = None;
        self.pending_count = 0;
        self.unmatched = None;
        self.unreconciled = 0;
        self.transient = 0;
        self.transient_key = None;
    }

    fn install(&mut self, pos: Chess, last_move: Option<String>) {
        self.turn_hint = pos.turn();
        self.entries = vec![Entry {
            placement: placement_of(&pos),
            pos,
            last_move,
        }];
        self.cursor = 0;
        self.state = TrackState::Tracking;
        self.clear_counters();
    }

    /// Start tracking from the standard start position with `white_to_move = true`.
    pub fn reset_to_start(&mut self) {
        self.install(Chess::default(), None);
    }

    /// Start from an arbitrary known FEN (e.g. puzzle/analysis boards).
    #[allow(clippy::result_large_err)]
    pub fn reset_to_fen(&mut self, fen: &str) -> Result<(), String> {
        let parsed: Fen = fen
            .trim()
            .parse()
            .map_err(|e| format!("invalid FEN: {e}"))?;
        let pos: Chess = parsed
            .into_position(CastlingMode::Standard)
            .or_else(|e| e.ignore_too_much_material())
            .or_else(|e| e.ignore_invalid_castling_rights())
            .map_err(|e| format!("illegal position: {e}"))?;
        self.install(pos, None);
        Ok(())
    }

    fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    fn snapshot(&self) -> Tracked {
        let tracking = self.state == TrackState::Tracking;
        let cur = self.current().filter(|_| tracking);
        Tracked {
            state: self.state.clone(),
            fen: cur.map(|e| fen_of(&e.pos)),
            last_move: cur.and_then(|e| e.last_move.clone()),
            orientation: self.orientation,
            confidence: self.confidence,
        }
    }

    /// Append the positions along `path` after the cursor (dropping any redo line).
    fn advance(&mut self, path: &[Move], from: &Chess) {
        self.entries.truncate(self.cursor + 1);
        let mut pos = from.clone();
        for m in path {
            let uci = uci_of(m);
            pos.play_unchecked(*m);
            self.entries.push(Entry {
                placement: placement_of(&pos),
                pos: pos.clone(),
                last_move: Some(uci),
            });
        }
        if self.entries.len() > MAX_ENTRIES {
            let drop = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..drop);
        }
        self.cursor = self.entries.len() - 1;
        self.turn_hint = self.entries[self.cursor].pos.turn();
    }

    fn recovered(&mut self) {
        self.state = TrackState::Tracking;
        self.unmatched = None;
        self.unreconciled = 0;
        self.transient = 0;
        self.transient_key = None;
    }

    /// Nearest other entry (by distance from the cursor) compatible with `p`.
    fn find_entry(&self, p: &Placement) -> Option<usize> {
        (1..self.entries.len()).find_map(|d| {
            let before = self
                .cursor
                .checked_sub(d)
                .filter(|&i| compatible(&self.entries[i].placement, p));
            let after = Some(self.cursor + d)
                .filter(|&i| i < self.entries.len() && compatible(&self.entries[i].placement, p));
            before.or(after)
        })
    }

    fn adopt(&mut self, p: &Placement) -> bool {
        if !plausible(p) {
            return false;
        }
        // Joining a game in progress: the side that did NOT make the highlighted last move is to
        // move. Without a highlight, keep the previous guess (corrected by the first move).
        let turn = self.mover_hint.map_or(self.turn_hint, other);
        match position_from_placement(p, turn) {
            Some(pos) => {
                self.install(pos, None);
                true
            }
            None => false,
        }
    }

    /// Explain `p` as nothing, one move, a takeback/navigation, a new game or two moves.
    /// Applies the explanation and returns `true` when one was found.
    fn explain(&mut self, p: &Placement, cur: &Entry) -> bool {
        if compatible(&cur.placement, p) {
            self.recovered();
            return true;
        }
        if !p.contains(&'?') && *p == start_placement() {
            self.reset_to_start();
            return true;
        }
        // One legal move.
        let mut found = Vec::new();
        search(&cur.pos, p, 1, &mut Vec::new(), &mut found);
        if let Some((path, _)) = pick(found) {
            self.advance(&path, &cur.pos);
            self.recovered();
            return true;
        }
        // Takeback / navigation along the known line.
        if let Some(i) = self.find_entry(p) {
            self.cursor = i;
            self.turn_hint = self.entries[i].pos.turn();
            self.recovered();
            return true;
        }
        // A move plus the reply between two frames.
        let mut found = Vec::new();
        search(&cur.pos, p, 2, &mut Vec::new(), &mut found);
        if let Some((path, _)) = pick(found) {
            self.advance(&path, &cur.pos);
            self.recovered();
            return true;
        }
        false
    }

    /// Process a reading that has been identical for at least two frames.
    fn process_stable(&mut self, p: &Placement) {
        let Some(cur) = self.current().cloned() else {
            // Not calibrated: join the game wherever it is. A start position seen from black's
            // side is recognised as such instead of being adopted upside down.
            if !self.orientation_locked && !p.contains(&'?') && *p == rotated(&start_placement()) {
                self.orientation = flip(self.orientation);
                self.pending = None;
                self.pending_count = 0;
                self.reset_to_start();
            } else {
                self.adopt(p);
            }
            return;
        };
        if self.explain(p, &cur) {
            return;
        }
        // The board may have been turned around (a new game as the other colour): the same
        // reading seen from the other side must then be a legal continuation.
        if !self.orientation_locked {
            let r = rotated(p);
            if self.explain(&r, &cur) {
                self.orientation = flip(self.orientation);
                self.pending = None;
                self.pending_count = 0;
                return;
            }
        }
        // The side to move was guessed wrong when the game was joined (or the same side moved
        // twice on an analysis board): a legal move by the other side explains the reading.
        if let Ok(swapped) = cur.pos.clone().swap_turn() {
            let mut found = Vec::new();
            search(&swapped, p, 1, &mut Vec::new(), &mut found);
            if let Some((path, _)) = pick(found) {
                self.install(swapped.clone(), None);
                self.advance(&path, &swapped);
                self.recovered();
                return;
            }
        }
        // Piece in hand (lifted, dragged, sliding): not a new position.
        let diff: Vec<usize> = (0..64)
            .filter(|&i| p[i] != '?' && cur.placement[i] != p[i])
            .collect();
        let emptied = diff.iter().any(|&i| p[i] == '.' && cur.placement[i] != '.');
        if diff.len() <= 2 && emptied {
            // Holding a piece while thinking can take long, but a reading that also fills a
            // square is only a drag ghost for a short while; both are bounded so that a changed
            // board can never leave a stale position reported as `Tracking` forever.
            if self.transient_key.as_ref() == Some(p) {
                self.transient = self.transient.saturating_add(1);
            } else {
                self.transient_key = Some(*p);
                self.transient = 1;
            }
            let limit = if diff.len() == 1 {
                LIFT_HOLD_FRAMES
            } else {
                LIFT_GHOST_FRAMES
            };
            if self.transient <= limit {
                return;
            }
        }
        // Unexplained.
        if self.unmatched.as_ref() == Some(p) {
            self.unreconciled += 1;
        } else {
            self.unmatched = Some(*p);
            self.unreconciled = 1;
        }
        if self.unreconciled >= LOST_FRAMES {
            self.state = TrackState::Lost;
        }
        if self.unreconciled >= RESYNC_FRAMES {
            self.resync(p);
        }
    }

    /// Try hard to explain `p`, then fall back to adopting it.
    fn resync(&mut self, p: &Placement) {
        // 3 plies from the current position.
        if let Some(cur) = self.current().cloned() {
            let mut found = Vec::new();
            search(&cur.pos, p, 3, &mut Vec::new(), &mut found);
            if let Some((path, _)) = pick(found) {
                self.advance(&path, &cur.pos);
                self.recovered();
                return;
            }
            // A deviation from an earlier point of the game (analysis board).
            for i in (0..self.entries.len()).rev() {
                if i == self.cursor {
                    continue;
                }
                let base = self.entries[i].clone();
                let mut found = Vec::new();
                search(&base.pos, p, 1, &mut Vec::new(), &mut found);
                if let Some((path, _)) = pick(found) {
                    self.cursor = i;
                    self.advance(&path, &base.pos);
                    self.recovered();
                    return;
                }
            }
        }
        // `install` (inside adopt) sets Tracking and clears the counters.
        self.adopt(p);
    }

    pub fn update(&mut self, reading: &Reading) -> Tracked {
        self.orientation = reading.orientation;
        self.confidence = reading.confidence;
        self.mover_hint = match reading.mover_hint {
            Some('w') => Some(Color::White),
            Some('b') => Some(Color::Black),
            _ => None,
        };
        let unknown = reading.placement.iter().filter(|&&c| c == '?').count();
        let count = |f: fn(&char) -> bool| reading.placement.iter().filter(|c| f(c)).count();
        // More pieces of one colour than a game can contain: the colours were misread.
        let impossible = count(char::is_ascii_uppercase) > 16
            || count(char::is_ascii_lowercase) > 16
            || reading.placement.iter().filter(|&&c| c == 'P').count() > 8
            || reading.placement.iter().filter(|&&c| c == 'p').count() > 8;
        if reading.confidence < MIN_CONFIDENCE || unknown > MAX_UNKNOWN || impossible {
            self.pending = None;
            self.pending_count = 0;
            return self.snapshot();
        }
        if self.pending.as_ref() == Some(&reading.placement) {
            self.pending_count = self.pending_count.saturating_add(1);
        } else {
            self.pending = Some(reading.placement);
            self.pending_count = 1;
        }
        if self.pending_count >= 2 {
            self.process_stable(&reading.placement);
        }
        self.snapshot()
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}
