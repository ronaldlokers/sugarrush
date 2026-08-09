//! Bake the build target and compiler version into the binary.
//!
//! `sugarrush about` is what the issue template asks a reporter to paste, and
//! "which build is this" is not answerable from the version alone — a crates.io
//! build, a Homebrew bottle and an AUR package of the same version behave
//! differently when the problem is an audio player or a notification daemon.
fn main() {
    println!(
        "cargo:rustc-env=SUGARRUSH_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
    let rustc =
        std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SUGARRUSH_RUSTC={rustc}");
    println!("cargo:rerun-if-changed=build.rs");
}
