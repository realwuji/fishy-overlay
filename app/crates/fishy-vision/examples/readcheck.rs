//! Read a screenshot with a templates JSON and compare with a known placement:
//! `cargo run -p fishy-vision --example readcheck -- <png> <templates.json> <placement> <white|black>`
use fishy_vision::{Frame, Orientation, Site, Templates, detect_board, read_board, structural_orientation};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [png, tj, placement, side] = a.as_slice() else { eprintln!("usage: readcheck <png> <templates.json> <placement> <white|black>"); std::process::exit(2) };
    let o = if side == "black" { Orientation::Black } else { Orientation::White };
    let img = image::open(png).expect("png").to_rgba8();
    let (w, h) = img.dimensions();
    let f = Frame { width: w, height: h, rgba: img.into_raw(), origin_x: 0, origin_y: 0, scale: 1.0 };
    let rect = detect_board(&f, Site::Lichess).expect("board");
    let t = Templates { data: serde_json::from_str(&std::fs::read_to_string(tj).unwrap()).unwrap() };
    let r = read_board(&f, &rect, &t, o);
    let mut truth = Vec::new();
    for row in placement.split('/') { for c in row.chars() { if let Some(d) = c.to_digit(10) { truth.extend(std::iter::repeat('.').take(d as usize)); } else { truth.push(c); } } }
    let mut wrong = 0;
    for i in 0..64 { if r.placement[i] != truth[i] { wrong += 1; println!("square {i}: read '{}' expected '{}'", r.placement[i], truth[i]); } }
    let rw = read_board(&f, &rect, &t, Orientation::White);
    println!("structural orientation (no labels): {:?}", structural_orientation(&rw.placement, rw.mover_hint, rw.last_move_squares));
    println!("{wrong} wrong squares, confidence {:.2}", r.confidence);
}
