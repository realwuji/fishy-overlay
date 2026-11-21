//! Deciding which colour is at the bottom of the screen, and never trusting a default.
//!
//! Reading a board with the wrong orientation does not fail loudly: the placement is simply
//! rotated by 180 degrees (`RNBKQBNR/...`) and, being a legal-looking position, it would be
//! tracked with full confidence. So the orientation is an explicit decision with a source:
//!
//! 1. **Manual** (`OrientationMode::White` / `Black`, the `capture.orientation` setting): final.
//! 2. **Labels**: chess.com's in-board coordinates, [`detect_orientation`]. Authoritative.
//! 3. **Structure**: when no label is readable (a decision that came from labels goes stale after
//!    two checks without a readable label and then yields to a clearly contradicting structure), the board is read both ways and the orientation
//!    under which the position is legal and *natural* wins ([`structural_orientation`]): kings and
//!    pawns sane, side not to move not in check, last move plausible for the piece that made it
//!    (pawns never retreat), kings and rooks on their home squares, pawns not advanced as if the
//!    game had been played backwards. If both fit, nothing is decided.
//! 4. **Calibration**: the start position the user showed to `learn_from_start`.
//!
//! While no orientation is known nothing is tracked: [`BoardPipeline`] reports `Lost` with the
//! status [`MSG_CANT_TELL`]. A decision is re-checked every [`RECHECK_FRAMES`] frames and whenever
//! the board is re-detected, so a new game as the other colour flips it (after the same answer
//! twice in a row, so a transient misread cannot flip a good decision). A change of orientation
//! starts a fresh tracker.
//!
//! # Precedence over the tracker's own rotation logic
//!
//! `PositionTracker` can also notice a turned board (a rotated start position, or a reading that
//! is a legal continuation only when rotated). Run inside [`BoardPipeline`] the tracker is
//! *locked* ([`PositionTracker::lock_orientation`]): this module is the single authority and the
//! tracker trusts `Reading::orientation`. Used on its own (tests, other callers) the tracker keeps
//! its behaviour unchanged.

use crate::{
    capture::Frame,
    detect::{BoardRect, Site},
    labels::detect_orientation,
    recognize::{Orientation, Placement, Reading, Recognizer, plausible_move},
    tracker::{PositionTracker, TrackState, Tracked},
};
use serde::{Deserialize, Serialize};
use shakmaty::{CastlingMode, Chess, Position, fen::Fen};

/// Status shown while the orientation is undecided (both fit, or none could be established).
pub const MSG_CANT_TELL: &str =
    "Can't tell which side is at the bottom - press Calibrate or choose orientation.";

/// Frames between orientation re-checks once decided (about 2 s at 8 fps).
pub const RECHECK_FRAMES: u32 = 16;
/// Minimum naturalness advantage that lets the structure decide.
const STRUCTURE_MARGIN: f32 = 14.0;
/// Readings below this mean confidence / above this many unknown squares are not judged.
const MIN_CONFIDENCE: f32 = 0.45;
const MAX_UNKNOWN: usize = 10;

/// User choice, mirrors the `capture.orientation` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrientationMode {
    #[default]
    Auto,
    White,
    Black,
}

impl OrientationMode {
    pub fn parse(s: &str) -> Option<OrientationMode> {
        match s {
            "auto" => Some(OrientationMode::Auto),
            "white" => Some(OrientationMode::White),
            "black" => Some(OrientationMode::Black),
            _ => None,
        }
    }

    fn fixed(self) -> Option<Orientation> {
        match self {
            OrientationMode::Auto => None,
            OrientationMode::White => Some(Orientation::White),
            OrientationMode::Black => Some(Orientation::Black),
        }
    }
}

/// Where the current orientation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationSource {
    Manual,
    Labels,
    Structure,
    Calibration,
}

// ---------------------------------------------------------------------------
// structural evidence
// ---------------------------------------------------------------------------

