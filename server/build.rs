//! Build script for embedding xzar-ui assets into the server binary
//!
//! This script scans the UI build output directory and generates Rust code
//! that embeds all files using include_bytes!/include_str!.
//!
//! Prerequisites:
//! - Build the UI first: `cd ui && dx build --release`
//! - The build output should be in `target/dx/xzar-ui/release/web/public/`

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_ui.rs");
    let mut f = File::create(&dest_path).unwrap();

    // Try release build first, then debug
    let ui_dir = find_ui_dir();

    if let Some(ui_path) = ui_dir {
        println!("cargo:rerun-if-changed={}", ui_path.display());
        generate_embedded_files(&mut f, &ui_path);
    } else {
        // No UI build found - generate empty placeholder
        generate_empty_placeholder(&mut f);
    }
}

fn find_ui_dir() -> Option<std::path::PathBuf> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = Path::new(&manifest_dir).parent().unwrap();

    // Try release build first
    let release_path = workspace_root.join("target/dx/xzar-ui/release/web/public");
    if release_path.exists() && release_path.join("index.html").exists() {
        return Some(release_path);
    }

    // Fall back to debug build
    let debug_path = workspace_root.join("target/dx/xzar-ui/debug/web/public");
    if debug_path.exists() && debug_path.join("index.html").exists() {
        return Some(debug_path);
    }

    None
}

fn generate_embedded_files(f: &mut File, ui_path: &Path) {
    let mut files = Vec::new();
    collect_files(ui_path, ui_path, &mut files);

    // Generate the static file array
    writeln!(f, "/// Embedded UI files").unwrap();
    writeln!(f, "pub static EMBEDDED_FILES: &[(&str, &[u8])] = &[").unwrap();

    for (web_path, fs_path) in &files {
        writeln!(
            f,
            "    (\"{}\", include_bytes!(\"{}\")),",
            web_path,
            fs_path.display()
        )
        .unwrap();
    }

    writeln!(f, "];").unwrap();

    // Generate helper function
    writeln!(f).unwrap();
    writeln!(f, "/// Get an embedded file by path").unwrap();
    writeln!(f, "pub fn get_embedded_file(path: &str) -> Option<&'static [u8]> {{").unwrap();
    writeln!(f, "    let path = path.trim_start_matches('/');").unwrap();
    writeln!(f, "    for (name, data) in EMBEDDED_FILES {{").unwrap();
    writeln!(f, "        if *name == path {{").unwrap();
    writeln!(f, "            return Some(data);").unwrap();
    writeln!(f, "        }}").unwrap();
    writeln!(f, "    }}").unwrap();
    writeln!(f, "    None").unwrap();
    writeln!(f, "}}").unwrap();

    // Generate index.html getter
    writeln!(f).unwrap();
    writeln!(f, "/// Get the index.html content").unwrap();
    writeln!(f, "pub fn get_index_html() -> Option<&'static [u8]> {{").unwrap();
    writeln!(f, "    get_embedded_file(\"index.html\")").unwrap();
    writeln!(f, "}}").unwrap();

    // Generate flag for UI availability
    writeln!(f).unwrap();
    writeln!(f, "/// Whether the UI is embedded").unwrap();
    writeln!(f, "pub const UI_EMBEDDED: bool = true;").unwrap();

    eprintln!(
        "cargo:warning=Embedded {} UI files from {}",
        files.len(),
        ui_path.display()
    );
}

fn generate_empty_placeholder(f: &mut File) {
    writeln!(f, "/// Embedded UI files (empty - UI not built)").unwrap();
    writeln!(f, "pub static EMBEDDED_FILES: &[(&str, &[u8])] = &[];").unwrap();
    writeln!(f).unwrap();
    writeln!(f, "/// Get an embedded file by path").unwrap();
    writeln!(f, "pub fn get_embedded_file(_path: &str) -> Option<&'static [u8]> {{").unwrap();
    writeln!(f, "    None").unwrap();
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();
    writeln!(f, "/// Get the index.html content").unwrap();
    writeln!(f, "pub fn get_index_html() -> Option<&'static [u8]> {{").unwrap();
    writeln!(f, "    None").unwrap();
    writeln!(f, "}}").unwrap();
    writeln!(f).unwrap();
    writeln!(f, "/// Whether the UI is embedded").unwrap();
    writeln!(f, "pub const UI_EMBEDDED: bool = false;").unwrap();

    eprintln!("cargo:warning=UI not embedded - build UI first with: cd ui && dx build --release");
}

fn collect_files(base: &Path, current: &Path, files: &mut Vec<(String, std::path::PathBuf)>) {
    if let Ok(entries) = fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(base, &path, files);
            } else if path.is_file() {
                let relative = path.strip_prefix(base).unwrap();
                let web_path = relative.to_string_lossy().replace('\\', "/");
                files.push((web_path, path));
            }
        }
    }
}
