//! Chess board detection.
//!
//! # Approach
//!
//! A chess board is the only place on a screen where two flat colours alternate
//! on a regular 8x8 lattice, and that is what the detector looks for. It does
//! not rely on a specific theme, so a custom board colour works as well as the
//! built-in ones; the per-site colour priors only decide which candidates are
//! tried first and give a (tolerant) fallback for textured boards.
//!
//! 1. **Downscale** the frame with a box filter to at most ~640 px (cheap, also
//!    averages noise away).
//! 2. **Colour pairs.** A 4-bit colour histogram yields the dominant flat
//!    colours; the pairs that are most often horizontally/vertically adjacent
//!    are the board candidates (a board contributes `112 * square_size` light/dark
//!    transitions, more than anything else on a page). Site priors bias the order.
//! 3. **Lattice.** For a pair, every pixel is labelled light/dark/other. The
//!    number of light<->dark transitions per column (row) peaks on the 7 inner
//!    grid lines; the best equally spaced run of 7 peaks gives position and
//!    square size ([`find_lattice`]). Pieces, highlights, coordinates, arrows
//!    and window chrome only remove some transitions or add isolated ones, they
//!    cannot create a second equally spaced set.
//! 4. **Sub-pixel refinement** at full resolution: each inner grid line is
//!    located from the anti-aliased light/dark edge profile (the amount of
//!    "left colour" in a short window), and the 7 lines are fitted with a
//!    straight line, giving the board origin and square size to a small
//!    fraction of a pixel, snapped to the 8-square grid by construction.
//! 5. **Scoring** ([`board_metrics`]) samples every square's corners/edges and
//!    checks they alternate between two colours and that the board border is
//!    tight. The same routine, self calibrating, backs [`verify_board`].
//!
//! Performance: dominated by the downscale (one pass over the frame); a
//! 2560x1440 frame takes a few milliseconds, [`verify_board`] ~50 microseconds.
//! Boards below 96 device pixels are ignored.

use crate::capture::Frame;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Site {
    ChessCom,
    Lichess,
    Generic,
}

/// Square board region in FRAME PIXELS (device pixels of the captured frame).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardRect {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

type Rgb = [f32; 3];

/// Smallest board (device pixels) that is considered.
const MIN_BOARD: f32 = 96.0;
/// A candidate needs at least this score to be accepted.
const ACCEPT_SCORE: f32 = 0.6;
/// A candidate this good ends the search early.
const EARLY_OUT_SCORE: f32 = 0.85;

/// Known board colour pairs: (light, dark, tolerance for textured fallback).
struct Prior {
    light: Rgb,
    dark: Rgb,
    tol: f32,
}

const fn prior(l: [u8; 3], d: [u8; 3], tol: f32) -> Prior {
    Prior {
        light: [l[0] as f32, l[1] as f32, l[2] as f32],
        dark: [d[0] as f32, d[1] as f32, d[2] as f32],
        tol,
    }
}

/// chess.com themes: green (default), brown, blue/ice, purple, wood, tournament, dark wood.
const CHESSCOM_PRIORS: &[Prior] = &[
    prior([238, 238, 210], [118, 150, 86], 30.0),
    prior([237, 214, 176], [184, 135, 98], 32.0),
    prior([222, 227, 230], [140, 162, 173], 30.0),
    prior([240, 241, 240], [132, 118, 186], 30.0),
    prior([235, 236, 208], [115, 149, 82], 30.0),
    prior([226, 193, 146], [168, 112, 66], 48.0),
    prior([199, 156, 108], [120, 78, 48], 48.0),
];

/// lichess themes: brown (default), blue, green, ic, pink.
const LICHESS_PRIORS: &[Prior] = &[
    prior([240, 217, 181], [181, 136, 99], 32.0),
    prior([222, 227, 230], [140, 162, 173], 30.0),
    prior([255, 255, 221], [134, 166, 102], 30.0),
    prior([236, 236, 236], [196, 196, 196], 20.0),
    prior([240, 226, 227], [196, 140, 154], 30.0),
];

