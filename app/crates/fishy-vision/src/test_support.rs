//! Synthetic chess.com-like screen renderer used by the unit tests.
//!
//! Draws a page (browser chrome, side panel, player bars, buttons) around a
//! board in several themes, with procedurally drawn but clearly distinct piece
//! silhouettes for all 12 piece kinds, plus last-move highlights, legal-move
//! dots, capture rings, coordinate labels, arrows, drag ghosts and mild noise.
//! No bundled artwork and no files: everything is computed.

use crate::capture::Frame;
use crate::recognize::Placement;

#[derive(Clone, Copy)]
pub struct Theme {
    pub light: [u8; 3],
    pub dark: [u8; 3],
    pub hl_light: [u8; 3],
    pub hl_dark: [u8; 3],
}

pub const GREEN: Theme = Theme {
    light: [238, 238, 210],
    dark: [118, 150, 86],
    hl_light: [245, 246, 130],
    hl_dark: [187, 203, 43],
};
pub const BROWN: Theme = Theme {
    light: [240, 217, 181],
    dark: [181, 136, 99],
    hl_light: [205, 210, 106],
    hl_dark: [170, 162, 58],
};
pub const BLUE: Theme = Theme {
    light: [222, 227, 230],
    dark: [140, 162, 173],
    hl_light: [196, 216, 140],
    hl_dark: [120, 160, 110],
};
pub const CUSTOM: Theme = Theme {
    light: [255, 226, 238],
    dark: [170, 96, 130],
    hl_light: [255, 240, 150],
    hl_dark: [200, 180, 80],
};

#[derive(Clone)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS pixel (informational; geometry below is in device px).
    pub scale: u32,
    pub board_x: i32,
    pub board_y: i32,
    /// Square side in device pixels.
    pub square: u32,
    pub theme: Theme,
    pub white_bottom: bool,
    pub placement: Placement,
    /// Board indices (0 = a8) tinted as last-move / selected highlights.
    pub highlights: Vec<usize>,
    /// Legal-move dots on (empty) squares.
    pub dots: Vec<usize>,
    /// Capture rings around (occupied) squares.
    pub rings: Vec<usize>,
    /// Arrows between board indices, drawn above the pieces.
    pub arrows: Vec<(usize, usize)>,
    pub labels: bool,
    pub noise: u8,
    pub seed: u64,
    /// Piece drawn at a free position (centre, device px) over everything.
    pub drag: Option<(char, f32, f32)>,
    /// Squares whose piece is not drawn (e.g. the origin of a dragged piece).
    pub hide: Vec<usize>,
}

impl Scene {
    pub fn new(placement: Placement) -> Scene {
        Scene {
            width: 1280,
            height: 800,
            scale: 1,
            board_x: 232,
            board_y: 72,
            square: 80,
            theme: GREEN,
            white_bottom: true,
            placement,
            highlights: vec![],
            dots: vec![],
            rings: vec![],
            arrows: vec![],
            labels: true,
            noise: 2,
            seed: 7,
            drag: None,
            hide: vec![],
        }
    }

    /// Same scene at device scale 2 (everything doubled).
    pub fn at_scale(mut self, k: u32) -> Scene {
        let m = k as i32;
        self.width *= k;
        self.height *= k;
        self.board_x *= m;
        self.board_y *= m;
        self.square *= k;
        self.scale = k;
        self
    }
}

pub fn placement_from_fen(fen: &str) -> Placement {
    let mut p = ['.'; 64];
    let mut i = 0;
    for ch in fen.split(' ').next().unwrap().chars() {
        match ch {
            '/' => {}
            '1'..='8' => i += ch.to_digit(10).unwrap() as usize,
            c => {
                p[i] = c;
                i += 1;
            }
        }
    }
    assert_eq!(i, 64, "bad fen {fen}");
    p
}

pub fn start_placement() -> Placement {
    placement_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR")
}

// ---------------------------------------------------------------------------
// piece silhouettes
// ---------------------------------------------------------------------------

