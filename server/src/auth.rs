use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use sha2::{Digest, Sha512};

pub struct AuthenticatedUser;

pub struct TokenStore {
    hashes: Vec<String>,
}

impl TokenStore {
    pub fn new(hashes: Vec<String>) -> Self {
        Self { hashes }
    }

    pub fn validate(&self, token: &str) -> bool {
        let mut hasher = Sha512::new();
        hasher.update(token.as_bytes());
        let result = hasher.finalize();
        let token_hash = hex_encode(&result);

        // Constant-time comparison to prevent timing attacks
        self.hashes
            .iter()
            .any(|h| constant_time_eq(h.as_bytes(), token_hash.as_bytes()))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX_CHARS[(byte >> 4) as usize] as char);
        hex.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    hex
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthenticatedUser {
    type Error = ();

    async fn from_request(request: &'r rocket::Request<'_>) -> Outcome<Self, Self::Error> {
        let token_store = request.rocket().state::<TokenStore>();

        let token = request
            .headers()
            .get_one("Authorization")
            .and_then(|h| h.strip_prefix("Bearer "));

        match (token_store, token) {
            (Some(store), Some(token)) if store.validate(token) => {
                Outcome::Success(AuthenticatedUser)
            }
            (Some(_), Some(_)) => Outcome::Error((Status::Unauthorized, ())),
            (Some(_), None) => Outcome::Error((Status::Unauthorized, ())),
            (None, _) => {
                // No tokens configured, allow all requests (development mode)
                tracing::warn!("No tokens configured, allowing unauthenticated access");
                Outcome::Success(AuthenticatedUser)
            }
        }
    }
}
