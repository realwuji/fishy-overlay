//! Add coordinate-label glyph masks learned from a clean screenshot to a templates JSON:
//! `cargo run -p fishy-vision --example add_labels -- <png> <templates.json> <white|black>`
use fishy_vision::{Frame, Orientation, Site, Templates, detect_board, detect_orientation_with, learn_labels};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [png, tj, side] = a.as_slice() else { eprintln!("usage: add_labels <png> <templates.json> <white|black>"); std::process::exit(2) };
    let o = if side == "black" { Orientation::Black } else { Orientation::White };
    let img = image::open(png).expect("png").to_rgba8();
    let (w, h) = img.dimensions();
    let f = Frame { width: w, height: h, rgba: img.into_raw(), origin_x: 0, origin_y: 0, scale: 1.0 };
    let rect = detect_board(&f, Site::Lichess).expect("board");
    let mut t = Templates { data: serde_json::from_str(&std::fs::read_to_string(tj).unwrap()).unwrap() };
    learn_labels(&f, &rect, o, &mut t).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) });
    std::fs::write(tj, serde_json::to_string(&t.data).unwrap()).unwrap();
    println!("labels added; orientation read back: {:?} (expected {o:?})", detect_orientation_with(&f, &rect, &t));
}
