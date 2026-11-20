fn main() {
    // Expose the target triple so the app can locate `binaries/stockfish-<triple>` in dev builds.
    println!(
        "cargo:rustc-env=FISHY_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    println!("cargo:rerun-if-changed=binaries");
    tauri_build::build()
}