fn priors_for(site: Site) -> Vec<&'static Prior> {
    match site {
        Site::ChessCom => CHESSCOM_PRIORS.iter().chain(LICHESS_PRIORS).collect(),
        Site::Lichess => LICHESS_PRIORS.iter().chain(CHESSCOM_PRIORS).collect(),
        Site::Generic => CHESSCOM_PRIORS.iter().chain(LICHESS_PRIORS).collect(),
    }
}

// ---------------------------------------------------------------------------
// colour helpers
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

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    let (_, m, _) = v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    *m
}

fn median_rgb(samples: &[Rgb]) -> Rgb {
    let mut out = [0.0; 3];
    let mut tmp: Vec<f32> = Vec::with_capacity(samples.len());
    for (ch, o) in out.iter_mut().enumerate() {
        tmp.clear();
        tmp.extend(samples.iter().map(|s| s[ch]));
        *o = median(&mut tmp);
    }
    out
}

// ---------------------------------------------------------------------------
// downscaling
// ---------------------------------------------------------------------------

/// Box-filtered RGB copy of a frame.
struct Small {
    w: usize,
    h: usize,
    /// Downscale factor (frame pixels per small pixel).
    f: usize,
    rgb: Vec<u8>,
}

impl Small {
    #[inline]
    fn at(&self, x: usize, y: usize) -> Rgb {
        let i = (y * self.w + x) * 3;
        [
            self.rgb[i] as f32,
            self.rgb[i + 1] as f32,
            self.rgb[i + 2] as f32,
        ]
    }
}

/// Default downscale factor: the longest side ends up at about 640 pixels.
fn default_factor(frame: &Frame) -> usize {
    (frame.width.max(frame.height) as usize)
        .div_ceil(640)
        .max(1)
}

fn downscale(frame: &Frame, f: usize) -> Small {
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    let f = f.max(1);
    let (w, h) = (fw / f, fh / f);
    let mut rgb = vec![0u8; w * h * 3];
    let mut acc = vec![0u32; w * 3];
    let div = (f * f) as u32;
    for oy in 0..h {
        acc.iter_mut().for_each(|a| *a = 0);
        for dy in 0..f {
            let row = &frame.rgba[((oy * f + dy) * fw) * 4..((oy * f + dy) * fw + w * f) * 4];
            for (ox, a) in acc.chunks_exact_mut(3).enumerate() {
                let block = &row[ox * f * 4..(ox + 1) * f * 4];
                for px in block.chunks_exact(4) {
                    a[0] += px[0] as u32;
                    a[1] += px[1] as u32;
                    a[2] += px[2] as u32;
                }
            }
        }
        let out = &mut rgb[oy * w * 3..(oy + 1) * w * 3];
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = ((a + div / 2) / div) as u8;
        }
    }
    Small { w, h, f, rgb }
}

// ---------------------------------------------------------------------------
// colour pair candidates
// ---------------------------------------------------------------------------

struct Peak {
    colour: Rgb,
}