fn rect(u: f32, v: f32, u0: f32, u1: f32, v0: f32, v1: f32) -> bool {
    u >= u0 && u <= u1 && v >= v0 && v <= v1
}
fn circle(u: f32, v: f32, cu: f32, cv: f32, r: f32) -> bool {
    (u - cu) * (u - cu) + (v - cv) * (v - cv) <= r * r
}
fn ellipse(u: f32, v: f32, cu: f32, cv: f32, ru: f32, rv: f32) -> bool {
    ((u - cu) / ru).powi(2) + ((v - cv) / rv).powi(2) <= 1.0
}
/// Symmetric trapezoid around u=0.5 between v0 (half width h0) and v1 (h1).
fn trap(u: f32, v: f32, v0: f32, v1: f32, h0: f32, h1: f32) -> bool {
    if v < v0 || v > v1 {
        return false;
    }
    let t = (v - v0) / (v1 - v0);
    (u - 0.5).abs() <= h0 + (h1 - h0) * t
}
fn polygon(u: f32, v: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > v) != (yj > v) && u < (xj - xi) * (v - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn inside(kind: char, u: f32, v: f32) -> bool {
    match kind {
        'p' => {
            circle(u, v, 0.5, 0.34, 0.13)
                || trap(u, v, 0.42, 0.70, 0.07, 0.18)
                || rect(u, v, 0.30, 0.70, 0.70, 0.80)
        }
        'r' => {
            let notch = rect(u, v, 0.39, 0.46, 0.22, 0.31) || rect(u, v, 0.54, 0.61, 0.22, 0.31);
            (rect(u, v, 0.27, 0.73, 0.76, 0.85)
                || rect(u, v, 0.33, 0.67, 0.38, 0.76)
                || rect(u, v, 0.27, 0.73, 0.22, 0.38))
                && !notch
        }
        'b' => {
            let slit = (u - 0.5 - (0.44 - v) * 0.9).abs() < 0.018 && (0.30..0.50).contains(&v);
            (rect(u, v, 0.28, 0.72, 0.78, 0.86)
                || ellipse(u, v, 0.5, 0.56, 0.15, 0.22)
                || circle(u, v, 0.5, 0.27, 0.07)
                || circle(u, v, 0.5, 0.17, 0.035))
                && !slit
        }
        'n' => polygon(
            u,
            v,
            &[
                (0.27, 0.84),
                (0.30, 0.55),
                (0.38, 0.42),
                (0.33, 0.30),
                (0.50, 0.15),
                (0.58, 0.21),
                (0.74, 0.34),
                (0.71, 0.50),
                (0.60, 0.50),
                (0.67, 0.62),
                (0.73, 0.84),
            ],
        ),
        'q' => {
            rect(u, v, 0.24, 0.76, 0.80, 0.87)
                || trap(u, v, 0.40, 0.80, 0.09, 0.22)
                || rect(u, v, 0.24, 0.76, 0.30, 0.42)
                || circle(u, v, 0.26, 0.26, 0.05)
                || circle(u, v, 0.38, 0.20, 0.05)
                || circle(u, v, 0.50, 0.16, 0.05)
                || circle(u, v, 0.62, 0.20, 0.05)
                || circle(u, v, 0.74, 0.26, 0.05)
        }
        'k' => {
            rect(u, v, 0.26, 0.74, 0.80, 0.87)
                || trap(u, v, 0.38, 0.80, 0.11, 0.21)
                || rect(u, v, 0.30, 0.70, 0.30, 0.40)
                || rect(u, v, 0.46, 0.54, 0.08, 0.32)
                || rect(u, v, 0.39, 0.61, 0.15, 0.22)
        }
        _ => false,
    }
}

/// RGBA premultiplied-free sprite: (r, g, b, alpha) per pixel.
fn sprite(piece: char, size: usize) -> Vec<[f32; 4]> {
    let white = piece.is_ascii_uppercase();
    let kind = piece.to_ascii_lowercase();
    let (fill, line) = if white {
        ([250.0, 250.0, 250.0], [30.0, 30.0, 30.0])
    } else {
        ([55.0, 55.0, 58.0], [12.0, 12.0, 14.0])
    };
    let d = 0.028;
    let mut out = vec![[0.0f32; 4]; size * size];
    for j in 0..size {
        for i in 0..size {
            let mut col = [0.0f32; 3];
            let mut a = 0.0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let u = (i as f32 + (sx as f32 + 0.5) / 3.0) / size as f32;
                    let v = (j as f32 + (sy as f32 + 0.5) / 3.0) / size as f32;
                    if inside(kind, u, v) {
                        let edge = !(inside(kind, u + d, v)
                            && inside(kind, u - d, v)
                            && inside(kind, u, v + d)
                            && inside(kind, u, v - d));
                        let c = if edge { line } else { fill };
                        for k in 0..3 {
                            col[k] += c[k];
                        }
                        a += 1.0;
                    }
                }
            }
            if a > 0.0 {
                out[j * size + i] = [col[0] / a, col[1] / a, col[2] / a, a / 9.0];
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// canvas
// ---------------------------------------------------------------------------

struct Canvas {
    w: i32,
    h: i32,
    px: Vec<[f32; 3]>,
}

impl Canvas {
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        for yy in y.max(0)..(y + h).min(self.h) {
            for xx in x.max(0)..(x + w).min(self.w) {
                self.px[(yy * self.w + xx) as usize] = [c[0] as f32, c[1] as f32, c[2] as f32];
            }
        }
    }
    fn blend(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let p = &mut self.px[(y * self.w + x) as usize];
        for k in 0..3 {
            p[k] = p[k] * (1.0 - a) + c[k] * a;
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
}

fn screen_cell(white_bottom: bool, idx: usize) -> (i32, i32) {
    let s = if white_bottom { idx } else { 63 - idx };
    ((s / 8) as i32, (s % 8) as i32)
}

fn draw_arrow(cv: &mut Canvas, from: (f32, f32), to: (f32, f32), sq: f32) {
    let colour = [255.0, 170.0, 0.0];
    let alpha = 0.8;
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let shaft_w = 0.09 * sq;
    let head_w = 0.24 * sq;
    let head_len = 0.38 * sq;
    let shaft_end = len - head_len;
    let (minx, maxx) = (from.0.min(to.0) - head_w, from.0.max(to.0) + head_w);
    let (miny, maxy) = (from.1.min(to.1) - head_w, from.1.max(to.1) + head_w);
    for y in miny.floor() as i32..=maxy.ceil() as i32 {
        for x in minx.floor() as i32..=maxx.ceil() as i32 {
            let (px, py) = (x as f32 + 0.5 - from.0, y as f32 + 0.5 - from.1);
            let along = px * ux + py * uy;
            let across = (px * nx + py * ny).abs();
            let in_shaft = along >= 0.0 && along <= shaft_end && across <= shaft_w;
            let in_head =
                along > shaft_end && along <= len && across <= head_w * (len - along) / head_len;
            if in_shaft || in_head {
                cv.blend(x, y, colour, alpha);
            }
        }
    }
}

fn draw_label(cv: &mut Canvas, x: i32, y: i32, w: i32, h: i32, c: [u8; 3], variant: usize) {
    // Tiny glyph made of bars: cheap stand-in for a coordinate letter/digit.
    for i in 0..3 {
        if (variant >> i) & 1 == 1 || i == 1 {
            cv.fill_rect(x + i * w / 3, y, (w / 3).max(1), h, c);
        }
    }
    cv.fill_rect(x, y, w, (h / 6).max(1), c);
}

/// Render the scene to an RGBA frame.
pub fn render(sc: &Scene) -> Frame {
    let (w, h) = (sc.width as i32, sc.height as i32);
    let k = sc.scale as i32;
    let mut cv = Canvas {
        w,
        h,
        px: vec![[0.0; 3]; (w * h) as usize],
    };
    cv.fill_rect(0, 0, w, h, [38, 36, 33]);
    // Browser chrome and tabs.
    cv.fill_rect(0, 0, w, 40 * k, [58, 60, 64]);
    cv.fill_rect(12 * k, 8 * k, 160 * k, 32 * k, [240, 240, 240]);
    cv.fill_rect(180 * k, 8 * k, 160 * k, 32 * k, [84, 86, 90]);
    cv.fill_rect(360 * k, 10 * k, 520 * k, 22 * k, [32, 33, 36]);
    // Left navigation with text-like stripes.
    cv.fill_rect(0, 40 * k, 180 * k, h - 40 * k, [49, 46, 43]);
    for i in 0..14 {
        cv.fill_rect(
            20 * k,
            (70 + i * 34) * k,
            (90 + (i * 17) % 50) * k,
            10 * k,
            [200, 200, 195],
        );
    }
    let (bx, by, sq) = (sc.board_x, sc.board_y, sc.square as i32);
    let board = 8 * sq;
    // Player bars above and below the board.
    cv.fill_rect(bx, by - 46 * k, board, 36 * k, [49, 46, 43]);
    cv.fill_rect(bx + 40 * k, by - 36 * k, 120 * k, 14 * k, [220, 220, 220]);
    cv.fill_rect(bx, by + board + 10 * k, board, 36 * k, [49, 46, 43]);
    cv.fill_rect(
        bx + 40 * k,
        by + board + 20 * k,
        90 * k,
        14 * k,
        [220, 220, 220],
    );
    // Right panel (near-white) with move list stripes and a green button.
    let px0 = bx + board + 24 * k;
    cv.fill_rect(px0, 72 * k, 300 * k, 560 * k, [242, 242, 238]);
    for i in 0..16 {
        cv.fill_rect(
            px0 + 16 * k,
            (100 + i * 26) * k,
            (60 + (i * 31) % 90) * k,
            10 * k,
            [90, 90, 90],
        );
        cv.fill_rect(
            px0 + 160 * k,
            (100 + i * 26) * k,
            (50 + (i * 13) % 80) * k,
            10 * k,
            [90, 90, 90],
        );
    }
    cv.fill_rect(px0 + 16 * k, 560 * k, 268 * k, 44 * k, [129, 182, 76]);

    // Board squares.
    let sqf = sq as f32;
    for idx in 0..64 {
        let (r, c) = screen_cell(sc.white_bottom, idx);
        let light = (r + c) % 2 == 0;
        let hl = sc.highlights.contains(&idx);
        let col = match (light, hl) {
            (true, false) => sc.theme.light,
            (false, false) => sc.theme.dark,
            (true, true) => sc.theme.hl_light,
            (false, true) => sc.theme.hl_dark,
        };
        cv.fill_rect(bx + c * sq, by + r * sq, sq, sq, col);
    }
    // Coordinate labels inside squares (opposite colour).
    if sc.labels {
        for idx in 0..64 {
            let (r, c) = screen_cell(sc.white_bottom, idx);
            let light = (r + c) % 2 == 0;
            let lc = if light { sc.theme.dark } else { sc.theme.light };
            let (x0, y0) = (bx + c * sq, by + r * sq);
            let (gw, gh) = ((0.09 * sqf) as i32, (0.14 * sqf) as i32);
            if c == 0 {
                draw_label(
                    &mut cv,
                    x0 + (0.04 * sqf) as i32,
                    y0 + (0.04 * sqf) as i32,
                    gw,
                    gh,
                    lc,
                    r as usize,
                );
            }
            if r == 7 {
                draw_label(
                    &mut cv,
                    x0 + sq - gw - (0.05 * sqf) as i32,
                    y0 + sq - gh - (0.04 * sqf) as i32,
                    gw,
                    gh,
                    lc,
                    c as usize + 2,
                );
            }
        }
    }
    // Legal move dots and capture rings (translucent black).
    for &idx in &sc.dots {
        let (r, c) = screen_cell(sc.white_bottom, idx);
        let (cx, cy) = (
            (bx + c * sq) as f32 + sqf / 2.0,
            (by + r * sq) as f32 + sqf / 2.0,
        );
        let rad = 0.17 * sqf;
        for y in (cy - rad) as i32 - 1..=(cy + rad) as i32 + 1 {
            for x in (cx - rad) as i32 - 1..=(cx + rad) as i32 + 1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d <= rad {
                    cv.blend(x, y, [0.0; 3], 0.14);
                }
            }
        }
    }
    for &idx in &sc.rings {
        let (r, c) = screen_cell(sc.white_bottom, idx);
        let (cx, cy) = (
            (bx + c * sq) as f32 + sqf / 2.0,
            (by + r * sq) as f32 + sqf / 2.0,
        );
        let (ro, ri) = (0.5 * sqf, 0.42 * sqf);
        for y in by + r * sq..by + (r + 1) * sq {
            for x in bx + c * sq..bx + (c + 1) * sq {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d <= ro && d >= ri {
                    cv.blend(x, y, [0.0; 3], 0.14);
                }
            }
        }
    }
    // Pieces.
    let mut cache: std::collections::HashMap<char, Vec<[f32; 4]>> =
        std::collections::HashMap::new();
    for idx in 0..64 {
        let ch = sc.placement[idx];
        if ch == '.' || sc.hide.contains(&idx) {
            continue;
        }
        let spr = cache.entry(ch).or_insert_with(|| sprite(ch, sq as usize));
        let (r, c) = screen_cell(sc.white_bottom, idx);
        for j in 0..sq {
            for i in 0..sq {
                let s = spr[(j * sq + i) as usize];
                if s[3] > 0.0 {
                    cv.blend(bx + c * sq + i, by + r * sq + j, [s[0], s[1], s[2]], s[3]);
                }
            }
        }
    }
    // Arrows above pieces.
    for &(a, b) in &sc.arrows {
        let (ra, ca) = screen_cell(sc.white_bottom, a);
        let (rb, cb) = screen_cell(sc.white_bottom, b);
        let centre = |r: i32, c: i32| {
            (
                (bx + c * sq) as f32 + sqf / 2.0,
                (by + r * sq) as f32 + sqf / 2.0,
            )
        };
        draw_arrow(&mut cv, centre(ra, ca), centre(rb, cb), sqf);
    }
    // Dragged piece.
    if let Some((ch, cx, cy)) = sc.drag {
        let spr = sprite(ch, sq as usize);
        let (ox, oy) = (cx as i32 - sq / 2, cy as i32 - sq / 2);
        for j in 0..sq {
            for i in 0..sq {
                let s = spr[(j * sq + i) as usize];
                if s[3] > 0.0 {
                    cv.blend(ox + i, oy + j, [s[0], s[1], s[2]], s[3]);
                }
            }
        }
    }
    // Noise + quantise.
    let mut rng = Rng(sc.seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let n = sc.noise as i32;
    for p in &cv.px {
        for ch in p {
            let jitter = if n > 0 {
                (rng.next() % (2 * n as u32 + 1)) as i32 - n
            } else {
                0
            };
            rgba.push((ch.round() as i32 + jitter).clamp(0, 255) as u8);
        }
        rgba.push(255);
    }
    Frame {
        width: sc.width,
        height: sc.height,
        rgba,
        origin_x: 0,
        origin_y: 0,
        scale: sc.scale as f32,
    }
}

// ---------------------------------------------------------------------------
// real-screenshot helpers (chess.com coordinate labels)
// ---------------------------------------------------------------------------

/// `Frame` of an RGBA image.
pub fn frame_of_image(img: &image::RgbaImage) -> Frame {
    Frame {
        width: img.width(),
        height: img.height(),
        rgba: img.clone().into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    }
}

/// Pixels of the label ink in `win` (fractions of the square) of square (`r`, `c`): offsets from
/// the square's top-left pixel with their coverage by the label colour, and that colour (the
/// candidate, plain or dimmed by a highlight, that explains the most pixels).
fn ink_pixels(
    img: &image::RgbaImage,
    rect: &crate::detect::BoardRect,
    cols: &crate::labels::BoardColours,
    r: usize,
    c: usize,
    win: [f32; 4],
) -> (Vec<(u32, u32, f32)>, [f32; 3]) {
    let s = rect.size / 8.0;
    let (sx, sy) = (
        (rect.x + c as f32 * s).round() as u32,
        (rect.y + r as f32 * s).round() as u32,
    );
    let bg = cols.bgs[r * 8 + c];
    let mut best: (Vec<(u32, u32, f32)>, [f32; 3]) = (Vec::new(), bg);
    for label in crate::labels::label_candidates(cols, r, c) {
        let d = [label[0] - bg[0], label[1] - bg[1], label[2] - bg[2]];
        let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let mut out = Vec::new();
        let (x0, x1) = ((win[0] * s) as u32, (win[2] * s).ceil() as u32);
        let (y0, y1) = ((win[1] * s) as u32, (win[3] * s).ceil() as u32);
        for dy in y0..y1 {
            for dx in x0..x1 {
                let px = img.get_pixel(sx + dx, sy + dy);
                let e = [
                    px[0] as f32 - bg[0],
                    px[1] as f32 - bg[1],
                    px[2] as f32 - bg[2],
                ];
                let ee = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
                let t = (e[0] * d[0] + e[1] * d[1] + e[2] * d[2]) / dd;
                let resid = (ee - t * t * dd).max(0.0).sqrt();
                if t > 0.03 && t <= 1.2 && resid <= 11.0 + 0.12 * ee.sqrt() {
                    out.push((dx, dy, t.clamp(0.0, 1.0)));
                }
            }
        }
        if out.len() > best.0.len() {
            best = (out, label);
        }
    }
    best
}

/// Remove the coordinate labels (all rank digits and file letters) from a chess.com screenshot
/// by subtracting the label colour from the pixels it covers.
pub fn erase_labels(img: &image::RgbaImage, rect: &crate::detect::BoardRect) -> image::RgbaImage {
    let frame = frame_of_image(img);
    let cols = crate::labels::board_colours(&frame, rect);
    let s = rect.size / 8.0;
    let mut out = img.clone();
    for k in 0..8 {
        for (r, c, win) in [
            (k, 0, crate::labels::TL_WINDOW),
            (7, k, crate::labels::BR_WINDOW),
        ] {
            let (sx, sy) = (
                (rect.x + c as f32 * s).round() as u32,
                (rect.y + r as f32 * s).round() as u32,
            );
            let bg = cols.bgs[r * 8 + c];
            let (ink, label) = ink_pixels(img, rect, &cols, r, c, win);
            for (dx, dy, a) in ink {
                let p = out.get_pixel_mut(sx + dx, sy + dy);
                for ch in 0..3 {
                    p[ch] = (p[ch] as f32 - a * (label[ch] - bg[ch]))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

/// A faithful flipped view of a chess.com screenshot (white at the bottom in, black at the
/// bottom out): the squares are rotated 180 degrees, pieces stay upright, and the coordinate
/// labels are redrawn the way chess.com draws them for black (rank 1 at the top-left of the left
/// column, files `h..a` at the bottom-right of the bottom row), with the glyphs of the original
/// re-coloured for the new square. `with_labels = false` leaves the labels out.
pub fn flipped_view(
    img: &image::RgbaImage,
    rect: &crate::detect::BoardRect,
    with_labels: bool,
) -> image::RgbaImage {
    let frame = frame_of_image(img);
    let cols = crate::labels::board_colours(&frame, rect);
    let s = rect.size / 8.0;
    let edge = |k: usize| (k as f32 * s).round() as i64;
    let sq_origin = |r: usize, c: usize| {
        (
            (rect.x + c as f32 * s).round() as i64,
            (rect.y + r as f32 * s).round() as i64,
        )
    };
    let side = (edge(1) - edge(0)) as u32;
    // Source glyphs.
    let digits: Vec<Vec<(u32, u32, f32)>> = (0..8)
        .map(|r| ink_pixels(img, rect, &cols, r, 0, crate::labels::TL_WINDOW).0)
        .collect();
    let letters: Vec<Vec<(u32, u32, f32)>> = (0..8)
        .map(|c| ink_pixels(img, rect, &cols, 7, c, crate::labels::BR_WINDOW).0)
        .collect();
    // Clean copy, then rotate square contents.
    let clean = erase_labels(img, rect);
    let mut out = clean.clone();
    for r in 0..8 {
        for c in 0..8 {
            let ((dx0, dy0), (sx0, sy0)) = (sq_origin(r, c), sq_origin(7 - r, 7 - c));
            for dy in 0..side {
                for dx in 0..side {
                    let p = *clean.get_pixel((sx0 + dx as i64) as u32, (sy0 + dy as i64) as u32);
                    out.put_pixel((dx0 + dx as i64) as u32, (dy0 + dy as i64) as u32, p);
                }
            }
        }
    }
    if !with_labels {
        return out;
    }
    // New labels: left column shows rank r + 1 (source square (7 - r, 0)), bottom row shows file
    // 7 - c (source square (7, 7 - c)).
    let out_frame = frame_of_image(&out);
    let out_cols = crate::labels::board_colours(&out_frame, rect);
    for k in 0..8 {
        for (r, c, glyph) in [(k, 0usize, &digits[7 - k]), (7, k, &letters[7 - k])] {
            let (ox, oy) = sq_origin(r, c);
            let label = crate::labels::label_candidates(&out_cols, r, c)[0];
            for &(dx, dy, a) in glyph {
                let p = out.get_pixel_mut((ox + dx as i64) as u32, (oy + dy as i64) as u32);
                for ch in 0..3 {
                    // The glyph is drawn over whatever is there (background or a piece edge).
                    let cur = p[ch] as f32;
                    p[ch] = (cur + a * (label[ch] - cur)).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}
