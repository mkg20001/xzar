//! Embedded UI serving
//!
//! Serves the embedded xzar-ui files from the binary.

use rocket::get;
use rocket::http::{ContentType, Status};
use rocket::response::content::RawHtml;

// Include the generated embedded UI code
include!(concat!(env!("OUT_DIR"), "/embedded_ui.rs"));

/// GET /ui
/// Serve the main UI page (index.html)
#[get("/ui")]
pub fn ui_index() -> Result<RawHtml<&'static [u8]>, Status> {
    get_index_html()
        .map(RawHtml)
        .ok_or(Status::NotFound)
}

/// GET /ui/<path..>
/// Serve static UI assets (js, wasm, css, etc.)
#[get("/ui/<path..>")]
pub fn ui_assets(path: std::path::PathBuf) -> Result<(ContentType, &'static [u8]), Status> {
    let path_str = path.to_string_lossy();

    get_embedded_file(&path_str)
        .map(|data| (content_type_for_path(&path_str), data))
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

/// Check if UI is embedded
pub fn is_ui_embedded() -> bool {
    UI_EMBEDDED
}
