//! List capture targets and, for a given id, grab and detect: `cargo run -p fishy-vision --example targets -- [id]`
use fishy_vision::{capture, detect_board, Site};
fn main() {
    for t in capture::list_targets().unwrap() {
        if t.app.to_lowercase().contains("chess") || t.kind == "monitor" { println!("{t:?}"); }
    }
    if let Some(id) = std::env::args().nth(1) {
        let f = capture::grab(&id).unwrap();
        println!("frame {}x{}", f.width, f.height);
        println!("board {:?}", detect_board(&f, Site::Generic));
    }
}
