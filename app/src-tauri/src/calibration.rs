//! Calibration from a position the user states (any point of a game), as opposed to the start
//! position. Pure: takes a captured frame and returns what to install, so it is testable without
//! a running session. The caller saves and installs the result.

use crate::templates::Source;
use fishy_vision::{
    BoardRect, Frame, MSG_ORIENTATION_AMBIGUOUS, Orientation, Recognizer, Templates,
    learn_from_placement_auto, merge_templates, missing_kind_names, placement_is_legal,
    placement_of_fen,
};

pub const MSG_ILLEGAL_FEN: &str = "That position is not a legal chess position (check the kings, pawns on the first or last rank).";
pub const MSG_ORIENTATION: &str = "Could not tell which side is at the bottom. Set Board orientation in Fishy > Settings, then try again.";

/// What a successful calibration produced.
pub struct Calibrated {
    pub templates: Templates,
    pub orientation: Orientation,
    /// Kinds (names) the set has no template for yet.
    pub missing: Vec<&'static str>,
}

impl Calibrated {
    /// "Press again" note when some piece kinds were not on the board.
    pub fn partial_note(&self) -> Option<String> {
        (!self.missing.is_empty()).then(|| {
            format!(
                "Pieces learned. Not seen yet: {}. Press Calibrate from this position again once they appear on the board.",
                self.missing.join(", ")
            )
        })
    }
}

/// The FEN as the tracker wants it: a bare placement gets default trailing fields.
pub fn full_fen(fen: &str) -> String {
    let fen = fen.trim();
    if fen.split_whitespace().count() == 1 {
        format!("{fen} w - - 0 1")
    } else {
        fen.to_string()
    }
}

fn wrong_squares(rec: &Recognizer, frame: &Frame, rect: &BoardRect, o: Orientation, truth: &[char; 64]) -> usize {
    let r = rec.read(frame, rect, o);
    (0..64).filter(|&i| r.placement[i] != truth[i]).count()
}

/// Turn the library's occupancy error into the user-facing sentence.
fn describe(e: &str) -> String {
    if e.contains(MSG_ORIENTATION_AMBIGUOUS) {
        return MSG_ORIENTATION.to_string();
    }
    if let Some(rest) = e.split("match the position (").nth(1) {
        if let Some(counts) = rest.split(" pieces seen").next() {
            if let Some((n, m)) = counts.split_once('/') {
                return format!(
                    "The picture does not match this position ({n}/{m} pieces seen). Check the FEN, that the board is fully visible, and that nothing covers the pieces (arrows, popups)."
                );
            }
        }
    }
    if e.contains("do not reproduce the position") {
        return "The picture does not match this position. Check the FEN, that the board is fully visible, and that nothing covers the pieces (arrows, popups).".to_string();
    }
    format!("Could not learn the pieces from this position: {e}")
}