/// Dominant flat colours of the small image (up to 10).
fn colour_peaks(s: &Small) -> Vec<Peak> {
    const B: usize = 16;
    let mut count = vec![0u32; B * B * B];
    let mut sum = vec![[0u32; 3]; B * B * B];
    for px in s.rgb.chunks_exact(3) {
        let bin = ((px[0] as usize >> 4) * B + (px[1] as usize >> 4)) * B + (px[2] as usize >> 4);
        count[bin] += 1;
        let sm = &mut sum[bin];
        sm[0] += px[0] as u32;
        sm[1] += px[1] as u32;
        sm[2] += px[2] as u32;
    }
    let min_count = ((s.w * s.h) as u32 / 1000).max(8);
    let smoothed = |r: usize, g: usize, b: usize| -> (u32, [u32; 3], u32) {
        let (mut c, mut sm, mut own) = (0u32, [0u32; 3], 0u32);
        for dr in r.saturating_sub(1)..=(r + 1).min(B - 1) {
            for dg in g.saturating_sub(1)..=(g + 1).min(B - 1) {
                for db in b.saturating_sub(1)..=(b + 1).min(B - 1) {
                    let i = (dr * B + dg) * B + db;
                    c += count[i];
                    for k in 0..3 {
                        sm[k] += sum[i][k];
                    }
                    if i == (r * B + g) * B + b {
                        own = count[i];
                    }
                }
            }
        }
        (c, sm, own)
    };
    let mut sm_counts = vec![0u32; B * B * B];
    for r in 0..B {
        for g in 0..B {
            for b in 0..B {
                if count[(r * B + g) * B + b] > 0 {
                    sm_counts[(r * B + g) * B + b] = smoothed(r, g, b).0;
                }
            }
        }
    }
    let mut peaks: Vec<(u32, Rgb)> = Vec::new();
    for r in 0..B {
        for g in 0..B {
            for b in 0..B {
                let i = (r * B + g) * B + b;
                let c = sm_counts[i];
                if count[i] == 0 || c < min_count {
                    continue;
                }
                let mut is_max = true;
                'n: for dr in r.saturating_sub(1)..=(r + 1).min(B - 1) {
                    for dg in g.saturating_sub(1)..=(g + 1).min(B - 1) {
                        for db in b.saturating_sub(1)..=(b + 1).min(B - 1) {
                            let j = (dr * B + dg) * B + db;
                            if j != i && (sm_counts[j] > c || (sm_counts[j] == c && j < i)) {
                                is_max = false;
                                break 'n;
                            }
                        }
                    }
                }
                if is_max {
                    let (cnt, sm, _) = smoothed(r, g, b);
                    let n = cnt as f32;
                    peaks.push((cnt, [sm[0] as f32 / n, sm[1] as f32 / n, sm[2] as f32 / n]));
                }
            }
        }
    }
    peaks.sort_by_key(|p| std::cmp::Reverse(p.0));
    let mut out: Vec<Peak> = Vec::new();
    for (_, colour) in peaks {
        if out.iter().all(|p| d2(p.colour, colour) > 20.0 * 20.0) {
            out.push(Peak { colour });
        }
        if out.len() == 10 {
            break;
        }
    }
    out
}

/// A (light, dark) colour pair to try, with the label tolerance to use.
#[derive(Clone, Copy)]
struct Candidate {
    light: Rgb,
    dark: Rgb,
    tol: f32,
}

/// Most plausible board colour pairs: adjacent-colour statistics + prior bias.
fn pair_candidates(s: &Small, peaks: &[Peak], priors: &[&Prior]) -> Vec<Candidate> {
    let n = peaks.len();
    if n < 2 {
        return Vec::new();
    }
    let label = |c: Rgb| -> u8 {
        let mut best = (255u8, 16.0f32 * 16.0);
        for (i, p) in peaks.iter().enumerate() {
            let d = d2(c, p.colour);
            if d <= best.1 {
                best = (i as u8, d);
            }
        }
        best.0
    };
    let labels: Vec<u8> = (0..s.w * s.h)
        .map(|i| label(s.at(i % s.w, i / s.w)))
        .collect();
    let mut adj = vec![0u32; n * n];
    for y in 0..s.h {
        for x in 0..s.w {
            let a = labels[y * s.w + x];
            if a == 255 {
                continue;
            }
            for (dx, dy) in [(1, 0), (2, 0), (0, 1), (0, 2)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= s.w || ny >= s.h {
                    continue;
                }
                let b = labels[ny * s.w + nx];
                if b != 255 && b != a {
                    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                    adj[lo as usize * n + hi as usize] += 1;
                }
            }
        }
    }
    let near_prior = |c: Rgb, light: bool| {
        priors
            .iter()
            .any(|p| d2(c, if light { p.light } else { p.dark }) < 36.0 * 36.0)
    };
    let mut pairs: Vec<(f32, Candidate)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let cnt = adj[i * n + j];
            if cnt < 24 || d2(peaks[i].colour, peaks[j].colour) < 30.0 * 30.0 {
                continue;
            }
            let (l, d) = if luma(peaks[i].colour) >= luma(peaks[j].colour) {
                (peaks[i].colour, peaks[j].colour)
            } else {
                (peaks[j].colour, peaks[i].colour)
            };
            let boost = if near_prior(l, true) && near_prior(d, false) {
                2.0
            } else {
                1.0
            };
            let sep = d2(l, d).sqrt();
            pairs.push((
                cnt as f32 * boost,
                Candidate {
                    light: l,
                    dark: d,
                    tol: (sep * 0.22).clamp(10.0, 24.0),
                },
            ));
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    pairs.into_iter().take(4).map(|p| p.1).collect()
}