/// Verdict of [`structural_orientation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structural {
    /// Exactly one orientation is legal and clearly more natural.
    Decided(Orientation),
    /// Both orientations fit: the position cannot tell which side is at the bottom.
    Ambiguous,
    /// The reading is too poor (or illegal either way) to judge.
    Unreadable,
}

fn rotated(p: &Placement) -> Placement {
    let mut r = *p;
    for (i, c) in p.iter().enumerate() {
        r[63 - i] = *c;
    }
    r
}

/// Placement (board coordinates, index 0 = a8) of the piece-placement field of a FEN.
pub fn placement_of_fen(fen: &str) -> Option<Placement> {
    let parsed: Fen = fen.trim().parse().ok()?;
    let pos: Chess = parsed.into_position(CastlingMode::Standard).ok()?;
    let board = pos.board();
    let mut p = ['.'; 64];
    for (i, c) in p.iter_mut().enumerate() {
        let sq = shakmaty::Square::from_coords(
            shakmaty::File::new((i % 8) as u32),
            shakmaty::Rank::new(7 - (i / 8) as u32),
        );
        if let Some(piece) = board.piece_at(sq) {
            *c = piece.char();
        }
    }
    Some(p)
}

fn board_fen(p: &Placement) -> String {
    let mut s = String::new();
    for r in 0..8 {
        let mut empty = 0;
        for f in 0..8 {
            let c = p[r * 8 + f];
            if c == '.' || c == '?' {
                empty += 1;
            } else {
                if empty > 0 {
                    s.push_str(&empty.to_string());
                    empty = 0;
                }
                s.push(c);
            }
        }
        if empty > 0 {
            s.push_str(&empty.to_string());
        }
        if r < 7 {
            s.push('/');
        }
    }
    s
}

/// Could `p` be a chess position: one king per side, at most 8 pawns and 16 pieces per side, no
/// pawn on the first or last rank, and legal for at least one side to move (the side not to
/// move is not in check).
pub fn placement_is_legal(p: &Placement) -> bool {
    let n = |c: char| p.iter().filter(|&&x| x == c).count();
    if n('K') != 1 || n('k') != 1 || n('P') > 8 || n('p') > 8 || p.contains(&'?') {
        return false;
    }
    let white = p.iter().filter(|c| c.is_ascii_uppercase()).count();
    let black = p.iter().filter(|c| c.is_ascii_lowercase()).count();
    if white > 16 || black > 16 {
        return false;
    }
    if (0..8).any(|f| matches!(p[f], 'P' | 'p') || matches!(p[56 + f], 'P' | 'p')) {
        return false;
    }
    legal_for(p, &['w', 'b'])
}

/// Is there a side to move for which `p` is a legal position?
fn legal_for(p: &Placement, allowed: &[char]) -> bool {
    let board = board_fen(p);
    allowed.iter().any(|t| {
        format!("{board} {t} - - 0 1")
            .parse::<Fen>()
            .ok()
            .and_then(|f| f.into_position::<Chess>(CastlingMode::Standard).ok())
            .is_some()
    })
}