/// Learn the piece set from `frame` showing the position `fen`.
///
/// `hint` is the orientation known from settings or coordinate labels. `existing` is the active
/// recognizer and where it came from: its kinds are merged in when it was the user's own
/// calibration of the same piece set, and the new set must read the frame at least as well as it.
/// Nothing is installed or saved here; any failure leaves everything as it was.
pub fn learn_from_fen(
    frame: &Frame,
    rect: &BoardRect,
    fen: &str,
    hint: Option<Orientation>,
    existing: Option<(&Templates, &Recognizer, Source)>,
) -> Result<Calibrated, String> {
    let truth = placement_of_fen(fen).ok_or_else(|| MSG_ILLEGAL_FEN.to_string())?;
    if !placement_is_legal(&truth) {
        return Err(MSG_ILLEGAL_FEN.to_string());
    }
    let auto = learn_from_placement_auto(frame, rect, &truth, hint)
        .map_err(|e| describe(&e.to_string()))?;
    let orientation = auto.orientation;
    let mut templates = auto.templates;
    if let Some((base, _, Source::User)) = existing {
        let out = merge_templates(base, &templates, frame, rect, &truth, orientation);
        templates = out.templates;
    }
    let new_rec = Recognizer::new(&templates)
        .ok_or_else(|| "Could not learn the pieces from this position.".to_string())?;
    if let Some((_, old, _)) = existing {
        let (new_wrong, old_wrong) = (
            wrong_squares(&new_rec, frame, rect, orientation, &truth),
            wrong_squares(old, frame, rect, orientation, &truth),
        );
        if new_wrong > old_wrong {
            return Err(format!(
                "The pieces Fishy already knows read this position better ({old_wrong} wrong squares against {new_wrong}), so nothing was changed. Check the FEN, or use Reset to built-in pieces first."
            ));
        }
    }
    let missing = missing_kind_names(&fishy_vision::learned_kinds(&templates));
    Ok(Calibrated {
        templates,
        orientation,
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fishy_vision::test_support::{BLUE, GREEN, Scene, placement_from_fen, render};
    use fishy_vision::{Site, detect_board};

    const MID: &str = "r1bqk2r/ppp2ppp/2n2n2/3pp3/1b1PP3/2N2N2/PPP2PPP/R1BQKB1R";

    fn shot(fen: &str, white_bottom: bool) -> (Frame, BoardRect) {
        let mut sc = Scene::new(placement_from_fen(fen));
        sc.white_bottom = white_bottom;
        sc.theme = BLUE;
        let f = render(&sc);
        let r = detect_board(&f, Site::Generic).unwrap();
        (f, r)
    }

    #[test]
    fn learns_midgame_with_no_prior_templates() {
        for white_bottom in [true, false] {
            let (f, r) = shot(MID, white_bottom);
            let c = learn_from_fen(&f, &r, MID, None, None).expect("learns");
            let want = if white_bottom { Orientation::White } else { Orientation::Black };
            assert_eq!(c.orientation, want);
            assert!(c.missing.is_empty(), "{:?}", c.missing);
            assert!(c.partial_note().is_none());
            let rec = Recognizer::new(&c.templates).unwrap();
            let truth = placement_from_fen(MID);
            assert_eq!(wrong_squares(&rec, &f, &r, want, &truth), 0);
        }
    }

    #[test]
    fn bad_fen_or_picture_returns_an_error() {
        let (f, r) = shot(MID, true);
        for bad in ["not a fen", "8/8/8/8/8/8/8/4K3", "k6P/8/8/8/8/8/8/K7"] {
            assert_eq!(learn_from_fen(&f, &r, bad, None, None).err().as_deref(), Some(MSG_ILLEGAL_FEN), "{bad}");
        }
        // A legal position that is not the one on screen.
        let e = learn_from_fen(&f, &r, "4k3/8/8/8/8/8/8/4K3", None, None).err().unwrap();
        assert!(e.contains("pieces seen") || e.contains("Could not learn"), "{e}");
        let e = learn_from_fen(&f, &r, "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR", None, None).err().unwrap();
        assert!(e.contains("does not match this position"), "{e}");
    }

    #[test]
    fn partial_kinds_are_named() {
        let fen = "r1b1kb1r/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/R1B1KB1R";
        let (f, r) = shot(fen, true);
        let c = learn_from_fen(&f, &r, fen, None, None).unwrap();
        assert_eq!(c.missing, ["knight", "queen"]);
        assert!(c.partial_note().unwrap().contains("Not seen yet: knight, queen"));
    }

    #[test]
    fn a_worse_candidate_is_refused_and_a_bare_placement_is_completed() {
        // Existing recognizer: a perfect one for the same screen (BLUE).
        let (f, r) = shot(MID, true);
        let good = learn_from_fen(&f, &r, MID, None, None).unwrap();
        let rec = Recognizer::new(&good.templates).unwrap();
        // The wrong-theme recogniser reads fine here, so calibrating again is accepted ...
        let again = learn_from_fen(&f, &r, MID, None, Some((&good.templates, &rec, Source::User)));
        assert!(again.is_ok());
        // ... and an existing set that reads the frame perfectly beats a new one that is worse.
        let (gf, gr) = {
            let mut sc = Scene::new(placement_from_fen(MID));
            sc.theme = GREEN;
            let f = render(&sc);
            let r = detect_board(&f, Site::Generic).unwrap();
            (f, r)
        };
        let green = learn_from_fen(&gf, &gr, MID, None, None).unwrap();
        let grec = Recognizer::new(&green.templates).unwrap();
        let other = learn_from_fen(&gf, &gr, MID, None, Some((&green.templates, &grec, Source::BuiltIn)));
        assert!(other.is_ok());
        assert_eq!(full_fen(MID), format!("{MID} w - - 0 1"));
        assert_eq!(full_fen("8/8/8/8/8/8/8/K6k b - - 3 9"), "8/8/8/8/8/8/8/K6k b - - 3 9");
    }
}
