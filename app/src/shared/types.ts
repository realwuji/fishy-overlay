// CONTRACT FILE: shared types between Rust (serde camelCase) and both windows.
// Owners may ADD optional fields but must not rename/remove existing ones.

export type MoveClass =
  | 'brilliant' | 'great' | 'best' | 'excellent' | 'good'
  | 'inaccuracy' | 'mistake' | 'blunder';

export type TacticTag =
  | 'fork' | 'pin' | 'skewer' | 'discovered' | 'hanging' | 'check' | 'mate'
  | 'capture' | 'promotion' | 'sacrifice' | 'castle' | 'enPassant';

export const MOVE_CLASSES: MoveClass[] = [
  'brilliant', 'great', 'best', 'excellent', 'good', 'inaccuracy', 'mistake', 'blunder',
];
export const TACTIC_TAGS: TacticTag[] = [
  'fork', 'pin', 'skewer', 'discovered', 'hanging', 'check', 'mate',
  'capture', 'promotion', 'sacrifice', 'castle', 'enPassant',
];

export interface CandidateMove {
  rank: number;            // 1-based MultiPV rank
  uci: string; san: string;
  from: string; to: string; // "e2"
  scoreCp: number | null;  // white POV
  mate: number | null;     // white POV
  winPct: number;          // white expected score 0..100
  lossPct: number;         // points lost vs best line
  cls: MoveClass;
  tags: TacticTag[];
  pvUci: string[]; pvSan: string[];
  depth: number;
  explanation?: string | null;
}

export interface AnalysisFrame {
  fen: string;
  sideToMove: 'w' | 'b';
  depth: number; nps: number;
  evalCp: number | null; mate: number | null; // white POV
  winPct: number;
  candidates: CandidateMove[];
  thinking: boolean;
  ts: number;
  /** White/draw/black win chances in per-mille (sum 1000), white POV. Additive. */
  wdl?: [number, number, number] | null;
  /** Depth the engine is heading for before the display is considered stable. Additive. */
  targetDepth?: number | null;
}

/** Board position in the OVERLAY window's logical (CSS px) coordinate space. */
export interface BoardGeometry {
  x: number; y: number; size: number;
  /** true = black at the bottom of the screen. */
  flipped: boolean;
}

export type TrackState = 'needsCalibration' | 'lost' | 'tracking';

export interface BoardState {
  geometry: BoardGeometry | null;   // null = no board found
  fen: string | null;
  lastMove: string | null;          // uci
  state: TrackState;
  confidence: number;               // 0..1
  calibrated: boolean;
}

export interface SessionStatus {
  running: boolean;
  permission: boolean;              // screen recording permission
  captureFps: number;
  message: string | null;           // human-readable status / last error
  /** Coarse pipeline phase so UIs can show loading states. Additive/optional. */
  phase?: SessionPhase;
  /** Engine facts for the UI (additive). */
  engine?: { name: string | null; threads: number; hashMb: number; nps: number; ready: boolean } | null;
}

/** startingEngine: spawning/warming Stockfish; findingBoard: capture running, no board yet;
 *  recognising: board found, reading pieces / joining the game; analysing: engine searching
 *  below the display depth; ready: showing stable analysis. */
export type SessionPhase = 'idle' | 'startingEngine' | 'findingBoard' | 'recognising' | 'analysing' | 'ready';

export interface CaptureTarget {
  id: string; kind: 'monitor' | 'window';
  title: string; app: string;
  x: number; y: number; width: number; height: number; scale: number;
}

// ---------------------------------------------------------------- Settings
export type ArrowStyle = 'classic' | 'slim' | 'dotted' | 'pill';
export type Corner = 'tl' | 'tr' | 'bl' | 'br';
export type PeekMode = 'off' | 'hold' | 'toggle';
export type Site = 'chessCom' | 'lichess' | 'generic';

export interface ClassStyle { enabled: boolean; label: string; color: string; icon: string; }
export interface TagStyle { enabled: boolean; label: string; color: string; icon: string; }