/// Naturalness of `p` read as a game seen with white at the bottom (higher = more natural), or
/// `None` when it is impossible under that reading.
fn naturalness(p: &Placement, mover: Option<char>, last: Option<(u8, u8)>) -> Option<f32> {
    let n = |c: char| p.iter().filter(|&&x| x == c).count();
    let unknown = n('?');
    let (wk, bk) = (n('K'), n('k'));
    if wk > 1 || bk > 1 || ((wk == 0 || bk == 0) && unknown == 0) {
        return None;
    }
    let white = p.iter().filter(|c| c.is_ascii_uppercase()).count();
    let black = p.iter().filter(|c| c.is_ascii_lowercase()).count();
    if white > 16 || black > 16 || n('P') > 8 || n('p') > 8 {
        return None;
    }
    // Pawns never stand on the first or last rank.
    if (0..8).any(|f| matches!(p[f], 'P' | 'p') || matches!(p[56 + f], 'P' | 'p')) {
        return None;
    }
    // Legal for the side that is (or may be) to move. The colour that just moved is not to move.
    let allowed: &[char] = match mover {
        Some('w') => &['b'],
        Some('b') => &['w'],
        _ => &['w', 'b'],
    };
    if wk == 1 && bk == 1 && !legal_for(p, allowed) {
        return None;
    }
    // The last move must be something that piece can do (a pawn only goes forward).
    if let Some((from, to)) = last {
        let piece = p[to as usize];
        if piece != '.' && piece != '?' && !plausible_move(piece, from as usize, to as usize) {
            return None;
        }
    }
    let mut score = 0.0f32;
    for (i, &c) in p.iter().enumerate() {
        let rank = 8 - i / 8;
        match c {
            // Pawns advance: a game seen the right way round has them near their start.
            'P' => score -= (rank as f32) - 2.0,
            'p' => score -= 7.0 - rank as f32,
            'K' => {
                score += match i {
                    60 => 6.0,
                    58 | 62 => 3.0,
                    _ => 0.0,
                }
            }
            'k' => {
                score += match i {
                    4 => 6.0,
                    2 | 6 => 3.0,
                    _ => 0.0,
                }
            }
            'R' if i == 56 || i == 63 => score += 2.0,
            'r' if i == 0 || i == 7 => score += 2.0,
            'N' | 'B' | 'Q' | 'R' if i >= 56 => score += 1.0,
            'n' | 'b' | 'q' | 'r' if i < 8 => score += 1.0,
            _ => {}
        }
    }
    Some(score)
}

/// Which side is at the bottom, from the position alone. `placement` is what was read with
/// `Orientation::White` assumed; `mover_hint` and `last_move` are the reading's (indices in that
/// same view).
///
/// Both views are tried; one that is impossible (illegal, a pawn retreating, ...) loses, one that
/// is clearly more natural (see the module docs) wins, and anything else is `Ambiguous`. This is
/// evidence, not proof: a late pawn race looks the same from both sides, hence the margin and
/// the preference for coordinate labels and manual choice.
pub fn structural_orientation(
    placement: &Placement,
    mover_hint: Option<char>,
    last_move: Option<(u8, u8)>,
) -> Structural {
    if placement.iter().filter(|&&c| c == '?').count() > MAX_UNKNOWN {
        return Structural::Unreadable;
    }
    let as_white = naturalness(placement, mover_hint, last_move);
    let flipped_last = last_move.map(|(a, b)| (63 - a, 63 - b));
    let as_black = naturalness(&rotated(placement), mover_hint, flipped_last);
    match (as_white, as_black) {
        (None, None) => Structural::Unreadable,
        (Some(_), None) => Structural::Decided(Orientation::White),
        (None, Some(_)) => Structural::Decided(Orientation::Black),
        (Some(w), Some(b)) => {
            if w - b >= STRUCTURE_MARGIN {
                Structural::Decided(Orientation::White)
            } else if b - w >= STRUCTURE_MARGIN {
                Structural::Decided(Orientation::Black)
            } else {
                Structural::Ambiguous
            }
        }
    }
}

// ---------------------------------------------------------------------------
// resolver
// ---------------------------------------------------------------------------

/// Result of one [`OrientationResolver::update`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrientationUpdate {
    pub orientation: Option<Orientation>,
    pub source: Option<OrientationSource>,
    /// The orientation differs from before (including "became known"): start a fresh tracker.
    pub changed: bool,
    /// Undecided because both orientations fit (or nothing could be read yet).
    pub undecided: bool,
    /// A different orientation than the current one was just seen and awaits confirmation: do
    /// not publish a position for now.
    pub disputed: bool,
    /// Manual mode: the labels show this side at the bottom, contradicting the setting.
    pub conflict: Option<Orientation>,
}

