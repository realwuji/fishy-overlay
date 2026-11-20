#!/usr/bin/env bash
# Build Stockfish from the fork in <repo>/src for the host machine and install it
# as a Tauri sidecar: app/src-tauri/binaries/stockfish-<rust host triple>[.exe]
#
# Usage:  scripts/build-stockfish.sh [--force] [--arch <STOCKFISH_ARCH>] [--no-pgo]
# Idempotent: skips the build when the installed binary is newer than every
# source file, unless --force is given.
#
# Builds with `make profile-build` (PGO: the compiler first runs an instrumented
# bench, then recompiles with that profile). --no-pgo falls back to `make build`.
# The new binary is smoke-tested BEFORE it replaces the installed one, and the
# previous binary is kept in app/.engine-backup/<name>.bak.
#
# Network use: only `make net` (downloads the NNUE networks from the official
# Stockfish data server). Nothing else touches the network.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SRC_DIR="$(cd "$APP_DIR/../engine/stockfish/src" && pwd)"
OUT_DIR="$APP_DIR/src-tauri/binaries"

FORCE=0
PGO=1
ARCH=""
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1 ;;
    --no-pgo) PGO=0 ;;
    --arch) ARCH="${2:?--arch needs a value}"; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# ---- host triple -----------------------------------------------------------
if command -v rustc >/dev/null 2>&1; then
  TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
else
  echo "rustc not found; cannot determine the host target triple" >&2
  exit 1
fi
EXE=""
case "$TRIPLE" in *windows*) EXE=".exe" ;; esac
DEST="$OUT_DIR/stockfish-$TRIPLE$EXE"

# ---- architecture selection ------------------------------------------------
norm() { tr '[:upper:]' '[:lower:]' | tr -d '_.' ; }   # sse4_1 / SSE4.1 / avx512_vnni / AVX512VNNI -> comparable
has_flag() { # has_flag <cpu flag> (x86 Linux/MSYS via /proc/cpuinfo, Intel macOS via sysctl)
  local want; want="$(echo "$1" | norm)"
  if [ -r /proc/cpuinfo ]; then
    grep -m1 -i '^flags' /proc/cpuinfo | tr ' ' '\n' | norm | grep -qx "$want"
  elif command -v sysctl >/dev/null 2>&1; then
    sysctl -n machdep.cpu.features machdep.cpu.leaf7_features 2>/dev/null | tr ' ' '\n' | norm | grep -qx "$want"
  else return 1; fi
}
if [ -z "$ARCH" ]; then
  MACHINE="$(uname -m)"
  case "$(uname -s)-$MACHINE" in
    Darwin-arm64|Darwin-aarch64) ARCH="apple-silicon" ;;
    *-arm64|*-aarch64) ARCH="armv8" ;;
    *-x86_64|*-AMD64|*-amd64)
      # Most specific first. Every name below is a valid Stockfish ARCH.
      if has_flag avx512f && has_flag avx512bw && has_flag avx512vl && has_flag avx512vnni; then ARCH="x86-64-vnni512"
      elif has_flag avx512f && has_flag avx512bw && has_flag avx512vl; then ARCH="x86-64-avx512"
      elif has_flag avxvnni && has_flag avx2 && has_flag bmi2; then ARCH="x86-64-avxvnni"
      elif has_flag bmi2 && has_flag avx2; then ARCH="x86-64-bmi2"
      elif has_flag avx2; then ARCH="x86-64-avx2"
      elif has_flag sse41 && has_flag popcnt; then ARCH="x86-64-sse41-popcnt"
      else ARCH="x86-64"; fi ;;
    *) ARCH="general-64" ;;
  esac
fi
echo "target triple : $TRIPLE"
echo "stockfish arch: $ARCH"

# ---- up-to-date check ------------------------------------------------------
if [ "$FORCE" -eq 0 ] && [ -x "$DEST" ]; then
  NEWER="$(find "$SRC_DIR" \( -name '*.cpp' -o -name '*.h' -o -name Makefile \) -newer "$DEST" -print -quit)"
  if [ -z "$NEWER" ]; then
    echo "up to date: $DEST (use --force to rebuild)"
    exit 0
  fi
fi

# ---- build -----------------------------------------------------------------
JOBS="$( (command -v nproc >/dev/null && nproc) || sysctl -n hw.ncpu 2>/dev/null || echo 4 )"
COMP_ARG=()
case "$TRIPLE" in
  *apple-darwin) COMP_ARG=(COMP=clang) ;;
  *windows-gnu|*windows*) COMP_ARG=(COMP=mingw) ;;
esac

cd "$SRC_DIR"
# Nets are downloaded by the Makefile (needs curl or wget). Idempotent: skips files already present.
echo "fetching NNUE networks (make net)..."
if ! make net; then
  echo "WARNING: 'make net' failed (offline?). The build continues, but the binary will not" >&2
  echo "         find its network at runtime unless the .nnue files are placed next to it." >&2
fi

TARGET=profile-build
[ "$PGO" -eq 1 ] || TARGET=build
echo "building (make -j$JOBS $TARGET ARCH=$ARCH ${COMP_ARG[*]:-})..."
make clean >/dev/null 2>&1 || true
make -j"$JOBS" "$TARGET" ARCH="$ARCH" "${COMP_ARG[@]}"

BUILT="$SRC_DIR/stockfish$EXE"
[ -f "$BUILT" ] || { echo "build did not produce $BUILT" >&2; exit 1; }
make strip ARCH="$ARCH" "${COMP_ARG[@]}" >/dev/null 2>&1 || true

# ---- smoke test the NEW binary before it replaces the installed one -------
smoke() { # smoke <binary>
  local out
  out="$(printf 'uci\nisready\nposition startpos\ngo depth 10\n' | (cat; sleep 3; echo quit) | "$1" 2>&1 || true)"
  echo "$out" | grep -q '^uciok' || { echo "smoke test failed: no uciok" >&2; return 1; }
  echo "$out" | grep -q '^readyok' || { echo "smoke test failed: no readyok" >&2; return 1; }
  echo "$out" | grep -q '^bestmove' || { echo "smoke test failed: no bestmove" >&2; return 1; }
}
smoke "$BUILT" || { make clean >/dev/null 2>&1 || true; exit 1; }

mkdir -p "$OUT_DIR"
# Backup lives outside src-tauri/binaries so Tauri never bundles it.
BACKUP_DIR="$APP_DIR/.engine-backup"
if [ -f "$DEST" ]; then mkdir -p "$BACKUP_DIR" && cp -f "$DEST" "$BACKUP_DIR/$(basename "$DEST").bak"; fi
cp -f "$BUILT" "$DEST.new" && chmod +x "$DEST.new" && mv -f "$DEST.new" "$DEST"
# The NNUE network is embedded in the binary, so no .nnue sidecar files are needed.
make clean >/dev/null 2>&1 || true
echo "installed: $DEST"
smoke "$DEST" && echo "smoke test OK"
