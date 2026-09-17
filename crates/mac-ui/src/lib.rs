#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod attributed;
#[cfg(target_os = "macos")]
mod images;
#[cfg(target_os = "macos")]
pub mod page_canvas;
#[cfg(target_os = "macos")]
mod tables;
#[cfg(target_os = "macos")]
mod toolbar;

#[cfg(target_os = "macos")]
pub use app::run;
#[cfg(target_os = "macos")]
pub use attributed::{attributed_to_blocks, bridged_to_attributed};

/// Non-macOS placeholder so the workspace still type-checks in CI elsewhere.
#[cfg(not(target_os = "macos"))]
pub fn run() {
    eprintln!("word-rs UI requires macOS (AppKit / TextKit).");
    std::process::exit(1);
}
