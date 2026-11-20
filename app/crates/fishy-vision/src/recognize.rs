//! Piece recognition learned from the user's own screen.
//!
//! No artwork is bundled. When the standard start position is on screen,
//! [`learn_from_start`] looks at the 32 pieces and builds, for the user's piece
//! set and board theme:
//!
//! * a **palette** of the colours the pieces are painted with (fills, outlines),
//!   split into a light and a dark class by luma, and
//! * one **template per piece** (12 of them): for every cell of a 32x32 grid the
//!   fraction of instances in which that cell was a light / a dark piece pixel.
//!
//! # Reading a square
//!
//! The square is resampled to 32x32 and the *background colour* is the median
//! of its border ring (so last-move/selected highlights are simply a different
//! background). Every cell is then classified by nearest colour among
//! {background, palette colours}:
//!
//! * `bg`   - close to the background, or a darker shade of it (legal-move
//!   dots, capture rings and premove tints are translucent black);
//! * `light` / `dark` piece pixel - close to a palette colour;
//! * `unknown` - anything else: arrows (Fishy's own included), coordinate
//!   letters, anti-aliased edges, drag ghosts.
//!
//! Unknown cells are *masked out* of the comparison instead of being treated as
//! background or piece, which is what makes arrows and labels harmless. A square
//! is empty if hardly any piece pixel is present, otherwise the piece with the
//! best masked IoU against the 12 templates (tolerating +-1 cell of offset)
//! wins; confidence comes from the IoU, the margin to the runner-up and how much
//! of the template is visible. Templates are background independent (they are
//! based on piece colours, not on difference to a particular square), so a piece
//! learned on a light square is recognised on a dark one and vice versa.
//!
//! # Promotions and unseen pieces
//!
//! All twelve kinds occur in the start position, so a promoted queen/rook/
//! bishop/knight matches its own template on any square. If a piece matches no
//! template well (a piece set with animations, partial occlusion, ...) a
//! **shape-feature fallback** classifies the silhouette (bounding box, fill
//! ratio and widths at three heights, compared with the learned kinds) and the
//! side from the light/dark pixel ratio; such squares get a low confidence. The
//! tracker additionally resolves a promotion piece by legality (queen preferred
//! when the picture is ambiguous), see `tracker.rs`.
//!
//! Learn from a start position shown without arrows when possible; arrows over
//! the 32 pieces are tolerated (palette colours must occur on many squares, and
//! templates are averaged over instances) but weaken the single-instance
//! queen/king templates.

use crate::{
    capture::{Frame, VisionError},
    detect::{BoardRect, Site},
};
use serde::{Deserialize, Serialize};

/// 64 chars, index 0 = a8 ... index 63 = h1 in FEN order for WHITE-at-bottom
/// orientation (always normalised to board coordinates, never screen coordinates).
/// 'PNBRQK' white, 'pnbrqk' black, '.' empty, '?' unknown.
pub type Placement = [char; 64];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    White,
    Black,
}

/// Piece templates learned from the user's own screen (see `learn_from_start`),
/// so recognition works for any piece set/theme and needs no bundled artwork.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Templates {
    pub data: serde_json::Value,
}

impl Templates {
    /// Templates bundled for a site's default look, so no calibration is needed.
    ///
    /// Only `ChessCom` (default green board, default "Neo" pieces) has any. They are 32x32
    /// colour-class masks derived from a screenshot (a few KB), not the screenshot itself, and
    /// are matched scale- and theme-independently like learned templates. Other sites and piece
    /// sets still need calibration.
    pub fn builtin(site: Site) -> Option<Templates> {
        let json = match site {
            Site::ChessCom => include_str!("builtin/chesscom.json"),
            Site::Lichess | Site::Generic => return None,
        };
        let data = serde_json::from_str(json).ok()?;
        let t = Templates { data };
        Recognizer::new(&t).map(|_| t)
    }
}

#[derive(Debug, Clone)]
pub struct Reading {
    pub placement: Placement,
    pub orientation: Orientation,
    /// 0..1, mean per-square confidence.
    pub confidence: f32,
    /// Colour ('w' or 'b') of the side that just moved, read from the last-move highlight
    /// (two tinted squares, one now empty, the other holding the moved piece). `None` when there
    /// is no such highlight or it is ambiguous. A lone tinted piece (a selected piece) never counts.
    pub mover_hint: Option<char>,
    /// Last move as (from, to) board indices (0 = a8), when `mover_hint` is known.
    pub last_move_squares: Option<(u8, u8)>,
}

pub(crate) type Rgb = [f32; 3];

/// Resampled square grid size.
const N: usize = 32;
/// Piece order used for templates.
const PIECES: [char; 12] = ['P', 'N', 'B', 'R', 'Q', 'K', 'p', 'n', 'b', 'r', 'q', 'k'];
const STORE_VERSION: u32 = 1;

/// Cell classes.
const BG: u8 = 0;
const LIGHT: u8 = 1;
const DARK: u8 = 2;
const UNKNOWN: u8 = 3;

