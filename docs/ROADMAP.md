# Fishy Roadmap

Goal: a beautiful screen overlay that reads a chess board from any app, browser
or website and shows live Stockfish analysis on top of it, for training and
education, on macOS and Windows.

Fishy is for study and analysis only. See "Fair play and intended use" in the
README.

Status key: [ ] planned, [~] in progress, [x] done.

## Phase 0: Foundations
- [x] Fork Stockfish and set up the repository
- [x] Logo, brand assets and overlay design concept
- [x] Project vision, README and fair-play note
- [x] Stack decision: Tauri 2 (Rust core, TypeScript UI)
- [x] Scaffold the Tauri app (`app/`) and bundle Stockfish as a sidecar
- [x] UCI bridge: launch the engine, send commands, parse `info` output

## Phase 1: Overlay and capture
- [x] Transparent, always-on-top, click-through overlay window
- [x] Screen capture via xcap (macOS and Windows), needs testing on real setups
- [x] Capture target picker (monitor or window)
- [x] Permissions flow (macOS Screen Recording)

## Phase 2: Board detection
- [x] Locate the 8x8 board in a frame (chess.com themes first); tune on real screenshots
- [x] Recognise pieces by learning templates from your own screen (any piece set)
- [x] Detect orientation and side to move, build the FEN
- [x] Infer moves between frames and ignore animations and highlights

## Phase 3: Live analysis
- [x] Best-move arrows aligned to the real board
- [x] Animated evaluation bar and floating panel
- [x] MultiPV candidates, depth and speed readout
- [x] Light and dark themes

## Phase 4: Teaching layer
- [x] Plain-language explanations for every candidate move
- [x] Tactic tags: fork, pin, skewer, hanging, check, mate and more
- [ ] Hint mode that reveals help step by step

## Next up
- [ ] Test and tune detection and recognition on real chess.com screenshots (use `cargo run -p fishy-vision --example inspect`)
- [ ] Windows build and DPI testing
- [ ] Support more sites and piece sets (lichess, desktop apps)
- [ ] Smaller installer (the Stockfish binary with embedded network is about 140 MB)

## Phase 5: Polish and release
- [x] Global hotkeys and tray; [ ] multi-monitor and Windows DPI testing
- [x] Fully customisable settings: arrow count and styles, move classes, tags, HUD, eval bar
- [ ] macOS (universal) and Windows installers, auto-update
- [ ] Public beta
