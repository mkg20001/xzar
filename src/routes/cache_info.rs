use rocket::get;
use rocket::http::ContentType;

/// GET /nix-cache-info
/// Returns cache metadata in Nix's key-value format
#[get("/nix-cache-info")]
pub fn nix_cache_info() -> (ContentType, &'static str) {
    (
        ContentType::Plain,
        "StoreDir: /nix/store\nWantMassQuery: 1\nPriority: 30\n",
    )
}
