//! Locating the bundled Stockfish sidecar.
//!
//! * Bundles: Tauri places the `externalBin` next to the app executable with
//!   the target triple stripped (`stockfish` / `stockfish.exe`).
//! * Dev builds: the engine build script writes
//!   `src-tauri/binaries/stockfish-<triple>[.exe]`, which is read in place.

use std::path::{Path, PathBuf};

/// Ordered candidate locations for the engine binary.
pub fn candidate_paths(
    exe_dir: Option<&Path>,
    manifest_dir: Option<&Path>,
    triple: &str,
    windows: bool,
) -> Vec<PathBuf> {
    let ext = if windows { ".exe" } else { "" };
    let mut out = Vec::new();
    if let Some(dir) = exe_dir {
        out.push(dir.join(format!("stockfish{ext}")));
        out.push(dir.join(format!("stockfish-{triple}{ext}")));
        // macOS bundles can also keep helpers in Contents/Resources.
        if let Some(parent) = dir.parent() {
            out.push(parent.join("Resources").join(format!("stockfish{ext}")));
        }
    }
    if let Some(dir) = manifest_dir {
        out.push(
            dir.join("binaries")
                .join(format!("stockfish-{triple}{ext}")),
        );
    }
    out
}

/// Resolve the engine binary or explain how to get one.
pub fn resolve() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    // The source-tree path is only meaningful (and only embedded) in dev builds.
    let manifest_dir = if cfg!(debug_assertions) {
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    } else {
        None
    };
    let triple = env!("FISHY_TARGET_TRIPLE");
    let candidates = candidate_paths(
        exe_dir.as_deref(),
        manifest_dir.as_deref(),
        triple,
        cfg!(windows),
    );
    if let Some(found) = candidates.iter().find(|p| p.is_file()) {
        return Ok(found.clone());
    }
    let hint = if cfg!(debug_assertions) {
        "Build it first with scripts/build-stockfish.sh (writes src-tauri/binaries/stockfish-<target-triple>)."
    } else {
        "The application bundle appears to be incomplete; please reinstall Fishy."
    };
    Err(format!(
        "The Stockfish engine was not found (looked in {}). {hint}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_the_bundle_then_the_dev_binary() {
        let c = candidate_paths(
            Some(Path::new("/app/Contents/MacOS")),
            Some(Path::new("/src/src-tauri")),
            "aarch64-apple-darwin",
            false,
        );
        assert_eq!(c[0], Path::new("/app/Contents/MacOS/stockfish"));
        assert_eq!(
            c.last().unwrap(),
            Path::new("/src/src-tauri/binaries/stockfish-aarch64-apple-darwin")
        );
    }

    #[test]
    fn windows_adds_exe_suffix() {
        let c = candidate_paths(
            Some(Path::new("C:/Fishy")),
            Some(Path::new("C:/src/src-tauri")),
            "x86_64-pc-windows-msvc",
            true,
        );
        assert!(c.iter().all(|p| p.to_string_lossy().ends_with(".exe")));
        assert!(c.iter().any(|p| {
            p.to_string_lossy()
                .ends_with("binaries/stockfish-x86_64-pc-windows-msvc.exe")
        }));
    }

    #[test]
    fn no_dirs_no_candidates() {
        assert!(candidate_paths(None, None, "t", false).is_empty());
    }
}
