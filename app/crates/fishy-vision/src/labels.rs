//! Which side is at the bottom? Read it from chess.com's in-board coordinate labels.
//!
//! The default chess.com board draws the rank digit in the top-left corner of the left-column
//! squares and the file letter in the bottom-right corner of the bottom-row squares:
//!
//! | white at the bottom | black at the bottom |
//! | --- | --- |
//! | `8` top-left of the top-left square | `1` top-left of the top-left square |
//! | `h` bottom-right of the bottom-right square | `a` bottom-right of the bottom-right square |
//!
//! Both corners are classified against tiny glyph masks (`labels` in the built-in template file,
//! 20x24 alpha cells per glyph, never pixels of a screenshot) and cross-checked. A mask is an
//! alpha map: how much of a cell is covered by the label colour. The label colour is the colour of
//! the *other* square parity (measured on the board itself, so brightness changes, board themes
//! and highlights are followed), which makes the same mask work on light and dark squares and at
//! any scale: windows are fractions of the square side and cells are area averaged.
//!
//! Pixels that are neither background nor on the background-to-label colour line (a piece
//! touching the window, an arrow) are *unknown* and masked out of the comparison, like in piece
//! recognition. Anything unsure gives `None`: other sites, labels hidden or drawn outside the
//! board, a piece covering the glyph, two corners that disagree.