/// Distance gates (RGB euclidean) for background / palette matches.
const BG_GATE: f32 = 26.0;
const PAL_GATE: f32 = 30.0;
/// Maximum distance of an edge pixel from the line between the background and a piece colour.
const BLEND_GATE: f32 = 13.0;

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    n: usize,
    /// Palette colours with their light (true) / dark class.
    palette: Vec<([u8; 3], bool)>,
    /// Hex strings, one per entry of `PIECES`: light channel then dark channel, u8 each.
    pieces: Vec<String>,
    /// Background colour of the (even, odd) parity squares when the templates were learned.
    /// Used to undo global brightness/tint changes before classification.
    #[serde(default)]
    squares: Option<[[u8; 3]; 2]>,
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

#[inline]
fn d2(a: Rgb, b: Rgb) -> f32 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    x * x + y * y + z * z
}

#[inline]
fn luma(c: Rgb) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

pub(crate) fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    let (_, m, _) = v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    *m
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    b.chunks_exact(2)
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

/// Board index (0 = a8) of the square at screen row `r`, column `c`.
#[inline]
fn board_index(o: Orientation, r: usize, c: usize) -> usize {
    match o {
        Orientation::White => r * 8 + c,
        Orientation::Black => 63 - (r * 8 + c),
    }
}

// ---------------------------------------------------------------------------
// square extraction and classification
// ---------------------------------------------------------------------------

