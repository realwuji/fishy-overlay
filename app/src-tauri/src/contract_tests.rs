//! Cross-language contract checks: the Rust wire types must match `src/shared/types.ts` exactly.
//!
//! `tests/fixtures/ts-contract.json` is generated from the TypeScript sources by
//! `node --experimental-strip-types scripts/export-defaults.mjs`; regenerate it whenever
//! `defaults.ts` or `types.ts` changes.

use crate::model::{
    BoardGeometry, BoardState, EVT_ANALYSIS, EVT_BOARD, EVT_OVERLAY_GEOMETRY, EVT_SETTINGS,
    EVT_STATUS, EngineStatus, LogicalRect, SessionPhase, SessionStatus,
};
use crate::settings::Settings;
use fishy_engine::{AnalysisFrame, CandidateMove, MoveClass, TacticTag};
use fishy_vision::{CaptureTarget, TrackState};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const FIXTURE: &str = include_str!("../tests/fixtures/ts-contract.json");
const TYPES_TS: &str = include_str!("../../src/shared/types.ts");
const LIB_RS: &str = include_str!("lib.rs");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture is valid JSON")
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}

/// Recursively compares object key sets. `null` on either side matches any scalar (optional values).
fn assert_same_shape(path: &str, rust: &Value, ts: &Value) {
    match (rust, ts) {
        (Value::Object(a), Value::Object(b)) => {
            let ka: BTreeSet<_> = a.keys().collect();
            let kb: BTreeSet<_> = b.keys().collect();
            assert_eq!(ka, kb, "key mismatch at `{path}` (rust vs ts)");
            for (k, va) in a {
                assert_same_shape(&format!("{path}.{k}"), va, &b[k]);
            }
        }
        (Value::Null, _) | (_, Value::Null) => {}
        (Value::Array(_), Value::Array(_)) => {}
        (Value::Bool(_), Value::Bool(_)) | (Value::String(_), Value::String(_)) => {}
        (Value::Number(_), Value::Number(_)) => {}
        _ => panic!("type mismatch at `{path}`: rust {rust:?} vs ts {ts:?}"),
    }
}

#[test]
fn default_settings_match_ts_default_settings() {
    let ts = fixture()["settings"].clone();
    let rust = Settings::default().to_value();
    assert_same_shape("settings", &rust, &ts);
    // Arrays that the UI indexes into must have the same length.
    assert_eq!(
        rust["arrows"]["rankColors"].as_array().unwrap().len(),
        ts["arrows"]["rankColors"].as_array().unwrap().len()
    );
    // Display defaults the user sees should agree too.
    for path in [
        "/site",
        "/arrows/count",
        "/arrows/style",
        "/labels/classes/best/color",
        "/labels/tags/fork/label",
        "/evalBar/position",
        "/hud/corner",
        "/engine/multiPv",
        "/capture/fps",
        "/hotkeys/toggleOverlay",
    ] {
        assert_eq!(
            rust.pointer(path),
            ts.pointer(path),
            "default differs at {path}"
        );
    }
}

#[test]
fn ts_default_settings_survive_the_rust_round_trip() {
    let ts = fixture()["settings"].clone();
    let parsed = Settings::from_value(ts.clone()).expect("TS defaults parse");
    assert_same_shape("roundtrip", &parsed.to_value(), &ts);
}

#[test]
fn move_class_and_tag_names_match() {
    let fx = fixture();
    let classes: Vec<String> = [
        MoveClass::Brilliant,
        MoveClass::Great,
        MoveClass::Best,
        MoveClass::Excellent,
        MoveClass::Good,
        MoveClass::Inaccuracy,
        MoveClass::Mistake,
        MoveClass::Blunder,
    ]
    .iter()
    .map(|c| {
        serde_json::to_value(c)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    })
    .collect();
    let ts_classes: Vec<String> = serde_json::from_value(fx["moveClasses"].clone()).unwrap();
    assert_eq!(classes, ts_classes);

    let tags: Vec<String> = [
        TacticTag::Fork,
        TacticTag::Pin,
        TacticTag::Skewer,
        TacticTag::Discovered,
        TacticTag::Hanging,
        TacticTag::Check,
        TacticTag::Mate,
        TacticTag::Capture,
        TacticTag::Promotion,
        TacticTag::Sacrifice,
        TacticTag::Castle,
        TacticTag::EnPassant,
    ]
    .iter()
    .map(|c| {
        serde_json::to_value(c)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    })
    .collect();
    let ts_tags: Vec<String> = serde_json::from_value(fx["tacticTags"].clone()).unwrap();
    assert_eq!(tags, ts_tags);
}