/// Holds the orientation decision and when to re-check it. Pure state: all evidence comes from
/// the frame handed to [`update`](OrientationResolver::update).
#[derive(Debug, Clone)]
pub struct OrientationResolver {
    mode: OrientationMode,
    current: Option<(Orientation, OrientationSource)>,
    since_check: u32,
    pending: Option<(Orientation, u8)>,
    undecided: bool,
    /// Manual mode only: the coordinate labels say the other side is at the bottom.
    conflict: Option<Orientation>,
    /// Consecutive checks without a readable label while the decision rests on labels. Two or
    /// more make that decision stale: the board may have been turned while they were hidden.
    label_misses: u8,
    /// The tracker was lost since the last check (see [`request_check`](Self::request_check)).
    suspect: bool,
}

impl OrientationResolver {
    pub fn new(mode: OrientationMode) -> Self {
        OrientationResolver {
            mode,
            current: mode.fixed().map(|o| (o, OrientationSource::Manual)),
            since_check: 0,
            pending: None,
            undecided: false,
            conflict: None,
            label_misses: 0,
            suspect: false,
        }
    }

    pub fn mode(&self) -> OrientationMode {
        self.mode
    }

    /// Change the user's choice. Returns whether the effective orientation changed.
    pub fn set_mode(&mut self, mode: OrientationMode) -> bool {
        let before = self.current.map(|c| c.0);
        self.mode = mode;
        self.pending = None;
        self.since_check = 0;
        self.undecided = false;
        self.conflict = None;
        self.label_misses = 0;
        self.suspect = false;
        match mode.fixed() {
            Some(o) => self.current = Some((o, OrientationSource::Manual)),
            None => {
                if self
                    .current
                    .is_some_and(|c| c.1 == OrientationSource::Manual)
                {
                    self.current = None;
                }
            }
        }
        self.current.map(|c| c.0) != before
    }

    pub fn current(&self) -> Option<Orientation> {
        self.current.map(|c| c.0)
    }

    pub fn source(&self) -> Option<OrientationSource> {
        self.current.map(|c| c.1)
    }

    /// The user calibrated on a start position seen with `o` at the bottom.
    pub fn note_calibration(&mut self, o: Orientation) {
        if self.mode == OrientationMode::Auto {
            self.current = Some((o, OrientationSource::Calibration));
            self.label_misses = 0;
            self.pending = None;
            self.since_check = 0;
            self.undecided = false;
        }
    }

    /// Adopt `o` found by other means (a known position matching one view).
    pub fn note_structure(&mut self, o: Orientation) {
        if self.mode == OrientationMode::Auto {
            self.current = Some((o, OrientationSource::Structure));
            self.label_misses = 0;
            self.pending = None;
            self.since_check = 0;
            self.undecided = false;
        }
    }

    /// Ask for a check on the frame after next (the tracker is lost: the board may have been
    /// turned around). Repeated calls cost at most one check every second frame.
    pub fn request_check(&mut self) {
        self.since_check = self.since_check.max(RECHECK_FRAMES - 2);
        self.suspect = true;
    }

    /// Forget an automatic decision (another site, another board).
    pub fn forget(&mut self) {
        if self.mode == OrientationMode::Auto {
            self.current = None;
        }
        self.pending = None;
        self.since_check = 0;
        self.undecided = false;
        self.label_misses = 0;
        self.suspect = false;
    }