use crate::{
    capture::{Frame, VisionError},
    detect::{BoardRect, Site},
    recognize::{
        Orientation, Rgb, Templates, background, extract_square, hex_decode, hex_encode,
        square_colours,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Glyph grid (cells) of every window.
const GW: usize = 20;
const GH: usize = 24;

/// Windows as fractions of the square side: `[x0, y0, x1, y1]`.
pub(crate) const TL_WINDOW: [f32; 4] = [0.03, 0.05, 0.25, 0.32];
pub(crate) const BR_WINDOW: [f32; 4] = [0.77, 0.69, 0.97, 0.95];

/// Largest offset (in cells) tried between the observed label and the mask: the board rectangle
/// is only accurate to a pixel or two.
const SHIFT: i32 = 2;
/// Smallest summed alpha (cells) of an observed label: below it the corner is empty.
const MIN_INK: f32 = 5.0;
/// Both corners readable: minimum IoU and margin to the other glyph.
const MIN_SCORE: f32 = 0.5;
const MIN_MARGIN: f32 = 0.15;
/// A single readable corner (the other one hidden) must be much clearer.
const SOLO_SCORE: f32 = 0.66;
const SOLO_MARGIN: f32 = 0.30;
/// Fraction of a mask that must lie on known (not occluded) cells.
const MIN_COVERAGE: f32 = 0.75;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Corner {
    TopLeft,
    BottomRight,
}

impl Corner {
    fn window(self) -> [f32; 4] {
        match self {
            Corner::TopLeft => TL_WINDOW,
            Corner::BottomRight => BR_WINDOW,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredLabels {
    gw: usize,
    gh: usize,
    /// One hex string (`gw * gh` alpha bytes) per glyph: `8`, `1` (top-left), `h`, `a` (bottom-right).
    glyphs: BTreeMap<String, String>,
}

/// Parsed glyph masks.
pub(crate) struct LabelSet {
    glyphs: BTreeMap<char, Vec<f32>>,
}

impl LabelSet {
    fn parse(data: &serde_json::Value) -> Option<LabelSet> {
        let st: StoredLabels = serde_json::from_value(data.get("labels")?.clone()).ok()?;
        if st.gw != GW || st.gh != GH {
            return None;
        }
        let mut glyphs = BTreeMap::new();
        for (k, h) in &st.glyphs {
            let mut chars = k.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let b = hex_decode(h)?;
            if b.len() != GW * GH {
                return None;
            }
            glyphs.insert(ch, b.iter().map(|&v| v as f32 / 255.0).collect());
        }
        for needed in ['8', '1', 'h', 'a'] {
            glyphs.get(&needed)?;
        }
        Some(LabelSet { glyphs })
    }

    fn pair(&self, corner: Corner) -> [(char, &[f32]); 2] {
        let (a, b) = match corner {
            Corner::TopLeft => ('8', '1'),
            Corner::BottomRight => ('h', 'a'),
        };
        [(a, &self.glyphs[&a][..]), (b, &self.glyphs[&b][..])]
    }
}

/// Label masks bundled for a site's default look (chess.com only).
fn builtin_labels(site: Site) -> Option<&'static LabelSet> {
    static CHESSCOM: OnceLock<Option<LabelSet>> = OnceLock::new();
    match site {
        Site::ChessCom => CHESSCOM
            .get_or_init(|| {
                let data = serde_json::from_str(include_str!("builtin/chesscom.json")).ok()?;
                LabelSet::parse(&data)
            })
            .as_ref(),
        Site::Lichess | Site::Generic => None,
    }
}

// ---------------------------------------------------------------------------
// observation
// ---------------------------------------------------------------------------

/// Label alpha of a window: `alpha[i]` is meaningful where `known[i]`.
pub(crate) struct Ink {
    pub alpha: Vec<f32>,
    pub known: Vec<bool>,
}

/// Area-averaged colours of `win` (fractions of the square side) of screen square (`r`, `c`),
/// as a `GW x GH` grid.
pub(crate) fn window_cells(
    frame: &Frame,
    rect: &BoardRect,
    r: usize,
    c: usize,
    win: [f32; 4],
) -> Vec<Rgb> {
    let s = rect.size / 8.0;
    let (x0, y0) = (
        rect.x + (c as f32 + win[0]) * s,
        rect.y + (r as f32 + win[1]) * s,
    );
    let (cw, ch) = (
        (win[2] - win[0]) * s / GW as f32,
        (win[3] - win[1]) * s / GH as f32,
    );
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let mut out = Vec::with_capacity(GW * GH);
    for j in 0..GH {
        let ya = ((y0 + j as f32 * ch).floor() as i32).clamp(0, fh - 1);
        let yb = ((y0 + (j + 1) as f32 * ch).ceil() as i32).clamp(ya + 1, fh);
        for i in 0..GW {
            let xa = ((x0 + i as f32 * cw).floor() as i32).clamp(0, fw - 1);
            let xb = ((x0 + (i + 1) as f32 * cw).ceil() as i32).clamp(xa + 1, fw);
            let mut sum = [0u32; 3];
            for y in ya..yb {
                let row = &frame.rgba[(y as usize * fw as usize + xa as usize) * 4
                    ..(y as usize * fw as usize + xb as usize) * 4];
                for p in row.chunks_exact(4) {
                    sum[0] += p[0] as u32;
                    sum[1] += p[1] as u32;
                    sum[2] += p[2] as u32;
                }
            }
            let n = ((yb - ya) * (xb - xa)) as f32;
            out.push([sum[0] as f32 / n, sum[1] as f32 / n, sum[2] as f32 / n]);
        }
    }
    out
}

/// Coverage of each cell by the label colour, assuming it blends `bg` towards `label`.
pub(crate) fn ink_of(cells: &[Rgb], bg: Rgb, label: Rgb) -> Option<Ink> {
    let d = [label[0] - bg[0], label[1] - bg[1], label[2] - bg[2]];
    let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if dd < 24.0 * 24.0 {
        // The label colour is indistinguishable from the square: nothing to read.
        return None;
    }
    let mut alpha = Vec::with_capacity(cells.len());
    let mut known = Vec::with_capacity(cells.len());
    for p in cells {
        let e = [p[0] - bg[0], p[1] - bg[1], p[2] - bg[2]];
        let ee = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
        let t = (e[0] * d[0] + e[1] * d[1] + e[2] * d[2]) / dd;
        let resid = (ee - t * t * dd).max(0.0).sqrt();
        let mag = ee.sqrt();
        if mag <= 9.0 {
            alpha.push(0.0);
            known.push(true);
        } else if resid <= 11.0 + 0.12 * mag && (-0.12..=1.2).contains(&t) {
            alpha.push(t.clamp(0.0, 1.0));
            known.push(true);
        } else {
            alpha.push(0.0);
            known.push(false);
        }
    }
    Some(Ink { alpha, known })
}

/// Best masked soft-IoU of `ink` against `mask` over small shifts, and the mask's coverage.
fn best_match(ink: &Ink, mask: &[f32]) -> f32 {
    let total: f32 = mask.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut best = 0.0f32;
    for dy in -SHIFT..=SHIFT {
        for dx in -SHIFT..=SHIFT {
            let (mut inter, mut uni, mut seen) = (0.0f32, 0.0f32, 0.0f32);
            for j in 0..GH as i32 {
                for i in 0..GW as i32 {
                    let o = (j as usize) * GW + i as usize;
                    if !ink.known[o] {
                        continue;
                    }
                    let (ti, tj) = (i + dx, j + dy);
                    let m = if (0..GW as i32).contains(&ti) && (0..GH as i32).contains(&tj) {
                        mask[tj as usize * GW + ti as usize]
                    } else {
                        0.0
                    };
                    let a = ink.alpha[o];
                    inter += a.min(m);
                    uni += a.max(m);
                    seen += m;
                }
            }
            // Cells of the mask that shifted outside the window never count as seen.
            if uni > 0.0 && seen / total >= MIN_COVERAGE {
                best = best.max(inter / uni);
            }
        }
    }
    best
}

/// Outcome of looking at one corner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CornerRead {
    pub glyph: char,
    pub score: f32,
    pub margin: f32,
}

/// Board-wide colours used by every corner read.
pub(crate) struct BoardColours {
    /// Normal colour of the (even, odd) parity squares.
    pub normal: [Rgb; 2],
    pub bgs: Vec<Rgb>,
}

pub(crate) fn board_colours(frame: &Frame, rect: &BoardRect) -> BoardColours {
    let mut bgs = Vec::with_capacity(64);
    let mut patch = Vec::with_capacity(32 * 32);
    for r in 0..8 {
        for c in 0..8 {
            extract_square(frame, rect, r, c, &mut patch);
            bgs.push(background(&patch));
        }
    }
    BoardColours {
        normal: square_colours(&bgs),
        bgs,
    }
}

/// Label-colour candidates of square (`r`, `c`): the other parity's colour as it is (the label is
/// drawn over a highlight), and as a highlight laid over the label dims it (the label then keeps a
/// share `k` of its contrast to the square: `bg' + k (label - normal square)`).
pub(crate) fn label_candidates(cols: &BoardColours, r: usize, c: usize) -> Vec<Rgb> {
    let par = (r + c) % 2;
    let (own, other) = (cols.normal[par], cols.normal[1 - par]);
    let bg = cols.bgs[r * 8 + c];
    let mut out = vec![other];
    let tinted = (0..3).map(|k| (bg[k] - own[k]).abs()).sum::<f32>() > 12.0;
    if tinted {
        for k in [0.5f32, 0.7] {
            out.push([
                (bg[0] + k * (other[0] - own[0])).clamp(0.0, 255.0),
                (bg[1] + k * (other[1] - own[1])).clamp(0.0, 255.0),
                (bg[2] + k * (other[2] - own[2])).clamp(0.0, 255.0),
            ]);
        }
    }
    out
}

fn classify_corner(
    frame: &Frame,
    rect: &BoardRect,
    cols: &BoardColours,
    set: &LabelSet,
    r: usize,
    c: usize,
    corner: Corner,
) -> Option<CornerRead> {
    let cells = window_cells(frame, rect, r, c, corner.window());
    let bg = cols.bgs[r * 8 + c];
    let pair = set.pair(corner);
    let mut best: Option<CornerRead> = None;
    for label in label_candidates(cols, r, c) {
        let Some(ink) = ink_of(&cells, bg, label) else {
            continue;
        };
        let seen: f32 = ink
            .alpha
            .iter()
            .zip(&ink.known)
            .filter(|(_, k)| **k)
            .map(|(a, _)| a)
            .sum();
        if seen < MIN_INK {
            continue;
        }
        let scores = [best_match(&ink, pair[0].1), best_match(&ink, pair[1].1)];
        let (hi, lo, which) = if scores[0] >= scores[1] {
            (scores[0], scores[1], 0)
        } else {
            (scores[1], scores[0], 1)
        };
        let read = CornerRead {
            glyph: pair[which].0,
            score: hi,
            margin: hi - lo,
        };
        if best.is_none_or(|b| read.score > b.score) {
            best = Some(read);
        }
    }
    best
}

/// Read the two corner labels against `set` and combine them. Both readable and equal gives the
/// orientation, both readable and different or nothing readable gives `None`, one readable gives
/// the orientation only when it is very clear.
fn orientation_from_labels(frame: &Frame, rect: &BoardRect, set: &LabelSet) -> Option<Orientation> {
    if rect.size < 128.0
        || rect.x < 0.0
        || rect.y < 0.0
        || rect.x + rect.size > frame.width as f32 + 1.0
        || rect.y + rect.size > frame.height as f32 + 1.0
        || frame.rgba.len() < (frame.width as usize) * (frame.height as usize) * 4
    {
        return None;
    }
    let cols = board_colours(frame, rect);
    let tl = classify_corner(frame, rect, &cols, set, 0, 0, Corner::TopLeft);
    let br = classify_corner(frame, rect, &cols, set, 7, 7, Corner::BottomRight);
    let side = |g: char| match g {
        '8' | 'h' => Orientation::White,
        _ => Orientation::Black,
    };
    let sure = |r: &CornerRead| r.score >= MIN_SCORE && r.margin >= MIN_MARGIN;
    let solo = |r: &CornerRead| r.score >= SOLO_SCORE && r.margin >= SOLO_MARGIN;
    match (tl.filter(sure), br.filter(sure)) {
        (Some(a), Some(b)) => (side(a.glyph) == side(b.glyph)).then(|| side(a.glyph)),
        (Some(a), None) => {
            // The other corner must not contradict it, even weakly.
            let contradicted =
                br.is_some_and(|b| b.margin >= 0.1 && side(b.glyph) != side(a.glyph));
            (solo(&a) && !contradicted).then(|| side(a.glyph))
        }
        (None, Some(b)) => {
            let contradicted =
                tl.is_some_and(|a| a.margin >= 0.1 && side(a.glyph) != side(b.glyph));
            (solo(&b) && !contradicted).then(|| side(b.glyph))
        }
        (None, None) => None,
    }
}

/// Orientation (colour at the bottom of the screen) from the chess.com in-board coordinate
/// labels, or `None` when unsure: another site, labels hidden or drawn outside the board, a
/// piece or arrow covering them, or the two corners disagreeing.
///
/// Works at any scale and on both square colours, follows brightness changes and highlights, and
/// never needs calibration: the glyph masks are bundled with the built-in chess.com templates.
pub fn detect_orientation(frame: &Frame, rect: &BoardRect, site: Site) -> Option<Orientation> {
    orientation_from_labels(frame, rect, builtin_labels(site)?)
}

/// Like [`detect_orientation`] with the glyph masks stored in `templates` (see [`learn_labels`]);
/// `None` when `templates` carries none.
pub fn detect_orientation_with(
    frame: &Frame,
    rect: &BoardRect,
    templates: &Templates,
) -> Option<Orientation> {
    orientation_from_labels(frame, rect, &LabelSet::parse(&templates.data)?)
}

// ---------------------------------------------------------------------------
// learning (used by examples/make_builtin.rs)
// ---------------------------------------------------------------------------

/// Alpha map of one corner glyph of the square at screen position (`r`, `c`), measured against
/// the square's own background and the board's other square colour.
pub(crate) fn corner_alpha(
    frame: &Frame,
    rect: &BoardRect,
    cols: &BoardColours,
    r: usize,
    c: usize,
    corner: Corner,
) -> Option<Vec<f32>> {
    let cells = window_cells(frame, rect, r, c, corner.window());
    let ink = ink_of(&cells, cols.bgs[r * 8 + c], label_candidates(cols, r, c)[0])?;
    // Keep the glyph only: the largest connected blob of ink. Piece edges at the border of the
    // window are other blobs and must not become part of the mask.
    let mut blob = vec![0usize; GW * GH];
    let (mut best, mut best_size, mut next) = (0usize, 0usize, 1usize);
    for start in 0..GW * GH {
        if blob[start] != 0 || ink.alpha[start] <= 0.15 {
            continue;
        }
        let (mut stack, mut size) = (vec![start], 0usize);
        blob[start] = next;
        while let Some(o) = stack.pop() {
            size += 1;
            let (j, i) = ((o / GW) as i32, (o % GW) as i32);
            for (dj, di) in (-1..=1).flat_map(|a| (-1..=1).map(move |b| (a, b))) {
                let (tj, ti) = (j + dj, i + di);
                if (0..GH as i32).contains(&tj) && (0..GW as i32).contains(&ti) {
                    let t = tj as usize * GW + ti as usize;
                    if blob[t] == 0 && ink.alpha[t] > 0.15 {
                        blob[t] = next;
                        stack.push(t);
                    }
                }
            }
        }
        if size > best_size {
            (best, best_size) = (next, size);
        }
        next += 1;
    }
    if best == 0 {
        return None;
    }
    // The glyph itself must not be partly hidden (unknown cells next to it).
    for (o, &b) in blob.iter().enumerate() {
        if b != best || ink.alpha[o] < 0.5 {
            continue;
        }
        let (j, i) = ((o / GW) as i32, (o % GW) as i32);
        for (dj, di) in (-1..=1).flat_map(|a| (-1..=1).map(move |b| (a, b))) {
            let (tj, ti) = (j + dj, i + di);
            if (0..GH as i32).contains(&tj)
                && (0..GW as i32).contains(&ti)
                && !ink.known[tj as usize * GW + ti as usize]
            {
                return None;
            }
        }
    }
    Some(
        (0..GW * GH)
            .map(|o| if blob[o] == best { ink.alpha[o] } else { 0.0 })
            .collect(),
    )
}

/// Learn the four glyph masks (`8`, `1`, `h`, `a`) from a screenshot with a known orientation and
/// store them in `templates` under `labels`. `orientation` says which colour is at the bottom of
/// the screen. Existing piece data is untouched, so the extended file still loads everywhere.
pub fn learn_labels(
    frame: &Frame,
    rect: &BoardRect,
    orientation: Orientation,
    templates: &mut Templates,
) -> Result<(), VisionError> {
    let fail = |m: &str| VisionError::Other(format!("coordinate labels not learned: {m}"));
    if rect.size < 256.0 {
        return Err(fail("the board is too small"));
    }
    let cols = board_colours(frame, rect);
    // Which glyph sits where on the screen.
    let spots: [(usize, usize, Corner, char); 4] = match orientation {
        Orientation::White => [
            (0, 0, Corner::TopLeft, '8'),
            (7, 0, Corner::TopLeft, '1'),
            (7, 7, Corner::BottomRight, 'h'),
            (7, 0, Corner::BottomRight, 'a'),
        ],
        Orientation::Black => [
            (0, 0, Corner::TopLeft, '1'),
            (7, 0, Corner::TopLeft, '8'),
            (7, 7, Corner::BottomRight, 'a'),
            (7, 0, Corner::BottomRight, 'h'),
        ],
    };
    let mut glyphs = BTreeMap::new();
    for (r, c, corner, ch) in spots {
        let alpha = corner_alpha(frame, rect, &cols, r, c, corner)
            .ok_or_else(|| fail(&format!("glyph {ch} is covered or unreadable")))?;
        if alpha.iter().sum::<f32>() < MIN_INK {
            return Err(fail(&format!("no label found for {ch}")));
        }
        let bytes: Vec<u8> = alpha.iter().map(|a| (a * 255.0).round() as u8).collect();
        glyphs.insert(ch.to_string(), hex_encode(&bytes));
    }
    let stored = StoredLabels {
        gw: GW,
        gh: GH,
        glyphs,
    };
    let value = serde_json::to_value(&stored).map_err(|e| VisionError::Other(e.to_string()))?;
    match templates.data.as_object_mut() {
        Some(o) => {
            o.insert("labels".to_string(), value);
            Ok(())
        }
        None => Err(fail("templates are not an object")),
    }
}

/// Test hook: classify one corner glyph of any square with the built-in masks.
#[cfg(test)]
pub(crate) fn classify_builtin_corner(
    frame: &Frame,
    rect: &BoardRect,
    r: usize,
    c: usize,
    corner: Corner,
) -> Option<CornerRead> {
    let set = builtin_labels(Site::ChessCom)?;
    let cols = board_colours(frame, rect);
    classify_corner(frame, rect, &cols, set, r, c, corner)
}