// ---------------------------------------------------------------------------
// lattice search
// ---------------------------------------------------------------------------

/// Lattice of the 7 inner grid lines of a board in small-image units.
struct Lattice {
    /// Position of the first inner line (boundary between squares 0 and 1) on x / y.
    first: f32,
    step: f32,
    score: f32,
}

/// Best equally spaced run of 7 peaks in a transition profile.
fn find_lattice(p: &[f32], min_s: f32, max_s: f32) -> Option<Lattice> {
    let max = p.iter().cloned().fold(0.0f32, f32::max);
    if max <= 0.0 || p.len() < 8 {
        return None;
    }
    let thr = max * 0.2;
    let mut peaks: Vec<(f32, f32)> = Vec::new(); // (position, height)
    for i in 1..p.len() - 1 {
        if p[i] >= thr && p[i] >= p[i - 1] && p[i] > p[i + 1] {
            let tot = p[i - 1] + p[i] + p[i + 1];
            let pos = i as f32 + (p[i + 1] - p[i - 1]) / tot;
            peaks.push((pos, p[i]));
        }
    }
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    peaks.truncate(48);
    let at = |x: f32| -> f32 {
        let i = x.round() as i64;
        let mut v = 0.0f32;
        for k in i - 1..=i + 1 {
            if k >= 0 && (k as usize) < p.len() {
                v = v.max(p[k as usize]);
            }
        }
        v
    };
    let mut best: Option<Lattice> = None;
    for a in 0..peaks.len() {
        for b in 0..peaks.len() {
            let (pa, pb) = (peaks[a].0, peaks[b].0);
            let s = (pb - pa) / 6.0;
            if s < min_s || s > max_s {
                continue;
            }
            let mut sum = 0.0;
            let mut min_v = f32::MAX;
            for k in 0..7 {
                let v = at(pa + k as f32 * s);
                sum += v;
                min_v = min_v.min(v);
            }
            if min_v < 0.25 * sum / 7.0 {
                continue;
            }
            if best.as_ref().is_none_or(|bst| sum > bst.score) {
                best = Some(Lattice {
                    first: pa,
                    step: s,
                    score: sum,
                });
            }
        }
    }
    best
}

/// Label map: 0 = other, 1 = light, 2 = dark.
fn label_small(s: &Small, c: &Candidate) -> Vec<u8> {
    let t2 = c.tol * c.tol;
    (0..s.w * s.h)
        .map(|i| {
            let p = s.at(i % s.w, i / s.w);
            let (dl, dd) = (d2(p, c.light), d2(p, c.dark));
            if dl <= t2 && dl <= dd {
                1
            } else if dd <= t2 {
                2
            } else {
                0
            }
        })
        .collect()
}