/// Area-average resample of screen square (`r`, `c`) to N x N cells.
pub(crate) fn extract_square(
    frame: &Frame,
    rect: &BoardRect,
    r: usize,
    c: usize,
    out: &mut Vec<Rgb>,
) {
    let s = rect.size / 8.0;
    let (x0, y0) = (rect.x + c as f32 * s, rect.y + r as f32 * s);
    let cell = s / N as f32;
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    out.clear();
    for j in 0..N {
        let ya = ((y0 + j as f32 * cell).floor() as i32).clamp(0, fh - 1);
        let yb = ((y0 + (j + 1) as f32 * cell).ceil() as i32).clamp(ya + 1, fh);
        for i in 0..N {
            let xa = ((x0 + i as f32 * cell).floor() as i32).clamp(0, fw - 1);
            let xb = ((x0 + (i + 1) as f32 * cell).ceil() as i32).clamp(xa + 1, fw);
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
}

/// Cell distances from the square's edge that form the background ring.
const BG_RING: (usize, usize) = (2, 4);

/// Background colour of a square: per-channel median of its border ring.
pub(crate) fn background(patch: &[Rgb]) -> Rgb {
    let mut ch: [Vec<f32>; 3] = [
        Vec::with_capacity(240),
        Vec::with_capacity(240),
        Vec::with_capacity(240),
    ];
    for j in 0..N {
        for i in 0..N {
            // Two cells wide, one cell inside the edge: a board rectangle that is off by a pixel or
            // two must not make the neighbouring square (or the page) part of the background.
            let edge = i.min(j).min(N - 1 - i).min(N - 1 - j);
            if (BG_RING.0..BG_RING.1).contains(&edge) {
                let p = patch[j * N + i];
                for k in 0..3 {
                    ch[k].push(p[k]);
                }
            }
        }
    }
    [median(&mut ch[0]), median(&mut ch[1]), median(&mut ch[2])]
}

/// Is `p` the background or a translucent-black darkening of it (dots, rings)?
#[inline]
fn bg_like(p: Rgb, bg: Rgb, gate2: f32) -> bool {
    if d2(p, bg) <= gate2 {
        return true;
    }
    let bb = bg[0] * bg[0] + bg[1] * bg[1] + bg[2] * bg[2];
    if bb < 1.0 {
        return false;
    }
    let lam = (p[0] * bg[0] + p[1] * bg[1] + p[2] * bg[2]) / bb;
    if !(0.72..=1.03).contains(&lam) {
        return false;
    }
    d2(p, [bg[0] * lam, bg[1] * lam, bg[2] * lam]) <= 12.0 * 12.0
}

#[derive(Clone)]
struct Palette {
    colours: Vec<(Rgb, bool)>,
}

impl Palette {
    /// Class of a pixel given the square background.
    #[inline]
    fn classify(&self, p: Rgb, bg: Rgb) -> u8 {
        // Background (or a translucent darkening of it) wins over any palette
        // colour: anti-aliased piece edges leave palette entries that resemble
        // a shaded square.
        if bg_like(p, bg, BG_GATE * BG_GATE) {
            return BG;
        }
        let (mut dc, mut light) = (f32::MAX, false);
        for (c, l) in &self.colours {
            let d = d2(p, *c);
            if d < dc {
                dc = d;
                light = *l;
            }
        }
        if dc <= PAL_GATE * PAL_GATE {
            return if light { LIGHT } else { DARK };
        }
        // Anti-aliased edge (or resampling ringing): a mix of the background and one piece
        // colour. Whichever dominates decides, so edges stay stable at any board size.
        let (mut best, mut cls) = (BLEND_GATE * BLEND_GATE, UNKNOWN);
        for (c, l) in &self.colours {
            let d = [c[0] - bg[0], c[1] - bg[1], c[2] - bg[2]];
            let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if dd < 40.0 * 40.0 {
                continue;
            }
            let v = [p[0] - bg[0], p[1] - bg[1], p[2] - bg[2]];
            let a = (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / dd;
            if !(0.1..=1.0).contains(&a) {
                continue;
            }
            let r = d2(v, [a * d[0], a * d[1], a * d[2]]);
            if r < best {
                best = r;
                cls = if a < 0.5 {
                    BG
                } else if *l {
                    LIGHT
                } else {
                    DARK
                };
            }
        }
        cls
    }
}

fn classify_patch(patch: &[Rgb], bg: Rgb, pal: &Palette, out: &mut Vec<u8>) {
    out.clear();
    out.extend(patch.iter().map(|&p| pal.classify(p, bg)));
}

/// Window of cells used for the occupancy decision.
const W0: usize = 4;
const W1: usize = N - 4;

// ---------------------------------------------------------------------------
// palette learning
// ---------------------------------------------------------------------------

/// Greedy colour clustering. `samples` are (colour, square id); a cluster is kept
/// only when it occurs on at least `min_squares` different squares (rejects arrows).
fn learn_palette(samples: &[(Rgb, u8)], min_squares: usize) -> Vec<(Rgb, f32)> {
    let total = samples.len();
    let mut remaining: Vec<(Rgb, u8)> = samples.to_vec();
    let mut out = Vec::new();
    while out.len() < 12 && remaining.len() as f32 > 0.015 * total as f32 {
        // Densest 16-level bin neighbourhood.
        const B: usize = 16;
        let mut cnt = vec![0u32; B * B * B];
        for (p, _) in &remaining {
            cnt[((p[0] as usize >> 4) * B + (p[1] as usize >> 4)) * B + (p[2] as usize >> 4)] += 1;
        }
        let (mut best, mut best_bin) = (0u32, 0usize);
        for r in 0..B {
            for g in 0..B {
                for b in 0..B {
                    if cnt[(r * B + g) * B + b] == 0 {
                        continue;
                    }
                    let mut s = 0;
                    for dr in r.saturating_sub(1)..=(r + 1).min(B - 1) {
                        for dg in g.saturating_sub(1)..=(g + 1).min(B - 1) {
                            for db in b.saturating_sub(1)..=(b + 1).min(B - 1) {
                                s += cnt[(dr * B + dg) * B + db];
                            }
                        }
                    }
                    if s > best {
                        best = s;
                        best_bin = (r * B + g) * B + b;
                    }
                }
            }
        }
        let seed_bin = [
            (best_bin / (B * B)) as f32 * 16.0 + 8.0,
            ((best_bin / B) % B) as f32 * 16.0 + 8.0,
            (best_bin % B) as f32 * 16.0 + 8.0,
        ];
        let mut centre = seed_bin;
        for _ in 0..3 {
            let (mut sum, mut n) = ([0.0f32; 3], 0.0);
            for (p, _) in &remaining {
                if d2(*p, centre) <= 24.0 * 24.0 {
                    for k in 0..3 {
                        sum[k] += p[k];
                    }
                    n += 1.0;
                }
            }
            if n == 0.0 {
                break;
            }
            centre = [sum[0] / n, sum[1] / n, sum[2] / n];
        }
        let mut squares = [false; 64];
        let mut n = 0usize;
        remaining.retain(|(p, sq)| {
            if d2(*p, centre) <= 24.0 * 24.0 {
                squares[*sq as usize] = true;
                n += 1;
                false
            } else {
                true
            }
        });
        if n == 0 {
            break;
        }
        if squares.iter().filter(|&&b| b).count() >= min_squares {
            out.push((centre, n as f32));
        }
    }
    out
}

/// Otsu-style luma split of the palette (weighted by pixel count).
fn luma_classes(pal: &[(Rgb, f32)]) -> Vec<(Rgb, bool)> {
    let mut best = (f32::MIN, 128.0f32);
    let total: f32 = pal.iter().map(|p| p.1).sum();
    for (c, _) in pal {
        let th = luma(*c);
        let (mut w0, mut s0, mut w1, mut s1) = (0.0, 0.0, 0.0, 0.0);
        for (c2, w) in pal {
            if luma(*c2) <= th {
                w0 += w;
                s0 += w * luma(*c2);
            } else {
                w1 += w;
                s1 += w * luma(*c2);
            }
        }
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (s0 / w0, s1 / w1);
        let between = (w0 / total) * (w1 / total) * (m0 - m1) * (m0 - m1);
        if between > best.0 {
            best = (between, (m0 + m1) / 2.0);
        }
    }
    pal.iter().map(|(c, _)| (*c, luma(*c) > best.1)).collect()
}

// ---------------------------------------------------------------------------
// learned recogniser
// ---------------------------------------------------------------------------

struct PieceTemplate {
    light: Vec<f32>,
    dark: Vec<f32>,
    total: f32,
    /// Silhouette features of the template (None if the template is empty).
    feat: Option<[f32; 6]>,
}

/// Ready-to-use recogniser built from [`Templates`]. Parsing the JSON once and
/// reusing the recogniser avoids re-decoding the templates for every frame.
pub struct Recognizer {
    palette: Palette,
    pieces: Vec<PieceTemplate>,
    /// Light-pixel ratio of white / black pieces (for the side fallback).
    side_ratio: [f32; 2],
    /// Square colours seen at learning time (see `Stored::squares`).
    square_refs: Option<[Rgb; 2]>,
}

/// Median background colour of the (even, odd) parity squares; `bgs` is in row-major order.
pub(crate) fn square_colours(bgs: &[Rgb]) -> [Rgb; 2] {
    let mut out = [[0.0f32; 3]; 2];
    for (parity, o) in out.iter_mut().enumerate() {
        for (k, v) in o.iter_mut().enumerate() {
            let mut vals: Vec<f32> = bgs
                .iter()
                .enumerate()
                .filter(|(sq, _)| ((sq / 8) + (sq % 8)) % 2 == parity)
                .map(|(_, c)| c[k])
                .collect();
            *v = median(&mut vals);
        }
    }
    out
}

/// Per-channel gain that maps the observed square colours back onto the learned ones, or
/// `None` when the board looks the same (or too different to be a mere brightness/tint change,
/// e.g. another board theme).
fn global_gain(reference: &[Rgb; 2], now: &[Rgb; 2]) -> Option<[f32; 3]> {
    let mut gain = [1.0f32; 3];
    for k in 0..3 {
        let (rl, rd, cl, cd) = (reference[0][k], reference[1][k], now[0][k], now[1][k]);
        if cl < 8.0 || cd < 8.0 {
            // Near-black squares carry no usable ratio.
            if (rl - cl).abs() > 6.0 || (rd - cd).abs() > 6.0 {
                return None;
            }
            continue;
        }
        let (gl, gd) = (rl / cl, rd / cd);
        if (gl - gd).abs() > 0.15 * gl.max(gd) {
            return None;
        }
        gain[k] = ((rl * cl + rd * cd) / (cl * cl + cd * cd)).clamp(0.7, 1.4);
    }
    gain.iter().any(|g| (g - 1.0).abs() > 0.012).then_some(gain)
}

const FEAT_SCALE: [f32; 6] = [0.08, 0.08, 0.12, 0.25, 0.25, 0.25];

/// Silhouette features of a binary mask over the N x N grid.
fn silhouette(mask: &[bool]) -> Option<[f32; 6]> {
    let (mut x0, mut x1, mut y0, mut y1) = (N, 0, N, 0);
    let mut count = 0;
    for j in 0..N {
        for i in 0..N {
            if mask[j * N + i] {
                x0 = x0.min(i);
                x1 = x1.max(i);
                y0 = y0.min(j);
                y1 = y1.max(j);
                count += 1;
            }
        }
    }
    if count < 12 {
        return None;
    }
    let (w, h) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
    let width_at = |frac: f32| -> f32 {
        let y = y0 + ((h - 1.0) * frac).round() as usize;
        let n = (x0..=x1).filter(|&i| mask[y * N + i]).count();
        n as f32 / w
    };
    Some([
        w / N as f32,
        h / N as f32,
        count as f32 / (w * h),
        width_at(0.2),
        width_at(0.5),
        width_at(0.8),
    ])
}

impl Recognizer {
    /// Parse templates; `None` if they are empty or from an incompatible version.
    pub fn new(t: &Templates) -> Option<Recognizer> {
        let st: Stored = serde_json::from_value(t.data.clone()).ok()?;
        if st.version != STORE_VERSION
            || st.n != N
            || st.pieces.len() != PIECES.len()
            || st.palette.is_empty()
        {
            return None;
        }
        let palette = Palette {
            colours: st
                .palette
                .iter()
                .map(|(c, l)| ([c[0] as f32, c[1] as f32, c[2] as f32], *l))
                .collect(),
        };
        let mut pieces = Vec::new();
        for h in &st.pieces {
            let b = hex_decode(h)?;
            if b.len() != 2 * N * N {
                return None;
            }
            let light: Vec<f32> = b[..N * N].iter().map(|&v| v as f32 / 255.0).collect();
            let dark: Vec<f32> = b[N * N..].iter().map(|&v| v as f32 / 255.0).collect();
            let total = light.iter().sum::<f32>() + dark.iter().sum::<f32>();
            let mask: Vec<bool> = (0..N * N).map(|i| light[i] + dark[i] >= 0.5).collect();
            pieces.push(PieceTemplate {
                light,
                dark,
                total,
                feat: silhouette(&mask),
            });
        }
        let ratio = |range: std::ops::Range<usize>| -> f32 {
            let (l, d) = range.fold((0.0, 0.0), |a, i| {
                (
                    a.0 + pieces[i].light.iter().sum::<f32>(),
                    a.1 + pieces[i].dark.iter().sum::<f32>(),
                )
            });
            l / (l + d).max(1.0)
        };
        let mut side_ratio = [ratio(0..6), ratio(6..12)];
        // One side without any template (learned from a position that lacks it): assume it is
        // painted with the opposite light/dark mix of the other side.
        let has = |range: std::ops::Range<usize>| range.into_iter().any(|i| pieces[i].total > 0.0);
        match (has(0..6), has(6..12)) {
            (true, false) => side_ratio[1] = 1.0 - side_ratio[0],
            (false, true) => side_ratio[0] = 1.0 - side_ratio[1],
            _ => {}
        }
        let square_refs = st.squares.map(|s| {
            [
                [s[0][0] as f32, s[0][1] as f32, s[0][2] as f32],
                [s[1][0] as f32, s[1][1] as f32, s[1][2] as f32],
            ]
        });
        Some(Recognizer {
            palette,
            pieces,
            side_ratio,
            square_refs,
        })
    }

    /// Masked IoU of observed classes vs a template shifted by (dx, dy) cells.
    /// Returns (iou, visible fraction of the template).
    fn similarity(cls: &[u8], t: &PieceTemplate, dx: i32, dy: i32) -> (f32, f32) {
        let (mut inter, mut uni, mut visible) = (0.0f32, 0.0f32, 0.0f32);
        for j in 0..N as i32 {
            let tj = j + dy;
            if !(0..N as i32).contains(&tj) {
                continue;
            }
            for i in 0..N as i32 {
                let ti = i + dx;
                if !(0..N as i32).contains(&ti) {
                    continue;
                }
                let c = cls[(j * N as i32 + i) as usize];
                if c == UNKNOWN {
                    continue;
                }
                let ti = (tj * N as i32 + ti) as usize;
                let (tl, td) = (t.light[ti], t.dark[ti]);
                let (ol, od) = ((c == LIGHT) as u8 as f32, (c == DARK) as u8 as f32);
                inter += ol.min(tl) + od.min(td);
                uni += ol.max(tl) + od.max(td);
                visible += tl + td;
            }
        }
        if uni <= 0.0 {
            return (0.0, 0.0);
        }
        (
            inter / uni,
            if t.total > 0.0 {
                visible / t.total
            } else {
                0.0
            },
        )
    }

    /// Shape/side fallback for silhouettes that match no template.
    fn fallback(&self, cls: &[u8]) -> Option<char> {
        let mask: Vec<bool> = cls.iter().map(|&c| c == LIGHT || c == DARK).collect();
        let f = silhouette(&mask)?;
        let mut best = (f32::MAX, 0usize);
        for kind in 0..6 {
            // Mean of the white and black template features of this kind.
            let feats: Vec<[f32; 6]> = [kind, kind + 6]
                .iter()
                .filter_map(|&i| self.pieces[i].feat)
                .collect();
            if feats.is_empty() {
                continue;
            }
            let mut d = 0.0;
            for k in 0..6 {
                let m = feats.iter().map(|x| x[k]).sum::<f32>() / feats.len() as f32;
                d += ((f[k] - m) / FEAT_SCALE[k]).powi(2);
            }
            if d < best.0 {
                best = (d, kind);
            }
        }
        if best.0.sqrt() > 3.0 {
            return None;
        }
        let light = cls.iter().filter(|&&c| c == LIGHT).count() as f32;
        let dark = cls.iter().filter(|&&c| c == DARK).count() as f32;
        let ratio = light / (light + dark).max(1.0);
        let white = (ratio - self.side_ratio[0]).abs() < (ratio - self.side_ratio[1]).abs();
        let ch = PIECES[best.1];
        Some(if white { ch } else { ch.to_ascii_lowercase() })
    }

    /// Classify one resampled square. Returns (char, confidence).
    fn classify_square(&self, patch: &[Rgb], cls: &mut Vec<u8>) -> (char, f32) {
        let bg = background(patch);
        classify_patch(patch, bg, &self.palette, cls);
        let (mut piece, mut unknown, mut total) = (0usize, 0usize, 0usize);
        for j in W0..W1 {
            for i in W0..W1 {
                total += 1;
                match cls[j * N + i] {
                    LIGHT | DARK => piece += 1,
                    UNKNOWN => unknown += 1,
                    _ => {}
                }
            }
        }
        let (pf, uf) = (piece as f32 / total as f32, unknown as f32 / total as f32);
        if pf < 0.06 {
            if uf >= 0.5 {
                return ('?', 0.0);
            }
            return ('.', (1.0 - uf).clamp(0.5, 1.0));
        }
        // Zero-shift scores for all templates, then +-1 cell search on the best few.
        let mut scores: Vec<(f32, f32, usize)> = self
            .pieces
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (s, v) = Self::similarity(cls, t, 0, 0);
                (s, v, i)
            })
            .collect();
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        for sc in scores.iter_mut().take(3) {
            for (dx, dy) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
            ] {
                let (s, v) = Self::similarity(cls, &self.pieces[sc.2], dx, dy);
                if s > sc.0 {
                    sc.0 = s;
                    sc.1 = v;
                }
            }
        }
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (best, cover, idx) = scores[0];
        let second = scores[1].0;
        if best < 0.45 {
            return match self.fallback(cls) {
                Some(ch) => (ch, 0.35),
                None => ('?', 0.0),
            };
        }
        let mut conf = (best / 0.7).min(1.0)
            * (0.5 + (best - second) / 0.2).clamp(0.0, 1.0)
            * (cover / 0.6).min(1.0);
        if uf > 0.3 {
            // Much of the square matches no known colour: do not trust the shape match.
            conf *= (1.0 - uf).clamp(0.2, 1.0);
        }
        if conf < 0.25 {
            return ('?', conf);
        }
        (PIECES[idx], conf)
    }

    /// Read every square of the board.
    pub fn read(&self, frame: &Frame, rect: &BoardRect, orientation: Orientation) -> Reading {
        let mut placement = ['?'; 64];
        let mut conf_sum = 0.0;
        let mut patches: Vec<Vec<Rgb>> = Vec::with_capacity(64);
        for r in 0..8 {
            for c in 0..8 {
                let mut patch = Vec::with_capacity(N * N);
                extract_square(frame, rect, r, c, &mut patch);
                patches.push(patch);
            }
        }
        let bgs: Vec<Rgb> = patches.iter().map(|p| background(p)).collect();
        // Undo a global brightness/tint change (monitor profile, night mode, page overlay)
        // measured on the square colours, so the fixed colour gates keep working.
        if let Some(reference) = &self.square_refs
            && let Some(g) = global_gain(reference, &square_colours(&bgs))
        {
            for px in patches.iter_mut().flatten() {
                for k in 0..3 {
                    // A channel at full scale was probably clipped by the brightening itself,
                    // so it must not be pulled down with the rest.
                    if g[k] < 1.0 && px[k] >= 253.0 {
                        continue;
                    }
                    px[k] = (px[k] * g[k]).min(255.0);
                }
            }
        }
        let mut cls = Vec::with_capacity(N * N);
        for r in 0..8 {
            for c in 0..8 {
                let (ch, conf) = self.classify_square(&patches[r * 8 + c], &mut cls);
                placement[board_index(orientation, r, c)] = ch;
                conf_sum += conf;
            }
        }
        let (mover_hint, last_move_squares) = last_move(&bgs, &placement, orientation);
        Reading {
            placement,
            orientation,
            confidence: conf_sum / 64.0,
            mover_hint,
            last_move_squares,
        }
    }
}

