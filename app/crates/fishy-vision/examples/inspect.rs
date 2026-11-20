//! Inspect a screenshot: `cargo run -p fishy-vision --example inspect -- <png> [chesscom|lichess|generic]`
//!
//! Loads a PNG, prints the detected board rectangle and, if the board shows the
//! standard start position, the learned orientation and the placement read back
//! with the freshly learned templates. An annotated copy is written next to the
//! input as `<name>.annotated.png` (board outline, 8x8 grid, recognised pieces).
//! Nothing is sent anywhere and no other file is written.

use fishy_vision::{
    BoardRect, Frame, Orientation, Site, detect_board, learn_from_start, read_board, verify_board,
};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

/// 3x5 glyphs (rows top to bottom, 3 bits each) for the piece letters and '?'.
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c.to_ascii_uppercase() {
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'N' => [0b101, 0b111, 0b111, 0b111, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        '?' => [0b110, 0b001, 0b010, 0b000, 0b010],
        _ => return None,
    })
}

fn put(img: &mut RgbaImage, x: i64, y: i64, c: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

fn rect_outline(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, c: Rgba<u8>) {
    let (x0, y0, x1, y1) = (
        x.round() as i64,
        y.round() as i64,
        (x + w).round() as i64,
        (y + h).round() as i64,
    );
    for t in 0..2 {
        for i in x0..=x1 {
            put(img, i, y0 + t, c);
            put(img, i, y1 - t, c);
        }
        for j in y0..=y1 {
            put(img, x0 + t, j, c);
            put(img, x1 - t, j, c);
        }
    }
}

fn letter(img: &mut RgbaImage, cx: f32, cy: f32, ch: char, scale: i64, fg: Rgba<u8>, bg: Rgba<u8>) {
    let Some(g) = glyph(ch) else { return };
    let (ox, oy) = (cx as i64 - 3 * scale / 2, cy as i64 - 5 * scale / 2);
    for (r, row) in g.iter().enumerate() {
        for c in 0..3 {
            let on = (row >> (2 - c)) & 1 == 1;
            for dy in 0..scale {
                for dx in 0..scale {
                    put(
                        img,
                        ox + c * scale + dx - 1,
                        oy + r as i64 * scale + dy - 1,
                        if on { fg } else { bg },
                    );
                }
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: inspect <png> [chesscom|lichess|generic]");
        std::process::exit(2);
    };
    let site = match args
        .next()
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("chesscom") | Some("chess.com") => Site::ChessCom,
        Some("lichess") => Site::Lichess,
        Some("generic") => Site::Generic,
        Some(other) => {
            eprintln!("unknown site '{other}' (chesscom | lichess | generic)");
            std::process::exit(2);
        }
    };
    let img = match image::open(&path) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = img.dimensions();
    let frame = Frame {
        width: w,
        height: h,
        rgba: img.clone().into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    };
    println!("image {w}x{h}, site {site:?}");

    let started = std::time::Instant::now();
    let rect: Option<BoardRect> = detect_board(&frame, site);
    println!(
        "detect_board: {:?} in {:.1} ms",
        rect,
        started.elapsed().as_secs_f64() * 1000.0
    );
    let mut out = img;
    let Some(rect) = rect else {
        println!("no board found");
        save(&path, &out);
        std::process::exit(1);
    };
    println!("verify_board: {}", verify_board(&frame, &rect, site));
    let sq = rect.size / 8.0;
    println!(
        "board x={:.2} y={:.2} size={:.2} (square {:.2} px)",
        rect.x, rect.y, rect.size, sq
    );

    let green = Rgba([0, 220, 80, 255]);
    rect_outline(&mut out, rect.x, rect.y, rect.size, rect.size, green);
    for i in 1..8 {
        let o = i as f32 * sq;
        for t in 0..rect.size as i64 {
            put(
                &mut out,
                (rect.x + o) as i64,
                rect.y as i64 + t,
                Rgba([0, 160, 255, 140]),
            );
            put(
                &mut out,
                rect.x as i64 + t,
                (rect.y + o) as i64,
                Rgba([0, 160, 255, 140]),
            );
        }
    }

    match learn_from_start(&frame, &rect) {
        Ok((templates, orientation)) => {
            let reading = read_board(&frame, &rect, &templates, orientation);
            println!(
                "start position learned; orientation: {} at the bottom; confidence {:.2}",
                if orientation == Orientation::White {
                    "white"
                } else {
                    "black"
                },
                reading.confidence
            );
            for r in 0..8 {
                let line: String = (0..8)
                    .map(|c| {
                        reading.placement[if orientation == Orientation::White {
                            r * 8 + c
                        } else {
                            63 - (r * 8 + c)
                        }]
                    })
                    .collect();
                println!("  {line}");
            }
            let scale = ((sq / 14.0) as i64).max(1);
            for r in 0..8 {
                for c in 0..8 {
                    let idx = if orientation == Orientation::White {
                        r * 8 + c
                    } else {
                        63 - (r * 8 + c)
                    };
                    let ch = reading.placement[idx];
                    if ch == '.' {
                        continue;
                    }
                    let (fg, bg) = if ch.is_ascii_uppercase() {
                        (Rgba([0, 0, 0, 255]), Rgba([255, 255, 255, 230]))
                    } else {
                        (Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 230]))
                    };
                    letter(
                        &mut out,
                        rect.x + (c as f32 + 0.5) * sq,
                        rect.y + (r as f32 + 0.5) * sq,
                        ch,
                        scale,
                        fg,
                        bg,
                    );
                }
            }
        }
        Err(e) => println!("not the start position (or not recognisable): {e}"),
    }
    save(&path, &out);
}

fn save(input: &str, img: &RgbaImage) {
    let p = Path::new(input);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let target: PathBuf = p.with_file_name(format!("{stem}.annotated.png"));
    match img.save(&target) {
        Ok(()) => println!("annotated image: {}", target.display()),
        Err(e) => eprintln!("cannot write {}: {e}", target.display()),
    }
}