/// Coarse board in small-image units from a label map: (x, y, size, light, dark).
fn coarse_board(s: &Small, labels: &[u8], c: &Candidate) -> Option<(f32, f32, f32, Rgb, Rgb)> {
    let (w, h) = (s.w, s.h);
    let mut pv = vec![0.0f32; w + 1];
    let mut ph = vec![0.0f32; h + 1];
    for y in 0..h {
        for x in 0..w {
            let l = labels[y * w + x];
            if l == 0 {
                continue;
            }
            if x + 1 < w {
                let r = labels[y * w + x + 1];
                if r != 0 && r != l {
                    pv[x + 1] += 1.0;
                } else if r == 0 && x + 2 < w {
                    let r2 = labels[y * w + x + 2];
                    if r2 != 0 && r2 != l {
                        pv[x + 1] += 0.5;
                        pv[x + 2] += 0.5;
                    }
                }
            }
            if y + 1 < h {
                let r = labels[(y + 1) * w + x];
                if r != 0 && r != l {
                    ph[y + 1] += 1.0;
                } else if r == 0 && y + 2 < h {
                    let r2 = labels[(y + 2) * w + x];
                    if r2 != 0 && r2 != l {
                        ph[y + 1] += 0.5;
                        ph[y + 2] += 0.5;
                    }
                }
            }
        }
    }
    let min_s = MIN_BOARD / 8.0 / s.f as f32;
    let max_s = (w.min(h) as f32) / 8.0;
    let lx = find_lattice(&pv, min_s, max_s)?;
    let ly = find_lattice(&ph, min_s, max_s)?;
    if (lx.step - ly.step).abs() > 0.05 * lx.step.max(ly.step) {
        return None;
    }
    let step = (lx.step + ly.step) / 2.0;
    let (x0, y0) = (lx.first - lx.step, ly.first - ly.step);
    // Mean colour of the labelled pixels inside the board = actual flat colours.
    let (mut sl, mut sd, mut nl, mut nd) = ([0.0f32; 3], [0.0f32; 3], 0u32, 0u32);
    let (xa, xb) = (x0.max(0.0) as usize, ((x0 + 8.0 * step) as usize).min(w));
    let (ya, yb) = (y0.max(0.0) as usize, ((y0 + 8.0 * step) as usize).min(h));
    for y in ya..yb {
        for x in xa..xb {
            let p = s.at(x, y);
            match labels[y * w + x] {
                1 => {
                    for k in 0..3 {
                        sl[k] += p[k];
                    }
                    nl += 1;
                }
                2 => {
                    for k in 0..3 {
                        sd[k] += p[k];
                    }
                    nd += 1;
                }
                _ => {}
            }
        }
    }
    if nl < 16 || nd < 16 {
        return None;
    }
    let light = [sl[0] / nl as f32, sl[1] / nl as f32, sl[2] / nl as f32];
    let dark = [sd[0] / nd as f32, sd[1] / nd as f32, sd[2] / nd as f32];
    let _ = c;
    Some((x0, y0, step * 8.0, light, dark))
}

// ---------------------------------------------------------------------------
// frame pixel access + sub-pixel refinement
// ---------------------------------------------------------------------------

#[inline]
fn px(frame: &Frame, x: i32, y: i32) -> Rgb {
    let x = x.clamp(0, frame.width as i32 - 1) as usize;
    let y = y.clamp(0, frame.height as i32 - 1) as usize;
    let i = (y * frame.width as usize + x) * 4;
    [
        frame.rgba[i] as f32,
        frame.rgba[i + 1] as f32,
        frame.rgba[i + 2] as f32,
    ]
}

/// Mean of a 3x3 patch around (x, y); None if the point is outside the frame.
fn patch(frame: &Frame, x: f32, y: f32) -> Option<Rgb> {
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    if xi < 1 || yi < 1 || xi >= frame.width as i32 - 1 || yi >= frame.height as i32 - 1 {
        return None;
    }
    let mut s = [0.0f32; 3];
    for dy in -1..=1 {
        for dx in -1..=1 {
            let p = px(frame, xi + dx, yi + dy);
            for k in 0..3 {
                s[k] += p[k];
            }
        }
    }
    Some([s[0] / 9.0, s[1] / 9.0, s[2] / 9.0])
}