/// Could `piece` have moved from board index `from` to `to` on an empty board (blockers ignored)?
pub(crate) fn plausible_move(piece: char, from: usize, to: usize) -> bool {
    let (dx, dy) = (
        (to % 8) as i32 - (from % 8) as i32,
        (to / 8) as i32 - (from / 8) as i32,
    );
    let (ax, ay) = (dx.abs(), dy.abs());
    match piece.to_ascii_lowercase() {
        'p' => {
            // Row 0 is rank 8: white pawns move towards lower rows.
            let fwd = if piece.is_ascii_uppercase() { -dy } else { dy };
            let start_row = if piece.is_ascii_uppercase() { 6 } else { 1 };
            (ax == 0 && (fwd == 1 || (fwd == 2 && from / 8 == start_row))) || (ax == 1 && fwd == 1)
        }
        'n' => (ax, ay) == (1, 2) || (ax, ay) == (2, 1),
        'b' => ax == ay && ax > 0,
        'r' => (ax == 0) != (ay == 0),
        'q' => (ax == ay && ax > 0) || ((ax == 0) != (ay == 0)),
        'k' => (ax.max(ay) == 1) || (ay == 0 && ax == 2),
        _ => false,
    }
}

/// Mover colour and (from, to) of the last move from tinted (highlighted) squares.
///
/// A square is tinted when its background colour is neither its parity's normal colour nor a
/// translucent darkening of it. Exactly one tinted pair with one empty and one occupied square
/// is the last move. If more squares are tinted (a selected piece also gets a tint), the pair
/// must be the only one that is geometrically a move of the piece standing on it. Anything
/// else gives `None`.
fn last_move(
    bgs: &[Rgb],
    placement: &Placement,
    orientation: Orientation,
) -> (Option<char>, Option<(u8, u8)>) {
    let normal = square_colours(bgs);
    let tinted: Vec<usize> = (0..64)
        .filter(|&sq| !bg_like(bgs[sq], normal[(sq / 8 + sq % 8) % 2], 20.0 * 20.0))
        .map(|sq| board_index(orientation, sq / 8, sq % 8))
        .collect();
    if !(2..=4).contains(&tinted.len()) || tinted.iter().any(|&i| placement[i] == '?') {
        return (None, None);
    }
    let empty: Vec<usize> = tinted
        .iter()
        .copied()
        .filter(|&i| placement[i] == '.')
        .collect();
    let occupied: Vec<usize> = tinted
        .iter()
        .copied()
        .filter(|&i| placement[i] != '.')
        .collect();
    let pairs: Vec<(usize, usize)> = if tinted.len() == 2 {
        empty
            .iter()
            .flat_map(|&e| occupied.iter().map(move |&o| (e, o)))
            .collect()
    } else {
        empty
            .iter()
            .flat_map(|&e| occupied.iter().map(move |&o| (e, o)))
            .filter(|&(e, o)| plausible_move(placement[o], e, o))
            .collect()
    };
    match pairs.as_slice() {
        [(from, to)] => (
            Some(if placement[*to].is_ascii_uppercase() {
                'w'
            } else {
                'b'
            }),
            Some((*from as u8, *to as u8)),
        ),
        _ => (None, None),
    }
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

const START_PLACEMENT: &str = "rnbqkbnrpppppppp................................PPPPPPPPRNBQKBNR";

fn start_placement_chars() -> Placement {
    let mut p = ['.'; 64];
    for (i, c) in START_PLACEMENT.chars().enumerate() {
        p[i] = c;
    }
    p
}

/// Orientation of the standard start position from the brightness of the piece pixels at the
/// top vs the bottom of the board.
fn start_orientation(
    patches: &[Vec<Rgb>],
    bgs: &[Rgb],
    fail: &dyn Fn(&str) -> VisionError,
) -> Result<Orientation, VisionError> {
    let mut side_luma = [Vec::new(), Vec::new()]; // top rows, bottom rows
    for sq in 0..64 {
        let r = sq / 8;
        if (2..6).contains(&r) {
            continue;
        }
        let mut l: Vec<f32> = Vec::new();
        for j in W0..W1 {
            for i in W0..W1 {
                let p = patches[sq][j * N + i];
                if !bg_like(p, bgs[sq], 40.0 * 40.0) {
                    l.push(luma(p));
                }
            }
        }
        if !l.is_empty() {
            side_luma[(r >= 6) as usize].push(median(&mut l));
        }
    }
    let (top, bottom) = (median(&mut side_luma[0]), median(&mut side_luma[1]));
    if (top - bottom).abs() < 25.0 {
        return Err(fail("cannot tell white from black pieces"));
    }
    Ok(if bottom > top {
        Orientation::White
    } else {
        Orientation::Black
    })
}

/// Shared learning core. `truth` is the true placement (board coordinates); `orientation` is
/// fixed or inferred (start position only). Returns the templates and the orientation used.
fn learn_core(
    frame: &Frame,
    rect: &BoardRect,
    truth: &Placement,
    orientation: Option<Orientation>,
    min_squares: usize,
    what: &str,
) -> Result<(Templates, Orientation), VisionError> {
    let fail = |m: &str| VisionError::Other(format!("{what} not recognised: {m}"));
    if rect.size < 64.0 || frame.rgba.len() < (frame.width * frame.height * 4) as usize {
        return Err(fail("board too small"));
    }
    if truth.contains(&'?') || truth.iter().any(|c| *c != '.' && !PIECES.contains(c)) {
        return Err(fail(
            "the placement must only contain pieces and empty squares",
        ));
    }
    // 1. Resample all squares; background per square.
    let mut patches: Vec<Vec<Rgb>> = Vec::with_capacity(64);
    let mut bgs: Vec<Rgb> = Vec::with_capacity(64);
    for r in 0..8 {
        for c in 0..8 {
            let mut p = Vec::with_capacity(N * N);
            extract_square(frame, rect, r, c, &mut p);
            bgs.push(background(&p));
            patches.push(p);
        }
    }
    // 2. Orientation: given, or from the start position's piece brightness.
    let orientation = match orientation {
        Some(o) => o,
        None => start_orientation(&patches, &bgs, &fail)?,
    };
    let truth_at = |sq: usize| truth[board_index(orientation, sq / 8, sq % 8)];
    // 3. Occupancy: foreground = clearly different from the (shaded) background.
    let mut fg_frac = [0.0f32; 64];
    for (sq, p) in patches.iter().enumerate() {
        let mut n = 0;
        for j in W0..W1 {
            for i in W0..W1 {
                if !bg_like(p[j * N + i], bgs[sq], 40.0 * 40.0) {
                    n += 1;
                }
            }
        }
        fg_frac[sq] = n as f32 / ((W1 - W0) * (W1 - W0)) as f32;
    }
    let n_occ = (0..64).filter(|&sq| truth_at(sq) != '.').count();
    if n_occ == 0 {
        return Err(fail("no pieces to learn from"));
    }
    let (mut occ_ok, mut empty_ok) = (0usize, 0usize);
    for (sq, &frac) in fg_frac.iter().enumerate() {
        if truth_at(sq) != '.' {
            occ_ok += (frac >= 0.1) as usize;
        } else {
            empty_ok += (frac <= 0.06) as usize;
        }
    }
    let n_empty = 64 - n_occ;
    if occ_ok * 100 < n_occ * 85 || empty_ok * 100 < n_empty * 65 {
        return Err(fail(&format!(
            "the picture does not match the position ({occ_ok}/{n_occ} pieces seen, {empty_ok}/{n_empty} empty squares clear)"
        )));
    }
    // 4. Palette from foreground pixels of occupied squares; colours must recur on many squares.
    let mut samples: Vec<(Rgb, u8)> = Vec::new();
    for sq in 0..64 {
        if truth_at(sq) == '.' {
            continue;
        }
        for j in 1..N - 1 {
            for i in 1..N - 1 {
                let p = patches[sq][j * N + i];
                if !bg_like(p, bgs[sq], 40.0 * 40.0) {
                    samples.push((p, sq as u8));
                }
            }
        }
    }
    let pal = learn_palette(&samples, min_squares.min(n_occ.div_ceil(3)).max(1));
    if pal.len() < 2 {
        return Err(fail("could not find piece colours"));
    }
    let palette = Palette {
        colours: luma_classes(&pal),
    };
    if !palette.colours.iter().any(|c| c.1) || !palette.colours.iter().any(|c| !c.1) {
        return Err(fail("piece colours are not distinguishable"));
    }

    // 5. Templates: average the observed classes over all instances of a piece. Kinds that do
    // not occur keep an all-zero template (and are then only found by the shape fallback).
    let mut acc: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = (0..12)
        .map(|_| (vec![0.0; N * N], vec![0.0; N * N], vec![0.0; N * N]))
        .collect();
    let mut cls = Vec::with_capacity(N * N);
    for sq in 0..64 {
        let ch = truth_at(sq);
        let Some(k) = PIECES.iter().position(|&p| p == ch) else {
            continue;
        };
        classify_patch(&patches[sq], bgs[sq], &palette, &mut cls);
        let a = &mut acc[k];
        for (cell, &cl) in cls.iter().enumerate() {
            if cl != UNKNOWN {
                a.2[cell] += 1.0;
                if cl == LIGHT {
                    a.0[cell] += 1.0;
                } else if cl == DARK {
                    a.1[cell] += 1.0;
                }
            }
        }
    }
    let mut pieces = Vec::with_capacity(12);
    for (l, d, n) in &acc {
        let mut bytes = Vec::with_capacity(2 * N * N);
        for src in [l, d] {
            for cell in 0..N * N {
                let v = if n[cell] > 0.0 {
                    src[cell] / n[cell]
                } else {
                    0.0
                };
                bytes.push((v * 255.0).round() as u8);
            }
        }
        pieces.push(hex_encode(&bytes));
    }
    let stored = Stored {
        version: STORE_VERSION,
        n: N,
        palette: palette
            .colours
            .iter()
            .map(|(c, l)| {
                (
                    [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8],
                    *l,
                )
            })
            .collect(),
        pieces,
        squares: Some(
            square_colours(&bgs)
                .map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]),
        ),
    };
    let templates = Templates {
        data: serde_json::to_value(&stored).map_err(|e| VisionError::Other(e.to_string()))?,
    };

    // 6. Self check: the learned templates must read the known position back.
    let rec = Recognizer::new(&templates).ok_or_else(|| fail("template encoding failed"))?;
    let reading = rec.read(frame, rect, orientation);
    let wrong = (0..64)
        .filter(|&i| reading.placement[i] != truth[i])
        .count();
    if wrong > 4 {
        return Err(fail(&format!(
            "templates do not reproduce the position ({wrong} squares differ)"
        )));
    }
    Ok((templates, orientation))
}

