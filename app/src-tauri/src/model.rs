//! IPC payload types owned by the shell. Field names mirror `src/shared/types.ts`
//! (camelCase on the wire).

use fishy_vision::TrackState;
use serde::{Deserialize, Serialize};

/// Board position in the OVERLAY window's logical (CSS px) coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardGeometry {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    /// true = black at the bottom of the screen.
    pub flipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardState {
    /// None = no board found.
    pub geometry: Option<BoardGeometry>,
    pub fen: Option<String>,
    /// UCI of the last inferred move.
    pub last_move: Option<String>,
    pub state: TrackState,
    /// 0..1
    pub confidence: f32,
    pub calibrated: bool,
}

impl BoardState {
    /// State before anything has been seen.
    pub fn idle() -> BoardState {
        BoardState {
            geometry: None,
            fen: None,
            last_move: None,
            state: TrackState::NeedsCalibration,
            confidence: 0.0,
            calibrated: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub running: bool,
    /// Screen recording permission.
    pub permission: bool,
    pub capture_fps: u32,
    /// Human-readable status / last error.
    pub message: Option<String>,
    /// Coarse pipeline phase so the UI can show loading states.
    pub phase: SessionPhase,
    /// Engine facts for the UI; `None` until an engine has been started.
    pub engine: Option<EngineStatus>,
}

/// Where the session is on its way from "Start" to a finished analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    #[default]
    Idle,
    StartingEngine,
    FindingBoard,
    Recognising,
    Analysing,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub name: Option<String>,
    pub threads: u32,
    pub hash_mb: u32,
    pub nps: u64,
    pub ready: bool,
}

/// Logical desktop rectangle (points / CSS px on the virtual desktop).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub const EVT_ANALYSIS: &str = "fishy://analysis";
pub const EVT_BOARD: &str = "fishy://board";
pub const EVT_STATUS: &str = "fishy://status";
pub const EVT_SETTINGS: &str = "fishy://settings";
pub const EVT_PEEK: &str = "fishy://peek";
pub const EVT_OVERLAY_GEOMETRY: &str = "fishy://overlay-geometry";

pub const MAIN_WINDOW: &str = "main";
pub const OVERLAY_WINDOW: &str = "overlay";