/// Fit the 7 inner grid lines of one axis. Returns (origin, step).
///
/// `vertical == true` locates the vertical lines (x positions), otherwise the
/// horizontal lines. `(ox, oy, sx, sy)` is the coarse lattice in frame pixels.
fn refine_axis(
    frame: &Frame,
    light: Rgb,
    dark: Rgb,
    (ox, oy, sx, sy): (f32, f32, f32, f32),
    vertical: bool,
    f: usize,
) -> Option<(f32, f32)> {
    let axis_step = if vertical { sx } else { sy };
    let half = ((1.5 * f as f32 + 2.0).min(0.3 * axis_step)).max(3.0) as i32;
    let axis_origin = if vertical { ox } else { oy };
    let (cross_origin, cross_step) = if vertical { (oy, sy) } else { (ox, sx) };
    let lo = (cross_origin + 0.08 * cross_step).ceil() as i32;
    let hi = (cross_origin + 8.0 * cross_step - 0.08 * cross_step).floor() as i32;
    let ld = d2(light, dark);
    if ld < 100.0 {
        return None;
    }
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut edges: Vec<f32> = Vec::new();
    for k in 1..=7i32 {
        let pred = axis_origin + k as f32 * axis_step;
        let start = pred.round() as i32 - half;
        edges.clear();
        for c in lo..=hi {
            let r = (((c as f32 - cross_origin) / cross_step).floor() as i32).clamp(0, 7);
            let (left, right) = if (r + k - 1) % 2 == 0 {
                (light, dark)
            } else {
                (dark, light)
            };
            let axis = [right[0] - left[0], right[1] - left[1], right[2] - left[2]];
            let mut sum_left = 0.0f32;
            let mut clean = true;
            let n = 2 * half + 1;
            for i in 0..n {
                let p = if vertical {
                    px(frame, start + i, c)
                } else {
                    px(frame, c, start + i)
                };
                let rel = [p[0] - left[0], p[1] - left[1], p[2] - left[2]];
                let t = (rel[0] * axis[0] + rel[1] * axis[1] + rel[2] * axis[2]) / ld;
                // Distance of the pixel from the left-right segment.
                let tc = t.clamp(0.0, 1.0);
                let proj = [
                    left[0] + axis[0] * tc,
                    left[1] + axis[1] * tc,
                    left[2] + axis[2] * tc,
                ];
                if d2(p, proj) > 28.0 * 28.0 {
                    clean = false;
                    break;
                }
                if (i < 2 && t > 0.25) || (i >= n - 2 && t < 0.75) {
                    clean = false;
                    break;
                }
                sum_left += 1.0 - tc;
            }
            if clean {
                edges.push(start as f32 + sum_left);
            }
        }
        if edges.len() >= 8 {
            pts.push((k as f32, median(&mut edges.clone())));
        }
    }
    let fit = |pts: &[(f32, f32)]| -> Option<(f32, f32)> {
        if pts.len() < 4 {
            return None;
        }
        let n = pts.len() as f32;
        let (sk, se) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let (mk, me) = (sk / n, se / n);
        let (mut num, mut den) = (0.0, 0.0);
        for p in pts {
            num += (p.0 - mk) * (p.1 - me);
            den += (p.0 - mk) * (p.0 - mk);
        }
        let b = num / den;
        Some((me - b * mk, b))
    };
    let (mut a, mut b) = fit(&pts)?;
    let kept: Vec<(f32, f32)> = pts
        .iter()
        .cloned()
        .filter(|p| (p.1 - (a + b * p.0)).abs() <= 1.0)
        .collect();
    if kept.len() != pts.len() {
        let (a2, b2) = fit(&kept)?;
        a = a2;
        b = b2;
    }
    Some((a, b))
}

/// Snap a refined rect to integer device pixels when it is within a small
/// fraction of one (typical for 1x boards and exact 2x scaling).
fn snap_rect(r: BoardRect) -> BoardRect {
    let mut r = r;
    let per = r.size / 8.0;
    if (per - per.round()).abs() < 0.02 {
        r.size = per.round() * 8.0;
    }
    if (r.x - r.x.round()).abs() < 0.05 {
        r.x = r.x.round();
    }
    if (r.y - r.y.round()).abs() < 0.05 {
        r.y = r.y.round();
    }
    r
}

// ---------------------------------------------------------------------------
// scoring / verification
// ---------------------------------------------------------------------------

