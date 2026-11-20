//! Turns raw engine snapshots into the UI-facing [`AnalysisFrame`]:
//! white-POV scores, SAN, win%/loss%, move classification, tactic tags (computed
//! from real board geometry with `shakmaty`) and one-sentence explanations.

use crate::uci::{EngineError, EngineUpdate, RawLine, Score, parse_position};
use serde::{Deserialize, Serialize};
use shakmaty::attacks::{attacks as piece_attacks, between, ray};
use shakmaty::san::SanPlus;
use shakmaty::uci::UciMove;
use shakmaty::{Bitboard, Board, Chess, Color, Move, Position, Role, Square};

/// Quality of a candidate move, chess.com-style review vocabulary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MoveClass {
    Brilliant,
    Great,
    Best,
    Excellent,
    Good,
    Inaccuracy,
    Mistake,
    Blunder,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum TacticTag {
    Fork,
    Pin,
    Skewer,
    Discovered,
    Hanging,
    Check,
    Mate,
    Capture,
    Promotion,
    Sacrifice,
    Castle,
    EnPassant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMove {
    /// 1-based MultiPV rank.
    pub rank: u32,
    pub uci: String,
    pub san: String,
    /// Algebraic squares, e.g. "e2".
    pub from: String,
    pub to: String,
    /// Centipawns from WHITE's point of view (None when mate).
    pub score_cp: Option<i32>,
    /// Mate distance from WHITE's point of view (positive = white mates).
    pub mate: Option<i32>,
    /// White's expected score 0..100 after this move.
    pub win_pct: f64,
    /// Expected-points loss vs the best line, in percentage points (>= 0).
    pub loss_pct: f64,
    pub cls: MoveClass,
    pub tags: Vec<TacticTag>,
    pub pv_uci: Vec<String>,
    pub pv_san: Vec<String>,
    pub depth: u32,
    /// One short plain-language sentence, e.g. "Forks the king and rook."
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFrame {
    pub fen: String,
    /// "w" or "b".
    pub side_to_move: String,
    pub depth: u32,
    pub nps: u64,
    /// White's point of view.
    pub eval_cp: Option<i32>,
    pub mate: Option<i32>,
    pub win_pct: f64,
    pub candidates: Vec<CandidateMove>,
    /// true while the search is still running.
    pub thinking: bool,
    /// Monotonic millis supplied by the caller.
    pub ts: u64,
    /// White/draw/black win chances in per-mille (sum 1000), white's point of view.
    #[serde(default)]
    pub wdl: Option<[u32; 3]>,
    /// Depth the shell wants before the display counts as stable. `None` from
    /// [`build_frame`]; the shell fills it in.
    #[serde(default)]
    pub target_depth: Option<u32>,
}

/// Maximum number of plies kept in `pv_uci` / `pv_san`.
pub const MAX_PV_PLIES: usize = 8;

/// Loss thresholds (percentage points of expected score) for each class.
const EXCELLENT_MAX_LOSS: f64 = 2.0;
const GOOD_MAX_LOSS: f64 = 5.0;
const INACCURACY_MAX_LOSS: f64 = 10.0;
const MISTAKE_MAX_LOSS: f64 = 20.0;
/// Losses below this count as "no loss" (rank-1 ties, mate-vs-mate).
const BEST_EPSILON: f64 = 0.05;
/// "Great": the runner-up loses at least this much, or falls from healthy to bad.
const ONLY_MOVE_GAP: f64 = 15.0;
const SWING_KEEP: f64 = 35.0;
const SWING_DROP: f64 = 20.0;
const SWING_MIN_GAP: f64 = 10.0;
/// "Brilliant": the mover must stay at least roughly equal...
const BRILLIANT_MIN_WIN: f64 = 45.0;
/// ...and a position where a quiet alternative already crushes does not qualify.
const CRUSHING_WIN: f64 = 85.0;
/// SEE (in pawns) at or below which a move counts as a piece sacrifice.
const SACRIFICE_SEE: i32 = -2;

/// Win-probability model (logistic on centipawns), 0..100 for the side whose
/// point of view the cp value has.
pub fn win_percent(cp: i32) -> f64 {
    50.0 + 50.0 * (2.0 / (1.0 + (-0.003_682_08 * f64::from(cp)).exp()) - 1.0)
}

/// Classification by loss alone (no sacrifice / only-move logic).
pub fn classify_loss(loss_pct: f64) -> MoveClass {
    if loss_pct < BEST_EPSILON {
        MoveClass::Best
    } else if loss_pct <= EXCELLENT_MAX_LOSS {
        MoveClass::Excellent
    } else if loss_pct <= GOOD_MAX_LOSS {
        MoveClass::Good
    } else if loss_pct <= INACCURACY_MAX_LOSS {
        MoveClass::Inaccuracy
    } else if loss_pct <= MISTAKE_MAX_LOSS {
        MoveClass::Mistake
    } else {
        MoveClass::Blunder
    }
}

// ---------------------------------------------------------------------------
// Scores
// ---------------------------------------------------------------------------

/// Convert a side-to-move score to White's point of view as `(cp, mate)`.
fn white_pov(score: Score, turn: Color) -> (Option<i32>, Option<i32>) {
    let sign = if turn == Color::White { 1 } else { -1 };
    match score {
        Score::Cp(cp) => (Some(cp.saturating_mul(sign)), None),
        Score::Mate(n) => (None, Some(n.saturating_mul(sign))),
    }
}

/// White's expected score for a white-POV `(cp, mate)` pair.
fn white_win(cp: Option<i32>, mate: Option<i32>) -> f64 {
    match (cp, mate) {
        (_, Some(m)) if m > 0 => 100.0,
        (_, Some(m)) if m < 0 => 0.0,
        (Some(cp), _) => win_percent(cp),
        _ => 50.0,
    }
}

fn mover_pct(white_pct: f64, mover: Color) -> f64 {
    if mover == Color::White {
        white_pct
    } else {
        100.0 - white_pct
    }
}

// ---------------------------------------------------------------------------
// Board helpers
// ---------------------------------------------------------------------------

fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 100,
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

fn is_slider(role: Role) -> bool {
    matches!(role, Role::Bishop | Role::Rook | Role::Queen)
}

/// True when nothing of `color` attacks `sq` given `occ`.
fn undefended(board: &Board, sq: Square, color: Color, occ: Bitboard) -> bool {
    (board.attacks_to(sq, color, occ) & occ).is_empty()
}

/// Static exchange evaluation of `mv` from the mover's point of view, in pawns.
fn see(board: &Board, mv: &Move) -> i32 {
    let (Some(from), to) = (mv.from(), mv.to()) else {
        return 0;
    };
    if mv.is_castle() {
        return 0;
    }
    let mover = board.color_at(from).unwrap_or(Color::White);
    let mut occ = board.occupied().without(from);
    let mut victim = mv.capture().map_or(0, value);
    if mv.is_en_passant() {
        victim = 1;
        occ = occ.without(Square::from_coords(to.file(), from.rank()));
    }
    if let Some(p) = mv.promotion() {
        victim += value(p) - 1;
    }
    let mut gains = [0i32; 32];
    gains[0] = victim;
    let mut cur = value(mv.promotion().unwrap_or(mv.role()));
    let mut side = mover.other();
    let mut d = 0usize;
    while d < gains.len() - 1 {
        let attackers = board.attacks_to(to, side, occ) & occ;
        let mut best: Option<(Square, Role)> = None;
        for sq in attackers {
            if let Some(role) = board.role_at(sq)
                && best.is_none_or(|(_, r)| value(role) < value(r))
            {
                best = Some((sq, role));
            }
        }
        let Some((sq, role)) = best else { break };
        d += 1;
        gains[d] = cur - gains[d - 1];
        cur = value(role);
        occ = occ.without(sq);
        side = side.other();
    }
    while d > 0 {
        gains[d - 1] = -(-gains[d - 1]).max(gains[d]);
        d -= 1;
    }
    gains[0]
}

// ---------------------------------------------------------------------------
// Tactic geometry
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Facts {
    tags: Vec<TacticTag>,
    fork: Vec<(Role, Square)>,
    /// (pinned piece, piece behind it)
    pin: Option<((Role, Square), (Role, Square))>,
    /// (piece attacked first, piece behind it)
    skewer: Option<((Role, Square), (Role, Square))>,
    /// (discovering slider, target)
    discovered: Option<(Role, Role, Square)>,
    discovered_check: bool,
    hanging: Vec<(Role, Square)>,
    captured: Option<(Role, Square)>,
    moved: Option<Role>,
    promotion: Option<Role>,
    see: i32,
}

impl Facts {
    fn push(&mut self, tag: TacticTag) {
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }
}

/// Compute tactic tags and supporting facts for `mv` played in `pos`.
fn tactic_facts(pos: &Chess, mv: &Move) -> Facts {
    let mut f = Facts {
        moved: Some(mv.role()),
        promotion: mv.promotion(),
        ..Facts::default()
    };
    let us = pos.turn();
    let them = us.other();
    let mut after = pos.clone();
    after.play_unchecked(*mv);

    if after.is_checkmate() {
        f.push(TacticTag::Mate);
    }
    if after.is_check() {
        f.push(TacticTag::Check);
    }
    if mv.is_castle() {
        f.push(TacticTag::Castle);
        return finish_tags(f);
    }
    if mv.is_capture() {
        f.push(TacticTag::Capture);
    }
    if mv.is_en_passant() {
        f.push(TacticTag::EnPassant);
    }
    if mv.is_promotion() {
        f.push(TacticTag::Promotion);
    }

    let before_board = pos.board();
    let after_board = after.board();
    let occ_b = before_board.occupied();
    let occ_a = after_board.occupied();
    let (Some(from), to) = (mv.from(), mv.to()) else {
        return finish_tags(f);
    };
    let Some(moved) = after_board.piece_at(to) else {
        return finish_tags(f);
    };
    let enemy_a = after_board.by_color(them);

    f.see = see(before_board, mv);
    if f.see <= SACRIFICE_SEE && !after.is_checkmate() {
        f.push(TacticTag::Sacrifice);
    }
    if let Some(role) = mv.capture() {
        let sq = if mv.is_en_passant() {
            Square::from_coords(to.file(), from.rank())
        } else {
            to
        };
        f.captured = Some((role, sq));
    }

    // Is the moved piece simply en prise afterwards? (Then forks/pins below do not hold.)
    let refuted = {
        let attackers = after_board.attacks_to(to, them, occ_a) & occ_a;
        let defended = !undefended(after_board, to, us, occ_a);
        attackers.into_iter().any(|sq| {
            let role = after_board.role_at(sq).unwrap_or(Role::King);
            if role == Role::King {
                !defended
            } else {
                !defended || value(role) < value(moved.role)
            }
        })
    };

    // Fork: the moved piece attacks two or more valuable/undefended pieces (the king always counts).
    if moved.role != Role::King && !refuted {
        let attacked = piece_attacks(to, moved, occ_a) & enemy_a;
        for sq in attacked {
            let Some(role) = after_board.role_at(sq) else {
                continue;
            };
            if role == Role::King
                || (role != Role::Pawn
                    && (value(role) > value(moved.role)
                        || undefended(after_board, sq, them, occ_a)))
            {
                f.fork.push((role, sq));
            }
        }
        if f.fork.len() >= 2 {
            f.fork.sort_by_key(|&(r, _)| std::cmp::Reverse(value(r)));
            f.push(TacticTag::Fork);
        } else {
            f.fork.clear();
        }
    }

    // Pin / skewer: a slider attacks one piece with a second enemy piece directly behind it.
    if is_slider(moved.role) && !refuted {
        let attacked = piece_attacks(to, moved, occ_a) & enemy_a;
        for x in attacked {
            let Some(x_role) = after_board.role_at(x) else {
                continue;
            };
            let beyond = piece_attacks(to, moved, occ_a.without(x))
                & ray(to, x)
                & !between(to, x)
                & !Bitboard::from(x);
            // `ray(to, x)` is the whole line, so `beyond` may also hold the first blocker on the
            // opposite side of `to`; only a piece with `x` between it and `to` is behind `x`.
            let Some(y) = (beyond & enemy_a)
                .into_iter()
                .find(|&y| between(to, y).contains(x))
            else {
                continue;
            };
            let Some(y_role) = after_board.role_at(y) else {
                continue;
            };
            if value(y_role) > value(x_role) && f.pin.is_none() {
                f.pin = Some(((x_role, x), (y_role, y)));
            } else if value(x_role) > value(y_role) && y_role != Role::Pawn && f.skewer.is_none() {
                f.skewer = Some(((x_role, x), (y_role, y)));
            }
        }
        if f.pin.is_some() {
            f.push(TacticTag::Pin);
        }
        if f.skewer.is_some() {
            f.push(TacticTag::Skewer);
        }
    }

    // Discovered attack/check: moving off a line unleashes another of our sliders.
    for s in after_board.by_color(us) & after_board.sliders() & !Bitboard::from(to) {
        let Some(s_piece) = after_board.piece_at(s) else {
            continue;
        };
        let was = piece_attacks(s, s_piece, occ_b) & before_board.by_color(them);
        let now = piece_attacks(s, s_piece, occ_a) & enemy_a;
        for t in now & !was {
            if !between(s, t).contains(from) {
                continue;
            }
            let Some(t_role) = after_board.role_at(t) else {
                continue;
            };
            if t_role == Role::King {
                f.discovered_check = true;
                f.discovered = Some((s_piece.role, t_role, t));
            } else if t_role != Role::Pawn
                && (undefended(after_board, t, them, occ_a) || value(t_role) > value(s_piece.role))
                && f.discovered.is_none()
            {
                f.discovered = Some((s_piece.role, t_role, t));
            }
        }
    }
    if f.discovered.is_some() {
        f.push(TacticTag::Discovered);
    }

    // Hanging: it captures an undefended piece, or newly attacks one that nothing defends.
    let hanging_set = |board: &Board, occ: Bitboard, attacker: Color| -> Bitboard {
        let victims = board.by_color(attacker.other()) & !board.pawns() & !board.kings();
        victims
            .into_iter()
            .filter(|&sq| {
                !(board.attacks_to(sq, attacker, occ) & occ).is_empty()
                    && undefended(board, sq, attacker.other(), occ)
            })
            .collect()
    };
    let hanging_before = hanging_set(before_board, occ_b, us);
    let hanging_after = hanging_set(after_board, occ_a, us);
    if let Some((role, sq)) = f.captured
        && role != Role::Pawn
        && role != Role::King
        && undefended(after_board, sq, them, occ_a)
    {
        f.hanging.push((role, sq));
    }
    for sq in hanging_after & !hanging_before {
        if let Some(role) = after_board.role_at(sq) {
            f.hanging.push((role, sq));
        }
    }
    if !f.hanging.is_empty() {
        f.push(TacticTag::Hanging);
    }

    finish_tags(f)
}

/// Stable, readable tag order.
fn finish_tags(mut f: Facts) -> Facts {
    const ORDER: [TacticTag; 12] = [
        TacticTag::Mate,
        TacticTag::Check,
        TacticTag::Capture,
        TacticTag::EnPassant,
        TacticTag::Promotion,
        TacticTag::Castle,
        TacticTag::Fork,
        TacticTag::Pin,
        TacticTag::Skewer,
        TacticTag::Discovered,
        TacticTag::Hanging,
        TacticTag::Sacrifice,
    ];
    let have = std::mem::take(&mut f.tags);
    f.tags = ORDER.into_iter().filter(|t| have.contains(t)).collect();
    f
}

// ---------------------------------------------------------------------------
// Explanations
// ---------------------------------------------------------------------------

fn target_phrase(role: Role, sq: Square) -> String {
    if role == Role::King {
        "the king".to_owned()
    } else {
        format!("the {} on {sq}", role_name(role))
    }
}

fn join_phrases(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    }
}

/// The tactical sentence without trailing punctuation, if the move has anything to say.
fn tactic_phrase(f: &Facts, brilliant: bool) -> Option<String> {
    let has = |t: TacticTag| f.tags.contains(&t);
    if has(TacticTag::Mate) {
        return Some("Delivers checkmate".into());
    }
    if brilliant && has(TacticTag::Sacrifice) {
        let role = f.moved.map_or("piece", role_name);
        return Some(format!(
            "Sacrifices the {role}, and the follow-up is worth more than the material"
        ));
    }
    if let Some(p) = f.promotion {
        return Some(format!("Promotes to a {}", role_name(p)));
    }
    if has(TacticTag::Fork) {
        let parts: Vec<String> = f.fork.iter().map(|&(r, s)| target_phrase(r, s)).collect();
        return Some(format!("Forks {}", join_phrases(&parts)));
    }
    if let Some(((xr, xs), (yr, ys))) = f.pin {
        let behind = if yr == Role::King {
            "the king".to_owned()
        } else {
            format!("the {} on {ys}", role_name(yr))
        };
        return Some(format!("Pins the {} on {xs} to {behind}", role_name(xr)));
    }
    if let Some(((xr, xs), (yr, ys))) = f.skewer {
        return Some(format!(
            "Skewers the {} on {xs}, leaving the {} on {ys} behind it exposed",
            role_name(xr),
            role_name(yr)
        ));
    }
    if let Some((sr, tr, ts)) = f.discovered {
        return Some(if f.discovered_check {
            format!("Discovers check from the {}", role_name(sr))
        } else {
            format!(
                "Uncovers an attack on {} from the {}",
                target_phrase(tr, ts),
                role_name(sr)
            )
        });
    }
    if let Some(&(role, sq)) = f.hanging.first() {
        return Some(if f.captured.is_some_and(|(_, s)| s == sq) {
            format!("Wins the undefended {}", role_name(role))
        } else {
            format!("Attacks the undefended {} on {sq}", role_name(role))
        });
    }
    if has(TacticTag::Check) {
        return Some("Gives check".into());
    }
    if has(TacticTag::EnPassant) {
        return Some("Captures a pawn en passant".into());
    }
    if let Some((role, sq)) = f.captured {
        return Some(format!("Captures the {} on {sq}", role_name(role)));
    }
    if has(TacticTag::Castle) {
        return Some("Castles, tucking the king away and activating the rook".into());
    }
    if has(TacticTag::Sacrifice) {
        let role = f.moved.map_or("piece", role_name);
        return Some(format!("Gives up the {role} for less material in return"));
    }
    None
}

fn explain(
    f: &Facts,
    cls: MoveClass,
    loss: f64,
    mover_mate: Option<i32>,
    only_move: bool,
) -> String {
    let phrase = tactic_phrase(f, cls == MoveClass::Brilliant);
    match cls {
        MoveClass::Inaccuracy | MoveClass::Mistake | MoveClass::Blunder => {
            let cost = format!("gives up about {loss:.0}% of your winning chances");
            match phrase {
                Some(p) => format!("{p}, but it {cost}."),
                None => format!("This {cost}."),
            }
        }
        _ => {
            if let Some(p) = phrase {
                return format!("{p}.");
            }
            match mover_mate {
                Some(n) if n > 0 => format!("Starts a forced mate in {n}."),
                Some(n) if n < 0 => format!("Allows mate in {}.", -n),
                _ => match cls {
                    MoveClass::Great if only_move => {
                        "The only move that keeps the position under control.".into()
                    }
                    MoveClass::Brilliant | MoveClass::Great | MoveClass::Best => {
                        "The engine's top choice.".into()
                    }
                    MoveClass::Excellent => {
                        "A strong alternative, nearly as good as the best move.".into()
                    }
                    _ => "A reasonable move that gives up very little.".into(),
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Frame construction
// ---------------------------------------------------------------------------

struct Prepared {
    line: RawLine,
    mv: Move,
    cp: Option<i32>,
    mate: Option<i32>,
    win: f64,
    mover_win: f64,
    pv_uci: Vec<String>,
    pv_san: Vec<String>,
}

fn prepare(pos: &Chess, line: &RawLine) -> Result<Prepared, EngineError> {
    let turn = pos.turn();
    let (cp, mate) = white_pov(line.score, turn);
    let win = white_win(cp, mate);
    let mut cur = pos.clone();
    let mut first: Option<Move> = None;
    let mut pv_uci = Vec::new();
    let mut pv_san = Vec::new();
    for tok in line.pv.iter().take(MAX_PV_PLIES) {
        let parsed = tok
            .parse::<UciMove>()
            .ok()
            .and_then(|u| u.to_move(&cur).ok());
        let Some(mv) = parsed else {
            if first.is_none() {
                return Err(EngineError::Protocol(format!(
                    "illegal first pv move `{tok}`"
                )));
            }
            break;
        };
        pv_san.push(SanPlus::from_move_and_play_unchecked(&mut cur, mv).to_string());
        pv_uci.push(tok.clone());
        first.get_or_insert(mv);
    }
    let mv = first.ok_or_else(|| EngineError::Protocol("empty pv".into()))?;
    Ok(Prepared {
        line: line.clone(),
        mv,
        cp,
        mate,
        win,
        mover_win: mover_pct(win, turn),
        pv_uci,
        pv_san,
    })
}

/// Turn a raw engine snapshot into the UI-facing frame: SAN conversion,
/// white-POV scores, win%, loss%, classification, tactic tags, explanations.
pub fn build_frame(update: &EngineUpdate, ts: u64) -> Result<AnalysisFrame, EngineError> {
    let pos = parse_position(&update.fen)?;
    let turn = pos.turn();
    let side_to_move = if turn == Color::White { "w" } else { "b" }.to_owned();

    let mut raw: Vec<&RawLine> = update.lines.iter().filter(|l| !l.pv.is_empty()).collect();
    raw.sort_by_key(|l| l.multipv);
    let prepared: Vec<Prepared> = raw
        .into_iter()
        .map(|l| prepare(&pos, l))
        .collect::<Result<_, _>>()?;

    if prepared.is_empty() {
        // No search lines: terminal position (or search not started).
        let (mate, win) = if pos.is_checkmate() {
            (Some(0), if turn == Color::White { 0.0 } else { 100.0 })
        } else {
            (None, 50.0)
        };
        return Ok(AnalysisFrame {
            fen: update.fen.clone(),
            side_to_move,
            depth: update.depth,
            nps: update.nps,
            eval_cp: if mate.is_none() { Some(0) } else { None },
            mate,
            win_pct: win,
            candidates: Vec::new(),
            thinking: !update.done,
            ts,
            wdl: None,
            target_depth: None,
        });
    }

    let best_win = prepared
        .iter()
        .map(|p| p.mover_win)
        .fold(f64::MIN, f64::max);
    let facts: Vec<Facts> = prepared.iter().map(|p| tactic_facts(&pos, &p.mv)).collect();
    let is_sac = |i: usize| facts[i].tags.contains(&TacticTag::Sacrifice);

    // Only-move: the top line is much better than the best alternative.
    let top_only_move = prepared.len() >= 2 && {
        let second = prepared[1..]
            .iter()
            .map(|p| p.mover_win)
            .fold(f64::MIN, f64::max);
        let gap = prepared[0].mover_win - second;
        gap >= ONLY_MOVE_GAP
            || (prepared[0].mover_win >= SWING_KEEP && second <= SWING_DROP && gap >= SWING_MIN_GAP)
    };

    let mut candidates = Vec::with_capacity(prepared.len());
    for (i, p) in prepared.iter().enumerate() {
        let loss = (best_win - p.mover_win).max(0.0);
        let mut cls = classify_loss(loss);
        if i == 0 && cls == MoveClass::Best && top_only_move {
            cls = MoveClass::Great;
        }
        if matches!(
            cls,
            MoveClass::Best | MoveClass::Excellent | MoveClass::Great
        ) && is_sac(i)
        {
            let quiet_crush = prepared
                .iter()
                .enumerate()
                .any(|(j, q)| j != i && !is_sac(j) && q.mover_win >= CRUSHING_WIN);
            if p.mover_win >= BRILLIANT_MIN_WIN && !quiet_crush {
                cls = MoveClass::Brilliant;
            }
        }
        let mover_mate = p.mate.map(|m| if turn == Color::White { m } else { -m });
        let explanation = explain(&facts[i], cls, loss, mover_mate, i == 0 && top_only_move);
        candidates.push(CandidateMove {
            rank: p.line.multipv,
            uci: p.pv_uci[0].clone(),
            san: p.pv_san[0].clone(),
            from: p.mv.from().map_or_else(String::new, |s| s.to_string()),
            to: uci_to_square(&p.pv_uci[0]),
            score_cp: p.cp,
            mate: p.mate,
            win_pct: p.win,
            loss_pct: loss,
            cls,
            tags: facts[i].tags.clone(),
            pv_uci: p.pv_uci.clone(),
            pv_san: p.pv_san.clone(),
            depth: p.line.depth,
            explanation: Some(explanation),
        });
    }

    let top = &prepared[0];
    Ok(AnalysisFrame {
        fen: update.fen.clone(),
        side_to_move,
        depth: update.depth,
        nps: update.nps,
        eval_cp: top.cp,
        mate: top.mate,
        win_pct: top.win,
        candidates,
        thinking: !update.done,
        ts,
        wdl: top.line.wdl.map(|w| wdl_white_pov(w, turn)),
        target_depth: None,
    })
}

/// Side-to-move win/draw/loss per-mille -> white/draw/black per-mille.
fn wdl_white_pov(w: [u32; 3], turn: Color) -> [u32; 3] {
    if turn == Color::White {
        w
    } else {
        [w[2], w[1], w[0]]
    }
}

/// Tunables of [`StabilityTracker`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilityConfig {
    /// The display is never "settled" below this depth (unless forced/mate/done).
    pub min_depth: u32,
    /// The best move must be unchanged for this many consecutive depths.
    pub stable_depths: usize,
    /// Largest allowed win% movement of the best line across those depths.
    pub max_swing_pct: f64,
    /// A position that stays settled for `early_factor * stable_depths` depths may
    /// lower its target depth down to the current depth, but never below `floor_depth`.
    pub early_factor: usize,
    pub floor_depth: u32,
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            min_depth: 12,
            stable_depths: 3,
            max_swing_pct: 4.0,
            early_factor: 2,
            floor_depth: 8,
        }
    }
}

/// Why a position counts as stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableReason {
    /// Only one legal move: nothing to compute.
    Forced,
    /// A forced mate was found.
    Mate,
    /// Minimum depth reached, best move unchanged and the evaluation steady.
    Settled,
    /// The search finished by itself (depth/time limit, or no moves).
    Finished,
}

/// Result of feeding one snapshot to [`StabilityTracker::feed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stability {
    pub stable: bool,
    pub reason: Option<StableReason>,
    /// Depth of the snapshot just fed.
    pub depth: u32,
    /// Depth the UI should wait for: `min_depth` normally, lower for simple positions.
    pub target_depth: u32,
}

/// Decides when the analysis of ONE position is trustworthy enough to display.
/// Feed every [`EngineUpdate`] of that position in order; a snapshot for another
/// FEN automatically starts a new position.
#[derive(Debug, Clone)]
pub struct StabilityTracker {
    cfg: StabilityConfig,
    fen: Option<String>,
    forced: bool,
    /// (depth, best move, white-POV-free mover win%) per distinct depth, oldest first.
    history: Vec<(u32, String, f64)>,
}

impl Default for StabilityTracker {
    fn default() -> Self {
        Self::new(StabilityConfig::default())
    }
}

impl StabilityTracker {
    pub fn new(cfg: StabilityConfig) -> Self {
        Self {
            cfg,
            fen: None,
            forced: false,
            history: Vec::new(),
        }
    }

    pub fn config(&self) -> StabilityConfig {
        self.cfg
    }

    /// Forget the current position.
    pub fn reset(&mut self) {
        self.fen = None;
        self.forced = false;
        self.history.clear();
    }

    /// Target depth to show before the first snapshot of `fen` arrives
    /// (1 for a forced move, otherwise `min_depth`).
    pub fn initial_target(&self, fen: &str) -> u32 {
        match parse_position(fen) {
            Ok(p) if p.legal_moves().len() <= 1 => 1,
            _ => self.cfg.min_depth,
        }
    }

    pub fn feed(&mut self, update: &EngineUpdate) -> Stability {
        if self.fen.as_deref() != Some(update.fen.as_str()) {
            self.reset();
            self.fen = Some(update.fen.clone());
            self.forced = parse_position(&update.fen).is_ok_and(|p| p.legal_moves().len() <= 1);
        }
        let depth = update.depth;
        let top = update
            .lines
            .iter()
            .find(|l| l.multipv == 1 && !l.pv.is_empty());
        if let Some(top) = top {
            let win = match top.score {
                Score::Cp(cp) => win_percent(cp),
                Score::Mate(m) if m > 0 => 100.0,
                Score::Mate(_) => 0.0,
            };
            let entry = (top.depth, top.pv[0].clone(), win);
            match self.history.last_mut() {
                Some(last) if last.0 == top.depth => *last = entry,
                Some(last) if last.0 > top.depth => {} // out-of-order older snapshot
                _ => self.history.push(entry),
            }
        }

        let mate = top.is_some_and(|t| matches!(t.score, Score::Mate(m) if m != 0));
        let (settled_for, swing) = self.settled_run();
        let reason = if self.forced {
            Some(StableReason::Forced)
        } else if mate {
            Some(StableReason::Mate)
        } else if update.done {
            Some(StableReason::Finished)
        } else if depth >= self.cfg.min_depth
            && settled_for >= self.cfg.stable_depths
            && swing <= self.cfg.max_swing_pct
        {
            Some(StableReason::Settled)
        } else {
            None
        };

        let mut target = self.cfg.min_depth;
        if self.forced {
            target = 1;
        } else if settled_for >= self.cfg.stable_depths * self.cfg.early_factor
            && swing <= self.cfg.max_swing_pct
            && depth >= self.cfg.floor_depth
        {
            // Quiet, clear-cut position: do not make the user wait for the full depth.
            target = target.min(depth);
        }
        Stability {
            stable: reason.is_some(),
            reason,
            depth,
            target_depth: target,
        }
    }

    /// Number of most-recent depths with the same best move, and the win% range over them.
    fn settled_run(&self) -> (usize, f64) {
        let Some((_, best, _)) = self.history.last() else {
            return (0, f64::INFINITY);
        };
        let run: Vec<&(u32, String, f64)> = self
            .history
            .iter()
            .rev()
            .take_while(|(_, mv, _)| mv == best)
            .collect();
        let (lo, hi) = run.iter().fold((f64::MAX, f64::MIN), |(lo, hi), e| {
            (lo.min(e.2), hi.max(e.2))
        });
        (run.len(), hi - lo)
    }
}

/// Destination square as drawn on the board (castling is `e1g1`, not king-takes-rook).
fn uci_to_square(uci: &str) -> String {
    uci.get(2..4).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn line(multipv: u32, score: Score, pv: &[&str]) -> RawLine {
        RawLine {
            multipv,
            depth: 20,
            seldepth: 24,
            score,
            nodes: 1000,
            nps: 1_000_000,
            pv: pv.iter().map(|s| (*s).to_owned()).collect(),
            wdl: None,
        }
    }

    fn update(fen: &str, lines: Vec<RawLine>, done: bool) -> EngineUpdate {
        EngineUpdate {
            fen: fen.to_owned(),
            lines,
            depth: 20,
            nps: 1_000_000,
            done,
        }
    }

    fn facts(fen: &str, uci: &str) -> Facts {
        let pos = parse_position(fen).unwrap();
        let mv = uci.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        tactic_facts(&pos, &mv)
    }

    fn explain_single(fen: &str, uci: &str, cp: i32) -> CandidateMove {
        let pos = parse_position(fen).unwrap();
        let sign = if pos.turn() == Color::White { 1 } else { -1 };
        let f = build_frame(
            &update(fen, vec![line(1, Score::Cp(cp * sign), &[uci])], true),
            0,
        )
        .unwrap();
        f.candidates.into_iter().next().unwrap()
    }

    #[test]
    fn win_percent_is_monotonic_and_symmetric() {
        assert!((win_percent(0) - 50.0).abs() < 1e-9);
        let mut prev = -1.0;
        for cp in (-1500..=1500).step_by(25) {
            let w = win_percent(cp);
            assert!(w > prev, "monotonic at {cp}");
            assert!(
                (w + win_percent(-cp) - 100.0).abs() < 1e-9,
                "symmetric at {cp}"
            );
            prev = w;
        }
        assert!(win_percent(100) > 59.0 && win_percent(100) < 60.0);
        assert!(win_percent(10_000) > 99.99);
    }

    #[test]
    fn classification_thresholds() {
        assert_eq!(classify_loss(0.0), MoveClass::Best);
        assert_eq!(classify_loss(0.04), MoveClass::Best);
        assert_eq!(classify_loss(0.5), MoveClass::Excellent);
        assert_eq!(classify_loss(2.0), MoveClass::Excellent);
        assert_eq!(classify_loss(2.01), MoveClass::Good);
        assert_eq!(classify_loss(5.0), MoveClass::Good);
        assert_eq!(classify_loss(5.01), MoveClass::Inaccuracy);
        assert_eq!(classify_loss(10.0), MoveClass::Inaccuracy);
        assert_eq!(classify_loss(10.01), MoveClass::Mistake);
        assert_eq!(classify_loss(20.0), MoveClass::Mistake);
        assert_eq!(classify_loss(20.01), MoveClass::Blunder);
    }

    #[test]
    fn frame_classifies_a_ladder_of_candidates() {
        let lines = vec![
            line(1, Score::Cp(60), &["e2e4", "e7e5"]),
            line(2, Score::Cp(50), &["d2d4"]),
            line(3, Score::Cp(-10), &["g1f3"]),
            line(4, Score::Cp(-100), &["c2c4"]),
            line(5, Score::Cp(-300), &["a2a3"]),
        ];
        let frame = build_frame(&update(START, lines, false), 42).unwrap();
        let cls: Vec<_> = frame.candidates.iter().map(|c| c.cls).collect();
        assert_eq!(cls[0], MoveClass::Best);
        assert_eq!(cls[1], MoveClass::Excellent);
        assert_eq!(cls[2], MoveClass::Inaccuracy);
        assert_eq!(cls[3], MoveClass::Mistake);
        assert_eq!(cls[4], MoveClass::Blunder);
        assert!(frame.candidates.iter().all(|c| c.loss_pct >= 0.0));
        assert!(frame.thinking);
        assert_eq!(frame.ts, 42);
        assert_eq!(frame.side_to_move, "w");
        assert_eq!(frame.eval_cp, Some(60));
        assert!(
            frame.candidates[4]
                .explanation
                .as_deref()
                .unwrap()
                .contains('%')
        );
    }

    #[test]
    fn white_pov_sign_handling_for_black_to_move() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let lines = vec![
            line(1, Score::Cp(30), &["e7e5"]),
            line(2, Score::Cp(-100), &["a7a6"]),
        ];
        let f = build_frame(&update(fen, lines, true), 0).unwrap();
        assert_eq!(f.side_to_move, "b");
        assert_eq!(f.eval_cp, Some(-30), "black +30 means white -30");
        assert!(f.win_pct < 50.0);
        assert_eq!(f.candidates[0].score_cp, Some(-30));
        assert_eq!(f.candidates[1].score_cp, Some(100));
        assert!(f.candidates[1].win_pct > 50.0, "white POV win%");
        // Loss is measured for the mover (black): second line is worse for black.
        assert!(f.candidates[1].loss_pct > 10.0);
        assert!(!f.thinking);

        let mated = build_frame(
            &update(fen, vec![line(1, Score::Mate(2), &["e7e5"])], true),
            0,
        )
        .unwrap();
        assert_eq!(
            mated.mate,
            Some(-2),
            "black mating in 2 is negative for white"
        );
        assert_eq!(mated.eval_cp, None);
        assert!(mated.win_pct < 1.0);
        assert_eq!(
            mated.candidates[0].explanation.as_deref(),
            Some("Starts a forced mate in 2.")
        );
    }

    #[test]
    fn san_conversion_and_pv_cap() {
        let pv = [
            "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4", "g8f6", "e1g1", "f8e7",
        ];
        let f = build_frame(&update(START, vec![line(1, Score::Cp(20), &pv)], true), 0).unwrap();
        let c = &f.candidates[0];
        assert_eq!(
            c.pv_san,
            ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6", "Ba4", "Nf6"]
        );
        assert_eq!(c.pv_uci.len(), MAX_PV_PLIES);
        assert_eq!(
            (
                c.uci.as_str(),
                c.san.as_str(),
                c.from.as_str(),
                c.to.as_str()
            ),
            ("e2e4", "e4", "e2", "e4")
        );
    }

    #[test]
    fn san_has_check_suffix_and_castling_destination() {
        let fen = "r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1";
        let f = build_frame(
            &update(fen, vec![line(1, Score::Cp(500), &["d5c7", "e8d7"])], true),
            0,
        )
        .unwrap();
        assert_eq!(f.candidates[0].san, "Nc7+");
        let castle = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        let f = build_frame(
            &update(castle, vec![line(1, Score::Cp(0), &["e1g1"])], true),
            0,
        )
        .unwrap();
        assert_eq!(f.candidates[0].san, "O-O");
        assert_eq!(f.candidates[0].to, "g1");
        assert!(f.candidates[0].tags.contains(&TacticTag::Castle));
    }

    #[test]
    fn illegal_first_move_and_bad_fen_are_errors() {
        let bad = update(START, vec![line(1, Score::Cp(0), &["e2e5"])], true);
        assert!(matches!(
            build_frame(&bad, 0),
            Err(EngineError::Protocol(_))
        ));
        let bad = update("garbage", vec![], true);
        assert!(matches!(
            build_frame(&bad, 0),
            Err(EngineError::InvalidFen(_))
        ));
    }

    #[test]
    fn terminal_positions_yield_empty_frames() {
        // Fool's mate: white is checkmated.
        let mated = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";
        let f = build_frame(&update(mated, vec![], true), 0).unwrap();
        assert!(f.candidates.is_empty());
        assert_eq!(f.mate, Some(0));
        assert_eq!(f.win_pct, 0.0);
        let stalemate = "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1";
        let f = build_frame(&update(stalemate, vec![], true), 0).unwrap();
        assert_eq!((f.eval_cp, f.mate), (Some(0), None));
    }

    #[test]
    fn royal_fork_is_tagged_and_explained() {
        let c = explain_single("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1", "d5c7", 500);
        assert!(c.tags.contains(&TacticTag::Fork), "{:?}", c.tags);
        assert!(c.tags.contains(&TacticTag::Check));
        assert_eq!(
            c.explanation.as_deref(),
            Some("Forks the king and the rook on a8.")
        );
    }

    #[test]
    fn fork_is_not_claimed_when_the_forker_hangs() {
        // The same knight jump, but a black queen on d8 simply takes it.
        let f = facts("r2qk3/8/8/3N4/8/8/8/4K3 w - - 0 1", "d5c7");
        assert!(!f.tags.contains(&TacticTag::Fork));
    }

    #[test]
    fn scholars_mate_is_mate() {
        let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
        let c = explain_single(fen, "h5f7", 10_000);
        assert!(c.tags.contains(&TacticTag::Mate));
        assert!(c.tags.contains(&TacticTag::Check));
        assert!(c.tags.contains(&TacticTag::Capture));
        assert_eq!(c.san, "Qxf7#");
        assert_eq!(c.explanation.as_deref(), Some("Delivers checkmate."));
    }

    #[test]
    fn simple_pin() {
        let c = explain_single("4k3/8/2n5/8/8/8/4B3/4K3 w - - 0 1", "e2b5", 50);
        assert!(c.tags.contains(&TacticTag::Pin), "{:?}", c.tags);
        assert_eq!(
            c.explanation.as_deref(),
            Some("Pins the knight on c6 to the king.")
        );
        assert!(!c.tags.contains(&TacticTag::Check));
    }

    #[test]
    fn skewer_through_the_king() {
        let f = facts("6kr/8/8/8/8/8/8/3R2K1 w - - 0 1", "d1d8");
        assert!(f.tags.contains(&TacticTag::Skewer), "{:?}", f.tags);
        assert!(f.tags.contains(&TacticTag::Check));
        assert!(!f.tags.contains(&TacticTag::Pin));
    }

    #[test]
    fn opposite_side_targets_are_not_pin_or_skewer() {
        // The rook attacks Ne6 above and Re1 below; nothing stands behind either target.
        let f = facts("6k1/8/4n3/8/R7/2KP4/8/4r3 w - - 0 1", "a4e4");
        assert!(f.tags.contains(&TacticTag::Fork), "{:?}", f.tags);
        assert!(!f.tags.contains(&TacticTag::Pin), "{:?}", f.tags);
        assert!(!f.tags.contains(&TacticTag::Skewer), "{:?}", f.tags);
    }

    #[test]
    fn hanging_piece_capture_and_attack() {
        let c = explain_single("4k3/8/8/3n4/8/1B6/8/4K3 w - - 0 1", "b3d5", 300);
        assert!(c.tags.contains(&TacticTag::Hanging) && c.tags.contains(&TacticTag::Capture));
        assert_eq!(
            c.explanation.as_deref(),
            Some("Wins the undefended knight.")
        );
        // Rook lift attacking an undefended bishop.
        let f = facts("4k3/8/1b6/8/8/8/8/R3K3 w - - 0 1", "a1a6");
        assert!(f.tags.contains(&TacticTag::Hanging), "{:?}", f.tags);
        assert!(!f.tags.contains(&TacticTag::Capture));
        // Defended knight: no hanging tag.
        let f = facts("4k3/8/2p5/3n4/8/1B6/8/4K3 w - - 0 1", "b3d5");
        assert!(!f.tags.contains(&TacticTag::Hanging));
    }

    #[test]
    fn discovered_check() {
        let f = facts("4k3/8/8/8/4N3/8/8/4R1K1 w - - 0 1", "e4c5");
        assert!(f.tags.contains(&TacticTag::Discovered));
        assert!(f.tags.contains(&TacticTag::Check));
        assert!(f.discovered_check);
    }

    #[test]
    fn special_move_tags() {
        let f = facts(
            "rnbqkbnr/ppp1pppp/8/8/3pP3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 3",
            "d4e3",
        );
        assert!(f.tags.contains(&TacticTag::EnPassant) && f.tags.contains(&TacticTag::Capture));
        let f = facts("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q");
        assert!(f.tags.contains(&TacticTag::Promotion));
        let c = explain_single("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q", 900);
        assert_eq!(c.explanation.as_deref(), Some("Promotes to a queen."));
    }

    #[test]
    fn see_values() {
        let pos = parse_position("4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv = "d1d5".parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        assert_eq!(see(pos.board(), &mv), -8, "queen for a defended pawn");
        let pos = parse_position("4k3/8/8/3p4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv = "d1d5".parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        assert_eq!(see(pos.board(), &mv), 1, "free pawn");
    }

    #[test]
    fn brilliant_requires_a_real_sacrifice_that_holds_up() {
        let fen = "4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1";
        let sac = line(1, Score::Cp(10), &["d1d5", "c6d5"]);
        let alt = line(2, Score::Cp(-400), &["e1d2"]);
        let f = build_frame(&update(fen, vec![sac, alt], true), 0).unwrap();
        assert!(f.candidates[0].tags.contains(&TacticTag::Sacrifice));
        assert_eq!(f.candidates[0].cls, MoveClass::Brilliant);

        // Already crushing with a quiet alternative: not brilliant.
        let sac = line(1, Score::Cp(900), &["d1d5", "c6d5"]);
        let alt = line(2, Score::Cp(890), &["e1d2"]);
        let f = build_frame(&update(fen, vec![sac, alt], true), 0).unwrap();
        assert_ne!(f.candidates[0].cls, MoveClass::Brilliant);

        // Sacrifice that leaves the mover clearly worse: not brilliant.
        let sac = line(1, Score::Cp(-300), &["d1d5", "c6d5"]);
        let f = build_frame(&update(fen, vec![sac], true), 0).unwrap();
        assert_ne!(f.candidates[0].cls, MoveClass::Brilliant);
    }

    #[test]
    fn great_is_an_only_move() {
        let lines = vec![
            line(1, Score::Cp(80), &["e2e4"]),
            line(2, Score::Cp(-250), &["a2a3"]),
        ];
        let f = build_frame(&update(START, lines, true), 0).unwrap();
        assert_eq!(f.candidates[0].cls, MoveClass::Great);
        assert_eq!(f.candidates[1].cls, MoveClass::Blunder);
        // Close runner-up: plain best.
        let lines = vec![
            line(1, Score::Cp(80), &["e2e4"]),
            line(2, Score::Cp(70), &["d2d4"]),
        ];
        let f = build_frame(&update(START, lines, true), 0).unwrap();
        assert_eq!(f.candidates[0].cls, MoveClass::Best);
        // A single candidate (or MultiPV 1) never claims "only move".
        let f = build_frame(
            &update(START, vec![line(1, Score::Cp(80), &["e2e4"])], true),
            0,
        )
        .unwrap();
        assert_eq!(f.candidates[0].cls, MoveClass::Best);
    }

    #[test]
    fn mate_lines_tie_at_best() {
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let lines = vec![
            line(1, Score::Mate(1), &["a1a8"]),
            line(2, Score::Mate(3), &["g1f1"]),
        ];
        let f = build_frame(&update(fen, lines, true), 0).unwrap();
        assert_eq!(f.candidates[0].mate, Some(1));
        assert_eq!(f.candidates[1].cls, MoveClass::Best);
        assert_eq!(f.candidates[1].loss_pct, 0.0);
    }

    #[test]
    fn wdl_is_converted_to_white_pov() {
        let mut l = line(1, Score::Cp(30), &["e7e5"]);
        l.wdl = Some([100, 300, 600]); // black to move: 10% win, 30% draw, 60% loss
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let f = build_frame(&update(fen, vec![l], true), 0).unwrap();
        assert_eq!(f.wdl, Some([600, 300, 100]));
        assert_eq!(f.target_depth, None);
        let f = build_frame(
            &update(START, vec![line(1, Score::Cp(30), &["e2e4"])], true),
            0,
        )
        .unwrap();
        assert_eq!(f.wdl, None);
    }

    fn snap(fen: &str, depth: u32, score: Score, mv: &str, done: bool) -> EngineUpdate {
        let mut l = line(1, score, &[mv]);
        l.depth = depth;
        EngineUpdate {
            fen: fen.to_owned(),
            lines: vec![l],
            depth,
            nps: 1,
            done,
        }
    }

    #[test]
    fn stability_needs_min_depth_and_consecutive_agreement() {
        let mut t = StabilityTracker::default();
        let mut stable_at = None;
        for d in 1..=20 {
            let mv = if d < 9 { "d2d4" } else { "e2e4" };
            let s = t.feed(&snap(START, d, Score::Cp(30), mv, false));
            if s.stable && stable_at.is_none() {
                stable_at = Some(d);
                assert_eq!(s.reason, Some(StableReason::Settled));
            }
        }
        // Best move changed at depth 9, so 3 agreeing depths exist from 11; min depth 12 gates.
        assert_eq!(stable_at, Some(12));
    }

    #[test]
    fn best_move_flip_or_big_swing_resets_stability() {
        let mut t = StabilityTracker::default();
        for d in 10..=14 {
            t.feed(&snap(START, d, Score::Cp(30), "e2e4", false));
        }
        assert!(
            t.feed(&snap(START, 15, Score::Cp(30), "e2e4", false))
                .stable
        );
        let s = t.feed(&snap(START, 16, Score::Cp(30), "d2d4", false));
        assert!(!s.stable, "new best move");
        // Same move but the evaluation jumps by far more than the allowed swing.
        let mut t = StabilityTracker::default();
        for (d, cp) in [(12, 0), (13, 150), (14, 300)] {
            let s = t.feed(&snap(START, d, Score::Cp(cp), "e2e4", false));
            assert!(!s.stable, "depth {d} swing too large");
        }
    }

    #[test]
    fn forced_move_is_immediately_stable() {
        // Ka1 is in check from Ra8 and Rh2 covers rank 2: Kb1 is the only legal move.
        let fen = "r6k/8/8/8/8/8/7r/K7 w - - 0 1";
        assert_eq!(parse_position(fen).unwrap().legal_moves().len(), 1);
        let mut t = StabilityTracker::default();
        assert_eq!(t.initial_target(fen), 1);
        let s = t.feed(&snap(fen, 1, Score::Cp(-900), "a1b1", false));
        assert_eq!(s.reason, Some(StableReason::Forced));
        assert_eq!(s.target_depth, 1);
        assert_eq!(t.initial_target(START), 12);
    }

    #[test]
    fn mate_and_finished_searches_are_stable() {
        let mut t = StabilityTracker::default();
        let fen = "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1";
        let s = t.feed(&snap(fen, 3, Score::Mate(1), "a1a8", false));
        assert_eq!(s.reason, Some(StableReason::Mate));
        let mut t = StabilityTracker::default();
        assert!(!t.feed(&snap(START, 5, Score::Cp(20), "e2e4", false)).stable);
        let s = t.feed(&snap(START, 6, Score::Cp(20), "e2e4", true));
        assert_eq!(s.reason, Some(StableReason::Finished));
    }

    #[test]
    fn quiet_positions_lower_the_target_and_new_fen_resets() {
        let mut t = StabilityTracker::default();
        let mut last = None;
        for d in 1..=11 {
            last = Some(t.feed(&snap(START, d, Score::Cp(25), "e2e4", false)));
        }
        let last = last.unwrap();
        assert!(!last.stable, "below min depth");
        assert_eq!(
            last.target_depth, 11,
            "settled for 2*K depths: target follows depth"
        );
        let other = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let s = t.feed(&snap(other, 2, Score::Cp(25), "e7e5", false));
        assert_eq!(s.target_depth, 12);
        assert!(!s.stable);
    }
}
