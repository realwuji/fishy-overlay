# Fishy engine layer

`crates/fishy-engine` drives a Stockfish child process and turns its output into
UI-ready analysis. It has no async runtime: plain `std::process` plus two helper threads.

## Pipeline

```
FEN --> Engine::analyze --> stockfish (UCI, MultiPV)
                                |
        reader thread: stdout lines -> channel
        processor thread: parse `info`, keep newest complete MultiPV set,
                          throttle (<= 20/s), deliver EngineUpdate via on_update
                                |
build_frame(&EngineUpdate, ts) --> AnalysisFrame (white POV, SAN, win%, loss%,
                                   class, tactic tags, explanation)
```

## `Engine` (uci.rs)

* `spawn` performs `uci`/`uciok` (capturing `id name`), applies `Threads`, `Hash`,
  `UCI_ShowWDL true` (and `Skill Level` only when `skill_level` is set), then `isready`/`readyok`.
  It returns only after `readyok`, so a spawned engine is ready.
* `info()` returns `EngineInfo { name, threads, hash_mb, multi_pv, ready }` for status displays.
* `new_game()` = `stop` (if searching) + `ucinewgame` + `isready`, blocking until `readyok`. It
  clears the hash and history; call it only for a real new game. **`analyze()` never clears the
  hash**, so consecutive positions of one game reuse the transposition table (measured: repeating a
  depth-20 search needs 39 % of the nodes of the cold one).
* `warm_up()` runs a silent depth-14 search (no callback updates) and returns its duration.
  Measured on the M4: 70-130 ms. It pays off mostly for the very first search after the OS has
  evicted the 140 MB binary (cold start `readyok` 0.75 s vs 0.19 s afterwards); once the process is
  running, depth 12 on the first real position already takes ~25 ms either way. Cheap, harmless.
* `RawLine.wdl` carries `wdl w d l` (per-mille, side-to-move POV); `build_frame` converts the top
  line's value to White's POV into `AnalysisFrame.wdl`, and leaves `AnalysisFrame.target_depth`
  `None` for the shell to fill in.
* `Move Overhead` and `nodestime` are **not** set: they only matter for clocked games, never for
  `go infinite`/`go depth`.
* `analyze(fen)` validates the FEN with shakmaty, clamps MultiPV to the number of legal
  moves, sends `position fen ...` and `go infinite | go depth N | go movetime N | both`.
  Positions without legal moves are answered immediately with an empty `done` update.
* **Stale output.** Each `go` yields exactly one `bestmove`. Switching positions sends
  `stop`, records one outstanding stale `bestmove`, and the processor discards every line
  until it passes. When `analyze` returns, no update for the old position is still in flight.
* Updates are emitted only when every MultiPV line of the newest depth is present and
  `info ... lowerbound/upperbound` lines are ignored. A throttled snapshot is flushed by a
  timer, so the final snapshot of a burst is never lost. `done = true` accompanies `bestmove`.
* `configure` only sends options whose value changed, resets `Skill Level` to 20 when it is
  cleared, and restarts a running search.
* `is_alive()` reports a crashed or exited engine; `quit()` and `Drop` send `quit` and
  kill the child if it does not exit within 1.5 s.
* The callback runs on the processor thread; keep it short.

## Analysis (analysis.rs)

* `win_percent(cp) = 50 + 50 * (2 / (1 + exp(-0.00368208 * cp)) - 1)`; mate scores map to 100/0.
* All scores are converted to White's point of view; `loss_pct` is the mover's expected-score
  loss against the best line.
* Classes: Best (rank 1 or loss < 0.05), Excellent (<= 2), Good (<= 5), Inaccuracy (<= 10),
  Mistake (<= 20), Blunder (> 20). **Great**: the rank-1 move when the runner-up loses >= 15
  points, or falls from >= 35% to <= 20%. **Brilliant**: Best/Excellent/Great and a genuine
  sacrifice (static exchange <= -2 pawns), mover still >= 45%, and no quiet alternative that
  already crushes (>= 85%).
* Tags are computed with real attack geometry: fork, pin, skewer, discovered attack/check,
  hanging, check, mate, capture, promotion, sacrifice, castle, en passant. A moved piece that is
  simply capturable is not credited with a fork/pin/skewer.
* `pv_uci`/`pv_san` are capped at 8 plies. Castling is reported as the king move (`e1g1`).

## Hardware policy (hardware.rs)

`Hardware::detect()` reads logical cores (`available_parallelism`), performance cores (macOS
`hw.perflevel0.physicalcpu`; unknown elsewhere) and RAM (macOS `hw.memsize`, Linux `/proc/meminfo`,
Windows `GlobalMemoryStatusEx` via `windows-sys`; the Windows path is not compiled on macOS CI, check
it on a Windows host). `recommended_config(&hw)`:

| rule | value |
|------|-------|
| threads, hybrid CPU (P < logical) | `P + (logical - P) / 3` |
| threads, uniform / unknown | `logical - max(2, ceil(logical / 3))` |
| threads clamp | `1..=16`, and at most 1 per 2 GB RAM |
| hash | largest power of two `<= RAM/32`, clamped to `16..=1024` MB |
| MultiPV 3, no skill limit, no depth/time limit | |

| machine | threads | hash |
|---------|---------|------|
| 2-core, 4 GB | 1 | 128 |
| 8-core, 16 GB | 5 | 512 |
| M4 4P+6E, 16 GB | 6 | 512 |
| 32-core, 64 GB | 16 | 1024 |

### Measurements (Apple M4, 10 cores = 4P+6E, 16 GB, binary stockfish dev-20261120, apple-silicon)

