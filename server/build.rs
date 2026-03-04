//! Build script for UI embedding path detection
//!
//! When the `embed-ui` feature is enabled, this script determines the
//! correct path to the UI build output (release or debug).

use std::env;
use std::path::Path;

fn main() {
    // Only needed when embed-ui feature is enabled
    if env::var("CARGO_FEATURE_EMBED_UI").is_ok() {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let workspace_root = Path::new(&manifest_dir).parent().unwrap();

        // Try release build first, then debug
        let release_path = workspace_root.join("target/dx/xzar-ui/release/web/public");
        let debug_path = workspace_root.join("target/dx/xzar-ui/debug/web/public");

        let ui_path = if release_path.join("index.html").exists() {
            release_path
        } else if debug_path.join("index.html").exists() {
            debug_path
        } else {
            panic!(
                "UI not found! Build the UI first:\n  cd ui && dx build --release\n\
                 Expected at: {} or {}",
                release_path.display(),
                debug_path.display()
            );
        };

        println!("cargo:rustc-env=UI_EMBED_PATH={}", ui_path.display());
        println!("cargo:rerun-if-changed={}", ui_path.display());
    }
}