#[test]
fn analysis_frame_wire_names_match_types_ts() {
    let cand = CandidateMove {
        rank: 1,
        uci: "e2e4".into(),
        san: "e4".into(),
        from: "e2".into(),
        to: "e4".into(),
        score_cp: Some(30),
        mate: None,
        win_pct: 52.0,
        loss_pct: 0.0,
        cls: MoveClass::Best,
        tags: vec![TacticTag::Check],
        pv_uci: vec!["e2e4".into()],
        pv_san: vec!["e4".into()],
        depth: 10,
        explanation: Some("x".into()),
    };
    let frame = AnalysisFrame {
        fen: "8/8/8/8/8/8/8/8 w - - 0 1".into(),
        side_to_move: "w".into(),
        depth: 10,
        nps: 1,
        eval_cp: Some(30),
        mate: None,
        win_pct: 52.0,
        candidates: vec![cand],
        thinking: false,
        ts: 1,
        wdl: Some([500, 300, 200]),
        target_depth: Some(12),
    };
    let v = serde_json::to_value(&frame).unwrap();
    let expect: BTreeSet<String> = [
        "fen",
        "sideToMove",
        "depth",
        "nps",
        "evalCp",
        "mate",
        "winPct",
        "candidates",
        "thinking",
        "ts",
        "wdl",
        "targetDepth",
    ]
    .map(String::from)
    .into();
    assert_eq!(keys(&v), expect);
    let c = &v["candidates"][0];
    let expect: BTreeSet<String> = [
        "rank",
        "uci",
        "san",
        "from",
        "to",
        "scoreCp",
        "mate",
        "winPct",
        "lossPct",
        "cls",
        "tags",
        "pvUci",
        "pvSan",
        "depth",
        "explanation",
    ]
    .map(String::from)
    .into();
    assert_eq!(keys(c), expect);
    assert_eq!(c["cls"], "best");
    assert_eq!(c["tags"][0], "check");
}

#[test]
fn board_state_status_and_target_wire_names_match_types_ts() {
    let board = BoardState {
        geometry: Some(BoardGeometry {
            x: 1.0,
            y: 2.0,
            size: 3.0,
            flipped: false,
        }),
        fen: Some("f".into()),
        last_move: Some("e2e4".into()),
        state: TrackState::Tracking,
        confidence: 0.9,
        calibrated: true,
    };
    let v = serde_json::to_value(&board).unwrap();
    let expect: BTreeSet<String> = [
        "geometry",
        "fen",
        "lastMove",
        "state",
        "confidence",
        "calibrated",
    ]
    .map(String::from)
    .into();
    assert_eq!(keys(&v), expect);
    assert_eq!(
        keys(&v["geometry"]),
        ["x", "y", "size", "flipped"].map(String::from).into()
    );
    assert_eq!(v["state"], "tracking");
    assert_eq!(
        serde_json::to_value(TrackState::NeedsCalibration).unwrap(),
        "needsCalibration"
    );
    assert_eq!(serde_json::to_value(TrackState::Lost).unwrap(), "lost");
    // The idle state must serialise `geometry` as null, not omit it.
    assert!(serde_json::to_value(BoardState::idle()).unwrap()["geometry"].is_null());

    let status = SessionStatus {
        running: true,
        permission: false,
        capture_fps: 8,
        message: None,
        phase: SessionPhase::StartingEngine,
        engine: None,
    };
    let v = serde_json::to_value(&status).unwrap();
    assert_eq!(
        keys(&v),
        [
            "running",
            "permission",
            "captureFps",
            "message",
            "phase",
            "engine"
        ]
        .map(String::from)
        .into()
    );
    assert_eq!(v["phase"], "startingEngine");
    assert!(v["engine"].is_null());
    let eng = serde_json::to_value(EngineStatus {
        name: Some("Stockfish".into()),
        threads: 6,
        hash_mb: 512,
        nps: 1,
        ready: true,
    })
    .unwrap();
    assert_eq!(
        keys(&eng),
        ["name", "threads", "hashMb", "nps", "ready"]
            .map(String::from)
            .into()
    );

    let target = CaptureTarget {
        id: "monitor:0".into(),
        kind: "monitor".into(),
        title: "t".into(),
        app: "a".into(),
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        scale: 2.0,
    };
    let v = serde_json::to_value(&target).unwrap();
    assert_eq!(
        keys(&v),
        [
            "id", "kind", "title", "app", "x", "y", "width", "height", "scale"
        ]
        .map(String::from)
        .into()
    );

    let rect = serde_json::to_value(LogicalRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    })
    .unwrap();
    assert_eq!(
        keys(&rect),
        ["x", "y", "width", "height"].map(String::from).into()
    );
}