Thread scaling (`bench 256 T 15`, nodes/s) and 5 s of MultiPV 3 on the real chess.com mid-game
position (final nps reported by the engine):

| Threads | bench nodes/s | x vs 1T | analysis nps |
|--------:|--------------:|--------:|-------------:|
| 1 | 1.01 M | 1.00 | 0.77 M |
| 2 | 1.80 M | 1.78 | 1.30 M |
| 3 | 2.52 M | 2.48 | 1.86 M |
| 4 | 2.87 M | 2.83 | 2.35 M |
| 5 | 3.72 M | 3.67 | 2.69 M |
| 6 | 3.80 M | 3.75 | 2.89 M |
| 8 | 4.38 M | 4.31 | 3.34 M |
| 10 | 4.09 M | 4.03 | 3.45 M |

* Scaling is near-linear while threads fit on the 4 P cores plus a few E cores (an E core is worth
  roughly half a P core); beyond 8 it flattens and the 10-thread bench is *slower* than 8 (all cores
  saturated, scheduler contention, noisy). 6 threads deliver ~87 % of the 8-thread nodes/s and leave
  4 cores (including 2-4 E cores) for Chrome, screen capture and the overlay. That is the policy.
* Hash (6 threads, MultiPV 3, 10 s on the real position; bench depth 17): nodes/s are flat from 64 to
  1024 MB (3.3-3.6 M), fall by ~8 % at 2048 MB and ~20 % at 4096 MB (page-in and TLB cost, and
  memory pressure next to a browser). Table fullness after 10 s: 64 MB 98 %, 128 MB 76 %,
  256 MB 46 %, 512 MB 23 %, 1 GB 12 %. Infinite analysis keeps running for minutes and the hash is
  kept between moves, so 512 MB is the sweet spot on 16 GB; the cap is 1 GB anywhere.
  `readyok` after resizing takes 0.2 s (<= 128 MB) to 0.9 s (>= 2 GB).
* Time to depth (6 threads, 512 MB, MultiPV 3, after `ucinewgame`): real mid-game position
  d12 0.02 s, d16 0.08 s, d20 0.39 s, d24 1.7 s, d28 6.2 s. A sharp middlegame:
  d12 0.07 s, d16 0.9 s, d20 4.7 s, d24 19 s, d28 59 s. So a display depth of 16-18 is reached
  well inside the 2.5 s budget for ordinary positions, and `minDisplayDepth` 12 is nearly instant.
* Memory: system had 52 % free with a browser open; the 512 MB hash + ~140 MB binary/network is
  well within that.

## Stability helper (analysis.rs)

`StabilityTracker::feed(&EngineUpdate) -> Stability { stable, reason, depth, target_depth }` for one
position (a new FEN resets it). Defaults: min depth 12, best move unchanged for 3 consecutive
depths, win% swing of the best line <= 4 points over those depths. Immediate stability:
`Forced` (one legal move, target depth 1), `Mate` (any mate score on line 1), `Finished`
(`done`). A position that stays settled for 6 depths lowers `target_depth` to the current depth
(floor 8), so quiet positions do not wait for the full depth. `initial_target(fen)` gives the
target before the first snapshot arrives.

## Building the sidecar

```sh
app/scripts/build-stockfish.sh            # idempotent; --force rebuilds, --arch X overrides
```

It picks the architecture (`apple-silicon` on arm64 macOS; on x86 the most specific of
`x86-64-vnni512`, `x86-64-avx512`, `x86-64-avxvnni`, `x86-64-bmi2`, `x86-64-avx2`,
`x86-64-sse41-popcnt`, `x86-64`; `general-64` otherwise), runs `make net` (the only network access,
to the official Stockfish network server) and builds with **`make profile-build`** (PGO; `--no-pgo`
for a plain `make build`). The new binary is smoke-tested (`uci`, `isready`, `go depth 10`) *before*
it replaces `app/src-tauri/binaries/stockfish-<rust host triple>[.exe]`; the previous binary is
copied to `app/.engine-backup/`. The network is embedded in the binary. A PGO build takes ~45 s on
the M4.

PGO result on the M4 (`bench 256 T 16`, 3 runs each, old = previous binary): 1 thread old
0.92/0.86/0.89 M nps vs new 0.84/0.87/0.88 M; 6 threads old 3.34/3.11/3.39 M vs new
3.25/3.07/3.64 M. Within noise: the previous binary was already built with the same flags, so
PGO brings nothing measurable on Apple silicon here. The PGO binary is installed because the script
now reproduces it; expect the gain on x86 (typically a few percent) rather than here.

### Windows (MSYS2 MinGW-w64)

`profile-build` works under MinGW because the instrumented binary runs on the build host (no
cross-compiling). Use a native MSYS2 MinGW-w64 shell so that bench can run.

1. Install MSYS2 and, in the *MINGW64* shell: `pacman -S make mingw-w64-x86_64-gcc mingw-w64-x86_64-rust curl`
   (or use a rustup toolchain with `x86_64-pc-windows-gnu` / `msvc` on PATH so `rustc -vV` works).
2. `cd app && bash scripts/build-stockfish.sh`
   The script selects `COMP=mingw` and writes `stockfish-x86_64-pc-windows-msvc.exe` (or the
   triple reported by `rustc`).
3. To force a baseline CPU: `bash scripts/build-stockfish.sh --arch x86-64-avx2`.

## Tests

```sh
cargo test -p fishy-engine       # unit tests + integration tests against the real binary
cargo clippy -p fishy-engine --all-targets -- -D warnings
```

The integration tests skip with a message when no binary is present. `FISHY_STOCKFISH=/path`
points them at another binary.
