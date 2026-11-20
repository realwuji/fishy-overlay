# Contributing to Fishy

Thanks for helping! Fishy is a study and analysis tool, so contributions should
keep it that way: no features meant to evade detection by chess sites, no input
automation, and captured screen frames must never leave memory.

## Getting started

```
git clone --recurse-submodules https://github.com/realwuji/fishy-overlay.git
cd fishy-overlay/app
npm install
scripts/build-stockfish.sh
npm run tauri dev
```

Read [app/docs/ARCHITECTURE.md](app/docs/ARCHITECTURE.md) first.

## Before opening a pull request

From `app/`:

```
npx tsc --noEmit && npx vite build
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
```

Tests that use real screenshots look for them in
`app/crates/fishy-vision/tests/fixtures/local/` (git-ignored, because they are
third-party artwork) and skip when absent. Do not commit screenshots.

## Good first areas

Support for more sites and piece sets (see "how to add support for a new site"
in the architecture doc), Windows testing, and detection robustness on real
screenshots (`cargo run -p fishy-vision --example inspect -- <png>`).
