//! Generate built-in templates from a screenshot with a known position:
//! `cargo run -p fishy-vision --example make_builtin -- <png> <placement> <white|black> <out.json>`
//!
//! `placement` is the piece-placement field of a FEN (`rnbqkbnr/ppp2ppp/...`); `white|black` is the
//! colour at the bottom of the screen. The output holds only 32x32 colour-class masks and a small
//! palette plus four label-glyph alpha masks (a few KB), never pixels of the screenshot. Nothing is
//! sent anywhere.

use fishy_vision::{
    Frame, Orientation, Placement, Site, detect_board, detect_orientation_with,
    learn_from_placement, learn_labels, read_board,
};

fn parse_placement(s: &str) -> Option<Placement> {
    let mut p = ['.'; 64];
    let rows: Vec<&str> = s.split('/').collect();
    if rows.len() != 8 {
        return None;
    }
    for (r, row) in rows.iter().enumerate() {
        let mut f = 0;
        for ch in row.chars() {
            if let Some(d) = ch.to_digit(10) {
                f += d as usize;
            } else if "PNBRQKpnbrqk".contains(ch) && f < 8 {
                p[r * 8 + f] = ch;
                f += 1;
            } else {
                return None;
            }
        }
        if f != 8 {
            return None;
        }
    }
    Some(p)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [png, placement, side, out] = args.as_slice() else {
        eprintln!("usage: make_builtin <png> <placement> <white|black> <out.json>");
        std::process::exit(2);
    };
    let Some(truth) = parse_placement(placement.split_whitespace().next().unwrap_or("")) else {
        eprintln!("invalid placement '{placement}'");
        std::process::exit(2);
    };
    let orientation = match side.as_str() {
        "white" => Orientation::White,
        "black" => Orientation::Black,
        _ => {
            eprintln!("side must be white or black");
            std::process::exit(2);
        }
    };
    let img = image::open(png)
        .unwrap_or_else(|e| {
            eprintln!("cannot read {png}: {e}");
            std::process::exit(1);
        })
        .to_rgba8();
    let (w, h) = img.dimensions();
    let frame = Frame {
        width: w,
        height: h,
        rgba: img.into_raw(),
        origin_x: 0,
        origin_y: 0,
        scale: 1.0,
    };
    let rect = detect_board(&frame, Site::ChessCom).unwrap_or_else(|| {
        eprintln!("no board found");
        std::process::exit(1);
    });
    println!("board at x={} y={} size={}", rect.x, rect.y, rect.size);
    let mut t = learn_from_placement(&frame, &rect, &truth, orientation).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    // Coordinate-label glyph masks (alpha maps of `8 1 h a`, a few hundred bytes each).
    if let Err(e) = learn_labels(&frame, &rect, orientation, &mut t) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let json = serde_json::to_string(&t.data).expect("serialise");
    std::fs::write(out, &json).expect("write output");
    let r = read_board(&frame, &rect, &t, orientation);
    let wrong = (0..64).filter(|&i| r.placement[i] != truth[i]).count();
    println!(
        "wrote {out} ({} bytes); read-back: {wrong} wrong squares, confidence {:.2}, mover {:?}",
        json.len(),
        r.confidence,
        r.mover_hint
    );
    println!(
        "label orientation on this image: {:?} (expected {orientation:?})",
        detect_orientation_with(&frame, &rect, &t)
    );
}
