use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

/// 32-byte (256-bit) URL-safe random token. Used for both per-claim
/// `puid` tokens and per-registration postback secrets (`?k=`).
pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Inject the puid into the admin-supplied Loot Labs short link.
/// The link looks like `https://links.lootlabs.gg/s?ABCDEF`. Loot Labs
/// reads `puid` (a query param) as user passthrough and echoes it back
/// in the postback's `{CLICK_ID}` macro.
pub fn append_puid(short_link: &str, token: &str) -> String {
    let sep = if short_link.contains('?') { '&' } else { '?' };
    format!(
        "{short_link}{sep}puid={}",
        urlencoding::encode(token)
    )
}
