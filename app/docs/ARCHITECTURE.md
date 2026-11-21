# Fishy architecture

Fishy is a Tauri 2 desktop app (Rust + TypeScript, macOS and Windows) that shows live Stockfish
analysis as a transparent overlay over any chess board on screen. It is a study and analysis tool.

## Data flow

```
 screen / window ──capture──▶ Frame (RGBA, memory only)
        │                         │
        │                  detect_board / verify_board     (fishy-vision)
        │                         │  BoardRect
        │                  Recognizer.read ──▶ Reading (64 squares, '?' = unknown)
        │                         │
        │                  PositionTracker.update ──▶ Tracked { fen, lastMove, state }
        │                         │
        │                  Engine.analyze(fen)  (Stockfish sidecar, MultiPV)   (fishy-engine)
        │                         │  EngineUpdate
        │                  build_frame ──▶ AnalysisFrame (classes, tags, explanations)
        ▼                         ▼
   session worker thread (src-tauri) ── emits events ──▶ overlay window (canvas)
                                                    └──▶ control window (settings + preview)
```

## Crates and folders

| Path | Role |
| --- | --- |
| `crates/fishy-vision` | capture (`xcap`), board detection, piece recognition and template learning, position tracker. `test-support` feature exposes a synthetic chess.com-style renderer. |
| `crates/fishy-engine` | UCI driver (std::process, reader and processor threads), analysis: win%, move classes, tactic tags (fork, pin, hanging...), one-line explanations. |
| `src-tauri` | Shell: settings store, session worker, overlay window, tray, hotkeys, IPC commands. |
| `src/shared` | The frozen contract: `types.ts`, `ipc.ts`, `defaults.ts`, `renderer-api.ts`, `demo.ts`. |
| `src/overlay` | Canvas renderer (arrows, badges, tag chips, eval bar, HUD) and the overlay entry (`overlay.html`). |
| `src/control` | Control window, vanilla TS: 9 settings sections plus live preview (`index.html`). |
| `scripts/build-stockfish.sh` | Builds the Stockfish sidecar from `/src` into `src-tauri/binaries/stockfish-<triple>`. |
| `scripts/export-defaults.mjs` | Dumps TS defaults to `src-tauri/tests/fixtures/ts-contract.json` for the Rust contract tests. |

## Windows

* `main`: control window (1040x760). Closing hides it to the tray while a session runs.
* `overlay`: transparent, always on top, click-through by default, sized to the capture target. It is an
  ordinary window: it is not hidden from screen capture or sharing.

## Commands and events

Commands (`invoke`, declared in `Commands` in `types.ts`): `get_settings`, `set_settings({settings})`,
`reset_settings`, `list_capture_targets`, `start_session`, `stop_session`, `get_status`,
`calibrate_start_position`, `recalibrate_board`, `set_known_position({fen})`,
`calibrate_from_position({fen})`, `reset_pieces`,
`set_board_orientation({orientation})` (`auto` | `white` | `black`, stored as `capture.orientation`),
`set_overlay_visible({visible})`, `set_click_through({enabled})`, `request_screen_permission`,
`open_screen_permission_settings`.

Events (`EVT`): `fishy://analysis` (AnalysisFrame, throttled to about 16 Hz), `fishy://board` (BoardState),
`fishy://status` (SessionStatus), `fishy://settings` (Settings, to all windows),
`fishy://overlay-geometry` (logical rect of the overlay).

`src-tauri/src/contract_tests.rs` checks command names and argument names, event names, serde camelCase
field names and the default Settings shape against the TS side. After editing `defaults.ts` or `types.ts`,
run `node --experimental-strip-types scripts/export-defaults.mjs`.

## Settings model

One `Settings` object (see `types.ts`): `site`, `arrows`, `labels` (8 move classes, 12 tactic tags, each
with enable, label, colour, icon), `evalBar`, `hud`, `engine`, `capture`, `theme`, `hotkeys`,
`acceptedFairPlay`. Rust types the fields it needs (site, engine, capture, hotkeys) and carries the rest
through `#[serde(flatten)]`, so new UI fields round-trip without Rust changes. Loading fills missing keys
from defaults and clamps ranges; a corrupt file is moved to `settings.json.bak`. Saves are atomic.
The control window edits immutably, debounces 250 ms, and saves through `set_settings`.

## Calibration (templates)