#[test]
fn event_names_match_ts() {
    let fx = fixture();
    let evt = &fx["events"];
    assert_eq!(evt["analysis"], json!(EVT_ANALYSIS));
    assert_eq!(evt["board"], json!(EVT_BOARD));
    assert_eq!(evt["status"], json!(EVT_STATUS));
    assert_eq!(evt["settings"], json!(EVT_SETTINGS));
    assert_eq!(evt["overlayGeometry"], json!(EVT_OVERLAY_GEOMETRY));
}

/// Command names declared in `Commands` (types.ts) with the argument names the TS side sends.
fn ts_commands() -> Vec<(String, Vec<String>)> {
    let start = TYPES_TS
        .find("export interface Commands")
        .expect("Commands interface");
    let body = &TYPES_TS[start..];
    let end = body.find("\n}").expect("end of interface");
    body[..end]
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (name, rest) = l.split_once('(')?;
            if name.contains(' ') || name.contains('/') {
                return None;
            }
            let args = rest.split_once(')')?.0;
            let names = args
                .split_once('{')
                .map(|(_, inner)| {
                    inner
                        .split('}')
                        .next()
                        .unwrap_or("")
                        .split(';')
                        .filter_map(|f| f.split_once(':').map(|(n, _)| n.trim().to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            Some((name.to_owned(), names))
        })
        .collect()
}

#[test]
fn every_ts_command_is_registered_in_rust_with_matching_argument_names() {
    let cmds = ts_commands();
    assert!(cmds.len() >= 15, "unexpected Commands parse: {cmds:?}");
    assert!(
        cmds.iter()
            .any(|(n, a)| n == "set_board_orientation" && a == &["orientation"])
    );
    let handler_start = LIB_RS.find("generate_handler![").expect("handler list");
    let handler =
        &LIB_RS[handler_start..LIB_RS[handler_start..].find(']').unwrap() + handler_start];
    let commands_rs = include_str!("commands.rs");
    for (name, args) in &cmds {
        assert!(
            handler.contains(&format!("commands::{name},")),
            "{name} not registered"
        );
        let sig_start = commands_rs
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("{name} missing in commands.rs"));
        let sig = &commands_rs[sig_start..];
        let sig = &sig[..sig.find(')').unwrap()];
        for a in args {
            // Tauri maps camelCase JS keys to snake_case parameters; these are all single words.
            assert!(
                sig.contains(&format!("{a}:")),
                "{name}: argument `{a}` missing in {sig}"
            );
        }
    }
    // Nothing is registered that the contract does not declare.
    let registered = handler.matches("commands::").count();
    assert_eq!(registered, cmds.len());
}

/// The control window's hotkey recorder emits `Ctrl+Alt+Shift+Cmd+<Key>`; the shell must parse every form.
#[test]
fn hotkeys_emitted_by_the_control_window_parse() {
    for s in [
        "Ctrl+Alt+K",
        "Alt+Space",
        "Ctrl+Shift+Cmd+Up",
        "Alt+Minus",
        "Ctrl+Alt+Shift+Cmd+F5",
        "Alt+H",
    ] {
        assert!(crate::hotkeys::parse_hotkey(s).unwrap().is_some(), "{s}");
    }
}