/// Learn templates from a frame showing the STANDARD STARTING POSITION.
/// Orientation is inferred from where the white king/pieces sit.
pub fn learn_from_start(
    frame: &Frame,
    rect: &BoardRect,
) -> Result<(Templates, Orientation), VisionError> {
    learn_core(
        frame,
        rect,
        &start_placement_chars(),
        None,
        10,
        "start position",
    )
}

/// Learn templates from ANY frame whose true placement is known (e.g. a game in progress whose
/// position the user supplies, or a bundled reference screenshot).
///
/// `placement` is in board coordinates (index 0 = a8, 'PNBRQK' white, 'pnbrqk' black, '.' empty,
/// see [`Placement`]); `orientation` says which colour is at the bottom of the screen. Piece
/// kinds that do not occur in the position get no template: they are then only recognised by the
/// shape-feature fallback (low confidence). Highlights, selected squares and legal-move dots are
/// harmless because every square is compared with its own background colour. Fails when the
/// picture does not match the placement.
pub fn learn_from_placement(
    frame: &Frame,
    rect: &BoardRect,
    placement: &Placement,
    orientation: Orientation,
) -> Result<Templates, VisionError> {
    learn_core(frame, rect, placement, Some(orientation), 4, "position").map(|(t, _)| t)
}

/// Read every square. Must ignore move highlights, legal-move dots, arrows
/// drawn on the board, coordinates labels and drag ghosts as far as possible.
///
/// Parses the templates on every call; use [`Recognizer`] to amortise that
/// when reading continuously. Invalid or empty templates yield an all-`?`
/// reading with zero confidence.
pub fn read_board(
    frame: &Frame,
    rect: &BoardRect,
    templates: &Templates,
    orientation: Orientation,
) -> Reading {
    match Recognizer::new(templates) {
        Some(r) => r.read(frame, rect, orientation),
        None => Reading {
            placement: ['?'; 64],
            orientation,
            confidence: 0.0,
            mover_hint: None,
            last_move_squares: None,
        },
    }
}