/// Result of sampling a board hypothesis.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    /// Overall 0..1 score (hard checks that fail return `None` instead).
    score: f32,
}

/// Sample every square (4 corners + 4 edge midpoints), self-calibrate the two
/// parity colours (medians, so a few highlighted squares do not matter) and
/// measure how well the lattice explains the pixels. A tight border check
/// makes a rect that is off by more than ~5% of a square fail.
fn board_metrics(frame: &Frame, rect: &BoardRect) -> Option<Metrics> {
    let s = rect.size / 8.0;
    if rect.size < MIN_BOARD
        || rect.x < -1.0
        || rect.y < -1.0
        || rect.x + rect.size > frame.width as f32 + 1.0
        || rect.y + rect.size > frame.height as f32 + 1.0
    {
        return None;
    }
    let corner = 0.14 * s;
    let edge = (0.05 * s).max(2.5);
    // (parity, sample, is_outer_edge)
    let mut samples: Vec<(usize, Rgb, bool)> = Vec::with_capacity(512);
    for r in 0..8usize {
        for c in 0..8usize {
            let (x0, y0) = (rect.x + c as f32 * s, rect.y + r as f32 * s);
            let (x1, y1) = (x0 + s, y0 + s);
            let par = (r + c) % 2;
            let pts = [
                (x0 + corner, y0 + corner, false),
                (x1 - corner, y0 + corner, false),
                (x0 + corner, y1 - corner, false),
                (x1 - corner, y1 - corner, false),
                ((x0 + x1) / 2.0, y0 + edge, r == 0),
                ((x0 + x1) / 2.0, y1 - edge, r == 7),
                (x0 + edge, (y0 + y1) / 2.0, c == 0),
                (x1 - edge, (y0 + y1) / 2.0, c == 7),
            ];
            for (x, y, outer) in pts {
                if let Some(p) = patch(frame, x, y) {
                    samples.push((par, p, outer));
                }
            }
        }
    }
    if samples.len() < 400 {
        return None;
    }
    let col = |par: usize| -> Rgb {
        let v: Vec<Rgb> = samples.iter().filter(|s| s.0 == par).map(|s| s.1).collect();
        median_rgb(&v)
    };
    let (light, dark) = (col(0), col(1));
    let sep = d2(light, dark).sqrt();
    if sep < 30.0 || luma(light) <= luma(dark) {
        return None;
    }
    let tol = (0.3 * sep).clamp(14.0, 42.0);
    let t2 = tol * tol;
    let matches = |par: usize, p: Rgb| d2(p, if par == 0 { light } else { dark }) <= t2;
    let inside = samples.iter().filter(|s| matches(s.0, s.1)).count() as f32 / samples.len() as f32;
    let outer: Vec<&(usize, Rgb, bool)> = samples.iter().filter(|s| s.2).collect();
    let border =
        outer.iter().filter(|s| matches(s.0, s.1)).count() as f32 / outer.len().max(1) as f32;
    // Points just outside the board must not look like board squares.
    let out_off = (0.05 * s).max(2.5);
    let mut out_n = 0u32;
    let mut out_match = 0u32;
    for i in 0..8 {
        let t = (i as f32 + 0.5) * s;
        let pts = [
            (rect.x + t, rect.y - out_off),
            (rect.x + t, rect.y + rect.size + out_off),
            (rect.x - out_off, rect.y + t),
            (rect.x + rect.size + out_off, rect.y + t),
        ];
        for (x, y) in pts {
            if let Some(p) = patch(frame, x, y) {
                out_n += 1;
                if d2(p, light) <= t2 || d2(p, dark) <= t2 {
                    out_match += 1;
                }
            }
        }
    }
    let outside = if out_n > 0 {
        out_match as f32 / out_n as f32
    } else {
        0.0
    };
    if inside < 0.6 || border < 0.7 || outside > 0.4 {
        return None;
    }
    let score = (inside * (0.5 + 0.5 * border) * (1.0 - 0.5 * outside)).clamp(0.0, 1.0);
    Some(Metrics { score })
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

/// Build a refined, scored rect for one colour candidate.
fn try_candidate(frame: &Frame, s: &Small, c: &Candidate) -> Option<(BoardRect, f32)> {
    let labels = label_small(s, c);
    let (x, y, size, light, dark) = coarse_board(s, &labels, c)?;
    let f = s.f as f32;
    let coarse = BoardRect {
        x: x * f,
        y: y * f,
        size: size * f,
    };
    if coarse.size < MIN_BOARD {
        return None;
    }
    let st = coarse.size / 8.0;
    let lat = (coarse.x, coarse.y, st, st);
    let tolerance = 2.0 * f + 2.0;
    let mut rect = coarse;
    if let (Some((ax, bx)), Some((ay, by))) = (
        refine_axis(frame, light, dark, lat, true, s.f),
        refine_axis(frame, light, dark, lat, false, s.f),
    ) {
        let size = 4.0 * (bx + by);
        if (ax - coarse.x).abs() < tolerance
            && (ay - coarse.y).abs() < tolerance
            && (size - coarse.size).abs() < 2.0 * tolerance
        {
            rect = snap_rect(BoardRect { x: ax, y: ay, size });
        }
    }
    let m = board_metrics(frame, &rect)?;
    Some((rect, m.score))
}

/// Locate the 8x8 board. `site` selects tuned colour priors (chess.com green
/// default, brown, blue, etc.) before falling back to generic checker detection.
pub fn detect_board(frame: &Frame, site: Site) -> Option<BoardRect> {
    if frame.width < 64
        || frame.height < 64
        || frame.rgba.len() < (frame.width * frame.height * 4) as usize
    {
        return None;
    }
    // The square lattice of a small board can alias at one downscale factor and be clean at
    // another, so a failed pass is retried with finer factors (down to full resolution).
    let f0 = default_factor(frame);
    let mut factors = vec![f0];
    for f in [(f0 * 2).div_ceil(3), f0.div_ceil(2), f0.div_ceil(4), 1] {
        if f >= 1 && f < *factors.last().unwrap_or(&f0) {
            factors.push(f);
        }
    }
    factors
        .into_iter()
        .find_map(|f| detect_with_factor(frame, site, f))
}

fn detect_with_factor(frame: &Frame, site: Site, factor: usize) -> Option<BoardRect> {
    let small = downscale(frame, factor);
    let priors = priors_for(site);
    let peaks = colour_peaks(&small);
    let mut best: Option<(BoardRect, f32)> = None;
    // Keep the best-scoring candidate; true once one is good enough to stop.
    fn consider(best: &mut Option<(BoardRect, f32)>, r: Option<(BoardRect, f32)>) -> bool {
        if let Some((rect, score)) = r
            && score >= ACCEPT_SCORE
            && best.as_ref().is_none_or(|b| score > b.1)
        {
            *best = Some((rect, score));
        }
        best.as_ref().is_some_and(|b| b.1 >= EARLY_OUT_SCORE)
    }
    // 1. Flat colour pairs found in the image itself (theme agnostic).
    for cand in pair_candidates(&small, &peaks, &priors) {
        if consider(&mut best, try_candidate(frame, &small, &cand)) {
            return best.map(|b| b.0);
        }
    }
    if best.is_some() {
        return best.map(|b| b.0);
    }
    // 2. Textured themes: nominal prior colours with a wide tolerance.
    for p in priors {
        let cand = Candidate {
            light: p.light,
            dark: p.dark,
            tol: p.tol,
        };
        if consider(&mut best, try_candidate(frame, &small, &cand)) {
            break;
        }
    }
    best.map(|b| b.0)
}

/// Cheap re-check that the board is still at `rect` (used to skip full detection every frame).
pub fn verify_board(frame: &Frame, rect: &BoardRect, _site: Site) -> bool {
    if frame.rgba.len() < (frame.width * frame.height * 4) as usize {
        return false;
    }
    board_metrics(frame, rect).is_some_and(|m| m.score >= ACCEPT_SCORE)
}
