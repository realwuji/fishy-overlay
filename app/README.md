# Fishy

Fishy is a desktop study tool that puts live Stockfish analysis on top of the chess board you already have on screen: best-move arrows, move-quality labels (best, excellent, good, blunder ...), tactic tags (fork, pin, hanging ...), an evaluation bar and a small HUD. It works as a transparent overlay, so the board can be in any browser or app. The first target is **chess.com** board detection; other sites and styles come later.

Everything is customisable from the control window: how many arrows, which move classes and tactic tags are shown, colours, icons, HUD, eval bar, hotkeys, engine strength and capture rate.

Built with Tauri 2 (Rust + TypeScript) for macOS and Windows.

## Fair play

Fishy is for **study and analysis**: reviewing your own finished games, analysis boards, puzzles you are allowed to analyse, and learning from engine lines. Do not use it during live rated or competitive games, or anywhere engine assistance is not permitted. Doing so breaks the rules of chess sites and is cheating.

By design Fishy never sends mouse or keyboard input to other apps, never hides itself from screen capture or screen sharing, and keeps captured frames in memory only. The session will not start until you accept the fair-play notice in the app.

## Run it (development)

Prerequisites: Node 20+, a Rust toolchain (stable), and a C++ toolchain to build Stockfish (Xcode command line tools on macOS, MSVC or MinGW on Windows).

```sh
cd app
npm install
scripts/build-stockfish.sh      # builds the engine into src-tauri/binaries/stockfish-<target-triple>
npm run tauri dev
```

`scripts/build-stockfish.sh` is the only step that touches the network (it downloads Stockfish's NNUE network). The app itself makes no network calls.

## macOS permission

Fishy reads the screen, so macOS needs **Screen Recording** permission: System Settings > Privacy & Security > Screen & System Audio Recording > enable Fishy (in dev, enable the terminal or app that launched `tauri dev`). The control window has a button that triggers the prompt and one that opens the right settings pane. macOS may ask you to restart the app after granting it. Windows needs no permission.

## How it works

```
 control window (index.html)                  overlay window (overlay.html)
 settings UI + live preview                   transparent, always on top, click-through
        |  invoke / events                            ^  events
        v                                             |
 +--------------------------------------------------- Tauri shell (src-tauri) ----+
 | commands.rs   settings.rs   tray.rs / hotkeys.rs / actions.rs   overlay.rs     |
 |                                                                                 |
 | session.rs  (one worker thread, ticks at capture.fps)                           |
 |                                                                                 |
 |  grab frame --> detect/verify board --> read pieces --> track position (FEN)    |
 |  (fishy-vision)    (fishy-vision)       (templates)      (fishy-vision)         |
 |                                                              |                  |
 |            EVT.board  <-- board rect -> overlay coordinates  |  FEN changed     |
 |                                                              v                  |
 |            EVT.analysis <-- classify + tag <-- UCI MultiPV <-- Stockfish        |
 |                             (fishy-engine)    (fishy-engine)    (sidecar)       |
 +---------------------------------------------------------------------------------+
```

* **Capture and vision** (`crates/fishy-vision`): screen capture, board detection tuned to chess.com themes, piece recognition from templates learned from your own screen, and a position tracker that turns noisy readings into legal moves.
* **Engine** (`crates/fishy-engine`): UCI driver for the bundled Stockfish sidecar, win-probability model, move classification and tactic tagging.
* **Shell** (`src-tauri`): orchestration, settings persistence, overlay window management, tray and global hotkeys.
* **Web** (`src`): the overlay renderer (canvas) and the control window. The shared contract lives in `src/shared`.

### First-time use

1. Open Fishy, accept the fair-play notice, pick a screen or window (default: primary monitor).
2. Start the session.
3. Put the standard starting position on the board and press **Calibrate**. Fishy learns your piece set from it (stored per site in the app data folder as small JSON files, never images). To resume mid-game, enter the FEN via *Set known position*.
4. Move the board or change theme? Use **Recalibrate** (default hotkey `Alt+R`) to re-detect it.

Default hotkeys (changeable): `Alt+Space` toggle overlay, `Alt+H` toggle HUD, `Alt+A` cycle arrow count 1-5, `Alt+R` recalibrate. The tray icon offers the same actions plus start/stop. Closing the control window while a session runs hides it to the tray.

### Where data lives

Settings: `settings.json` in the app config directory (a corrupt file is kept as `settings.json.bak` and defaults are used). Piece templates: `templates-<site>.json` in the app data directory. Nothing else is written; captured frames are never saved or sent anywhere.

## Build and bundle

```sh
npm run tauri build                         # host platform
```

* **macOS**: produces `Fishy.app` and a `.dmg` under `src-tauri/target/release/bundle/`. Build the engine for the target architecture first (`scripts/build-stockfish.sh`); the sidecar is placed next to the executable inside the bundle. For distribution, sign and notarise the app with your Developer ID.
* **Windows**: run the same commands on Windows (build the engine with MSVC or MinGW); produces an NSIS installer (`.exe`) under `src-tauri\target\release\bundle\nsis\`.

The sidecar name must match the target triple, for example `src-tauri/binaries/stockfish-aarch64-apple-darwin` or `stockfish-x86_64-pc-windows-msvc.exe`.

## Tests and checks

```sh
cargo check -p fishy
cargo clippy -p fishy -- -D warnings
cargo test -p fishy            # settings, coordinate conversion, hotkeys, throttling, engine path
npx tsc --noEmit
```

## Project layout

```
app/
  src/                 TypeScript: shared contract, overlay renderer, control UI
  src-tauri/           Tauri shell (Rust): settings, session, overlay, tray, hotkeys, commands
  crates/fishy-vision  capture, board detection, recognition, tracking
  crates/fishy-engine  Stockfish driver, classification, tactic tags
  scripts/             build-stockfish.sh
```

## Signing (macOS)

macOS ties Screen Recording permission to the app's code signature. An ad-hoc
signed build gets a new fingerprint on every rebuild, which invalidates the
permission. Sign release builds with a stable Developer ID identity instead, so
the permission survives rebuilds:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" npm run tauri build
```

Keep the identity in the environment, not in `tauri.conf.json`. To distribute
the `.dmg` to other people it must also be notarized (set `APPLE_ID`,
`APPLE_PASSWORD` (app-specific) and `APPLE_TEAM_ID`, or an App Store Connect API
key, before building; Tauri then submits and staples it).

### Notarizing a release (macOS)

One-time: store a notary profile (prompts for an app-specific password; it is
kept in your keychain, never in the repo):

```sh
xcrun notarytool store-credentials "fishy-notary" --apple-id you@example.com --team-id TEAMID
```

Then, after building the signed `.dmg`:

```sh
xcrun notarytool submit Fishy_0.1.0_aarch64.dmg --keychain-profile fishy-notary --wait
xcrun stapler staple Fishy_0.1.0_aarch64.dmg
spctl -a -vv -t open --context context:primary-signature Fishy_0.1.0_aarch64.dmg   # expect "Notarized Developer ID"
```

If `tauri build` fails with "can't find crate" for a proc-macro on macOS 27,
run `cargo clean --release -p tauri-macros -p tauri` and build again (the
workspace compiles build-time macros unoptimised to avoid a dlopen problem).
