//! Embedded UI serving
//!
//! Serves the embedded xzar-ui files from the binary.
//!
//! Enable the `embed-ui` feature to embed the UI:
//! ```
//! cargo build --features embed-ui
//! ```
//!
//! Prerequisites:
//! - Build the UI first: `cd ui && dx build --release`

use rocket::get;
use rocket::http::{ContentType, Status};
use rocket::response::content::RawHtml;
use std::borrow::Cow;

// ============ Embedded UI (with rust-embed) ============

#[cfg(feature = "embed-ui")]
mod embedded {
    use rust_embed::RustEmbed;

    #[derive(RustEmbed)]
    #[folder = "$UI_EMBED_PATH"]
    pub struct Asset;
}

#[cfg(feature = "embed-ui")]
fn get_embedded_file(path: &str) -> Option<Cow<'static, [u8]>> {
    let path = path.trim_start_matches('/');
    embedded::Asset::get(path).map(|f| f.data)
}

#[cfg(feature = "embed-ui")]
fn get_index_html() -> Option<Cow<'static, [u8]>> {
    get_embedded_file("index.html")
}

#[cfg(feature = "embed-ui")]
pub fn is_ui_embedded() -> bool {
    true
}

// ============ No UI embedded (feature disabled) ============

#[cfg(not(feature = "embed-ui"))]
fn get_embedded_file(_path: &str) -> Option<Cow<'static, [u8]>> {
    None
}

#[cfg(not(feature = "embed-ui"))]
fn get_index_html() -> Option<Cow<'static, [u8]>> {
    None
}

#[cfg(not(feature = "embed-ui"))]
pub fn is_ui_embedded() -> bool {
    false
}

// ============ Routes ============

/// GET /ui
/// Serve the main UI page (index.html)
#[get("/ui")]
pub fn ui_index() -> Result<RawHtml<Vec<u8>>, Status> {
    get_index_html()
        .map(|data| RawHtml(data.into_owned()))
        .ok_or(Status::NotFound)
}

/// GET /ui/<path..>
/// Serve static UI assets (js, wasm, css, etc.)
#[get("/ui/<path..>")]
pub fn ui_assets(path: std::path::PathBuf) -> Result<(ContentType, Vec<u8>), Status> {
    let path_str = path.to_string_lossy();

    get_embedded_file(&path_str)
        .map(|data| (content_type_for_path(&path_str), data.into_owned()))
        .ok_or(Status::NotFound)
}

/// Determine content type from file extension
fn content_type_for_path(path: &str) -> ContentType {
    if path.ends_with(".html") {
        ContentType::HTML
    } else if path.ends_with(".js") {
        ContentType::JavaScript
    } else if path.ends_with(".wasm") {
        ContentType::new("application", "wasm")
    } else if path.ends_with(".css") {
        ContentType::CSS
    } else if path.ends_with(".json") {
        ContentType::JSON
    } else if path.ends_with(".png") {
        ContentType::PNG
    } else if path.ends_with(".svg") {
        ContentType::SVG
    } else if path.ends_with(".ico") {
        ContentType::Icon
    } else {
        ContentType::Binary
    }
}