    /// Advance one frame. Checks the evidence when no orientation is known, `redetected`, or
    /// [`RECHECK_FRAMES`] have passed. `recognizer` is needed for the structural fallback.
    pub fn update(
        &mut self,
        frame: &Frame,
        rect: &BoardRect,
        site: Site,
        recognizer: Option<&Recognizer>,
        redetected: bool,
    ) -> OrientationUpdate {
        let before = self.current.map(|c| c.0);
        self.since_check = self.since_check.saturating_add(1);
        let due = self.current.is_none() || redetected || self.since_check >= RECHECK_FRAMES;
        if self.mode == OrientationMode::Auto {
            if due {
                self.since_check = 0;
                self.check(frame, rect, site, recognizer);
            }
        } else if due && (redetected || self.since_check >= RECHECK_FRAMES) {
            // A manual choice is final, but say so when the board's own labels disagree.
            self.since_check = 0;
            self.conflict = detect_orientation(frame, rect, site).filter(|o| Some(*o) != before);
        }
        let now = self.current.map(|c| c.0);
        OrientationUpdate {
            orientation: now,
            source: self.source(),
            changed: now != before,
            undecided: now.is_none() && self.undecided,
            disputed: self.pending.is_some(),
            conflict: self.conflict,
        }
    }

    fn check(
        &mut self,
        frame: &Frame,
        rect: &BoardRect,
        site: Site,
        recognizer: Option<&Recognizer>,
    ) {
        let mut evidence: Option<(Orientation, OrientationSource)> = None;
        let mut undecided = false;
        let held_labels = self.source() == Some(OrientationSource::Labels);
        let suspect = std::mem::take(&mut self.suspect);
        if let Some(o) = detect_orientation(frame, rect, site) {
            evidence = Some((o, OrientationSource::Labels));
            self.label_misses = 0;
        } else {
            if held_labels {
                self.label_misses = self.label_misses.saturating_add(1);
                if self.label_misses < 2 {
                    // Labels gone: look again on the next frame before calling them stale.
                    self.since_check = RECHECK_FRAMES - 1;
                }
            }
            if let Some(rec) = recognizer {
                // No label to read: structural evidence. A decision that came from labels that
                // are no longer readable is stale and gets no protection from it: the board may
                // have been turned (a new game as the other colour) while they were hidden.
                let reading = rec.read(frame, rect, Orientation::White);
                let unknown = reading.placement.iter().filter(|&&c| c == '?').count();
                if reading.confidence >= MIN_CONFIDENCE && unknown <= MAX_UNKNOWN {
                    match structural_orientation(
                        &reading.placement,
                        reading.mover_hint,
                        reading.last_move_squares,
                    ) {
                        Structural::Decided(o) => {
                            evidence = Some((o, OrientationSource::Structure))
                        }
                        Structural::Ambiguous | Structural::Unreadable => undecided = true,
                    }
                }
            }
        }
        if held_labels && self.label_misses >= 2 && undecided && suspect {
            // Stale label decision, the tracker lost the board, and the position cannot say which
            // way round it is now: publish nothing rather than a rotated position.
            self.current = None;
            self.pending = None;
            self.undecided = true;
            return;
        }
        match (evidence, self.current) {
            (Some((o, src)), None) => {
                self.current = Some((o, src));
                self.pending = None;
                self.undecided = false;
            }
            (Some((o, src)), Some((cur, cur_src))) => {
                if o == cur {
                    // Same answer: keep the stronger source.
                    if src == OrientationSource::Labels || cur_src != OrientationSource::Labels {
                        self.current = Some((cur, src));
                    }
                    self.pending = None;
                } else {
                    // A different answer must repeat on the very next frame before it flips.
                    let count = match self.pending {
                        Some((p, n)) if p == o => n + 1,
                        _ => 1,
                    };
                    if count >= 2 {
                        self.current = Some((o, src));
                        self.pending = None;
                    } else {
                        self.pending = Some((o, count));
                        self.since_check = RECHECK_FRAMES - 1;
                    }
                }
                self.undecided = false;
            }
            (None, cur) => {
                self.pending = None;
                // An established decision is kept when the evidence goes away.
                self.undecided = cur.is_none() && undecided;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// pipeline
// ---------------------------------------------------------------------------

fn side_name(o: Orientation) -> &'static str {
    match o {
        Orientation::White => "white",
        Orientation::Black => "black",
    }
}

/// One processed frame.
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// `None` while the orientation is unknown (nothing was read).
    pub reading: Option<Reading>,
    pub tracked: Tracked,
    pub orientation: Option<Orientation>,
    /// The orientation just changed and the tracker was restarted.
    pub changed: bool,
    /// Message for the user when the orientation could not be established.
    pub status: Option<String>,
}

/// `OrientationResolver` + `PositionTracker`: the per-frame path from a located board to a FEN
/// that never starts from a default orientation.
pub struct BoardPipeline {
    pub resolver: OrientationResolver,
    pub tracker: PositionTracker,
}

impl BoardPipeline {
    pub fn new(mode: OrientationMode) -> Self {
        let mut tracker = PositionTracker::new();
        tracker.lock_orientation(true);
        BoardPipeline {
            resolver: OrientationResolver::new(mode),
            tracker,
        }
    }

    /// Restart tracking (keeps the orientation decision unless `forget`).
    pub fn reset_tracker(&mut self) {
        self.tracker = PositionTracker::new();
        self.tracker.lock_orientation(true);
    }

    /// Orientation + read + track one frame whose board is at `rect`. `redetected` is true on the
    /// first frame after the board was (re)located.
    pub fn process(
        &mut self,
        recognizer: &Recognizer,
        frame: &Frame,
        rect: &BoardRect,
        site: Site,
        redetected: bool,
    ) -> PipelineOutput {
        let up = self
            .resolver
            .update(frame, rect, site, Some(recognizer), redetected);
        if up.changed {
            self.reset_tracker();
        }
        let lost = |status: Option<String>, changed: bool| PipelineOutput {
            reading: None,
            tracked: Tracked {
                state: TrackState::Lost,
                fen: None,
                last_move: None,
                orientation: Orientation::White,
                confidence: 0.0,
            },
            orientation: None,
            changed,
            status,
        };
        let Some(o) = up.orientation else {
            return lost(up.undecided.then(|| MSG_CANT_TELL.to_string()), up.changed);
        };
        if up.disputed {
            // The board may just have been turned around: say nothing rather than something wrong.
            let mut out = lost(None, up.changed);
            out.orientation = Some(o);
            return out;
        }
        let reading = recognizer.read(frame, rect, o);
        let mut tracked = self.tracker.update(&reading);
        // The decision is the authority: the tracker's own orientation may lag it by a few frames.
        tracked.orientation = o;
        if tracked.state == TrackState::Lost {
            self.resolver.request_check();
        }
        let status = up.conflict.map(|c| {
            format!(
                "Orientation is set to {} at the bottom but the board's coordinates show {}.",
                side_name(o),
                side_name(c)
            )
        });
        PipelineOutput {
            reading: Some(reading),
            tracked,
            orientation: Some(o),
            changed: up.changed,
            status,
        }
    }

    /// Start tracking from a position the user states. With an unknown orientation the view
    /// that reads as that position decides it; if neither does, the position does not match the
    /// board and the call fails.
    pub fn set_known_position(
        &mut self,
        recognizer: &Recognizer,
        frame: &Frame,
        rect: &BoardRect,
        fen: &str,
    ) -> Result<(), String> {
        if self.resolver.current().is_none() {
            let want =
                placement_of_fen(fen).ok_or_else(|| "That position is not valid.".to_string())?;
            let seen = recognizer.read(frame, rect, Orientation::White).placement;
            let fits = |p: &Placement| {
                p.iter().zip(&want).all(|(a, b)| *a == '?' || a == b)
                    && p.iter().filter(|&&c| c == '?').count() <= MAX_UNKNOWN
            };
            match (fits(&seen), fits(&rotated(&seen))) {
                (true, false) => self.resolver.note_structure(Orientation::White),
                (false, true) => self.resolver.note_structure(Orientation::Black),
                _ => {
                    return Err(format!(
                        "{MSG_CANT_TELL} The board does not match that position, or looks the same from both sides."
                    ));
                }
            }
        }
        self.tracker
            .reset_to_fen(fen)
            .map_err(|e| format!("That position is not valid: {e}"))
    }
}