export interface Settings {
  version: 1;
  acceptedFairPlay: boolean;
  site: Site;
  arrows: {
    show: boolean;
    count: number;                 // 1..5 candidate arrows drawn
    style: ArrowStyle;
    curveKnightMoves: boolean;     // draw knight moves as L-shaped arrows
    opacity: number;               // 0.1..1
    widthScale: number;            // 0.5..1.6
    glow: boolean;
    animate: boolean;
    colorMode: 'rank' | 'classification';
    rankColors: string[];          // length 5
    forSide: 'sideToMove' | 'white' | 'black' | 'bottom';
    minClass: MoveClass | null;    // hide arrows worse than this (null = show all)
  };
  labels: {
    show: boolean;
    placement: 'cornerBadge' | 'arrowTail' | 'arrowHead';
    style: 'icon' | 'text' | 'iconText';
    size: number;                  // 0.6..1.6 scale
    showTags: boolean;             // fork / pin / hanging ... chips
    maxTags: number;               // per move 0..3
    classes: Record<MoveClass, ClassStyle>;
    tags: Record<TacticTag, TagStyle>;
  };
  evalBar: {
    show: boolean; position: 'left' | 'right'; width: number;
    showNumber: boolean; style: 'solid' | 'gradient'; animate: boolean;
    whiteColor: string; blackColor: string;
  };
  hud: {
    show: boolean; corner: Corner; lines: number; showPv: boolean;
    showExplanation: boolean; showDepth: boolean; showWinPct: boolean;
    opacity: number; scale: number; compact: boolean;
  };
  engine: {
    threads: number; hashMb: number; multiPv: number;
    depthLimit: number | null; moveTimeMs: number | null; skillLevel: number | null;
    /** true (default): Fishy picks threads/hash for this machine; the numbers above are then ignored. */
    auto?: boolean;
    /** Don't show arrows/labels until the search reached this depth (0 = always). */
    minDisplayDepth?: number;
  };
  /** `orientation`: which colour is at the bottom of the screen. Absent = 'auto' (detected). */
  capture: { fps: number; targetId: string | null; orientation?: 'auto' | 'white' | 'black' };
  theme: { mode: 'dark' | 'light' | 'system'; accent: string; uiScale: number; reduceMotion: boolean };
  hotkeys: { toggleOverlay: string; toggleHud: string; cycleArrows: string; recalibrate: string; peek: string };
  /** Screen-share privacy. */
  screenShare: { hideOverlay: boolean };
  /** Peek mode: analysis keeps running but the overlay only draws while the peek key says so. */
  peek: {
    mode: PeekMode;
    fadeMs: number;            // 0..1000
    autoHideSec: number;       // toggle mode: hide again after N s (0 = never)
    indicator: boolean;        // tiny dot while hidden
  };
}

// ------------------------------------------------------------ IPC contract
/** Events emitted by Rust (listen with @tauri-apps/api/event). */
export const EVT = {
  analysis: 'fishy://analysis',   // AnalysisFrame
  board: 'fishy://board',         // BoardState
  status: 'fishy://status',       // SessionStatus
  settings: 'fishy://settings',   // Settings (after any change, to all windows)
  peek: 'fishy://peek',           // { revealed: boolean } overlay content shown/hidden (peek mode)
  overlayGeometry: 'fishy://overlay-geometry', // { x,y,width,height } logical desktop rect of the overlay window
} as const;

/** Commands (invoke) implemented in src-tauri. */
export interface Commands {
  get_settings(): Settings;
  set_settings(args: { settings: Settings }): Settings;
  reset_settings(): Settings;
  list_capture_targets(): CaptureTarget[];
  start_session(): SessionStatus;
  stop_session(): SessionStatus;
  get_status(): SessionStatus;
  calibrate_start_position(): BoardState;   // user has the starting position on screen
  recalibrate_board(): BoardState;          // re-run board detection
  set_known_position(args: { fen: string }): BoardState; // resume mid-game from FEN
  calibrate_from_position(args: { fen: string }): BoardState; // learn the piece set from the screen + this FEN
  reset_pieces(): BoardState;               // drop the saved calibration, back to built-in pieces
  /** 'auto' detects it (default); 'white'/'black' is final. Stored as capture.orientation. */
  set_board_orientation(args: { orientation: 'auto' | 'white' | 'black' }): Settings;
  set_peek(args: { revealed: boolean }): void;
  get_peek(): boolean;
  set_overlay_visible(args: { visible: boolean }): void;
  set_click_through(args: { enabled: boolean }): void; // false = overlay editable (drag HUD)
  request_screen_permission(): boolean;
  open_screen_permission_settings(): void;
}