Piece artwork differs by site and theme, so Fishy learns it from the user's own screen. With the starting
position visible, `calibrate_start_position` runs `detect_board`, then `learn_from_start`: it finds the two
square colours, derives orientation from piece brightness, learns 12 piece templates (all kinds occur in the
start position) and verifies it can read the start position back. Templates are serialised into
`Templates.data` (kept in memory by the session). Calibrate without arrows over the pieces. Built-in
templates exist for the default chess.com and Lichess pieces. The tracker needs two identical frames, treats unknown
squares as wildcards, matches readings against legal moves, tolerates a dragged piece, and reports `lost`
after 3 unexplained stable frames before trying to resync.

### Calibrating mid-game

The start position is not required. `calibrate_from_position(fen)` (`session.rs`, logic in
`src-tauri/src/calibration.rs`) learns the piece set from the current screen plus a position the user
types in, so the ground truth comes from the user, never from the recognizer's own reading:

1. The FEN must parse and pass `placement_is_legal` (one king per side, at most 8 pawns, no pawns on the
   first or last rank, legal for some side to move).
2. Orientation: the manual setting, else coordinate labels, else both views are tried through
   `learn_from_placement_auto`. When only one fits the picture it wins; when both fit (an occupancy
   pattern that is symmetric) the view in which the white pieces are the lighter ones wins, and without a
   clear difference the call fails ("Could not tell which side is at the bottom") instead of guessing.
3. `learn_core` rejects a picture that does not match the position (pieces seen / empty squares clear) and
   a template set that cannot read the same frame back (more than 4 wrong squares).
4. Kinds missing from the position (for example no queens) get no template and are named in a note ("Not
   seen yet: queen, rook"); pressing Calibrate from this position again later adds them. If the saved
   calibration has the same paint colours (`merge_templates`), its kinds are carried over and the merged
   set must still read the frame; a different piece set is never mixed in.
5. The new set is installed only if it reads the frame at least as well as the set already active, then
   saved, the resolver is told the orientation (`note_structure`) and the tracker starts from the FEN.
   Any failure changes nothing. `reset_pieces` deletes the saved file so the built-in set applies again.

Limit: the self check reads the same frame the templates were learned from, so a FEN that swaps two kinds
consistently (every bishop typed as a knight) is not detected. Recovery is Reset to built-in pieces and
calibrating again. `set_known_position` (the Resume button) never learns and needs existing templates.

Orientation (which colour is at the bottom) is never defaulted: manual `capture.orientation` > chess.com
coordinate labels (`detect_orientation`) > legal-and-natural structure > calibration, re-checked every
16 frames and on board re-detection; undecided means "not tracking, press Calibrate or choose orientation"
(`fishy-vision/src/orient.rs`; inside `BoardPipeline` the tracker's own rotation logic is locked off).

## Adding support for a new site

1. Add a `Site` variant in `detect.rs` and a prior (board colour pairs) in `priors_for`; mirror it in the
   TS `Site` type and `defaults.ts`.
2. Run `cargo run -p fishy-vision --example inspect -- <screenshot.png> <site>` on real screenshots; tune
   `BG_GATE` / `PAL_GATE` in `recognize.rs` if cells read as unknown.
3. Add a synthetic theme to `test_support.rs` and extend the theme loops in `tests.rs`.
4. Add the site to the control window's site selector and the preview colours (`preview.ts`).
5. Update `scripts/export-defaults.mjs` fixture and run `cargo test --workspace --release`.

## Performance budgets

* Capture 8 fps by default (1..30). Detection about 12 ms at 2560x1440 only on (re)detection; per frame
  `verify_board` costs microseconds and reading a board a few ms.
* Engine: MultiPV 3, throttled updates, stale output suppressed; the UI receives at most about 16 analysis
  frames per second.
* Overlay: canvas, redraws only while a spring is moving or the scene changed; idle CPU is zero.
* Release profile uses `panic = "unwind"` so the session's `catch_unwind` guards work.

## Privacy and fair play

* Frames stay in memory on the device; never written to disk, never sent over a network.
* No telemetry. The CSP allows no remote origins. The only network access is the NNUE download inside
  `build-stockfish.sh`, at build time.
* No input is ever synthesised: no clicks or mouse moves are sent to other apps.
* No evasion: the overlay is not excluded from capture or sharing, there is no stealth mode and no timing
  randomisation. Starting a session requires acknowledging the fair-play notice (`acceptedFairPlay`).
* Fishy is for studying and analysing games. Do not use it in live rated play where assistance is prohibited.
