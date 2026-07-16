//! Tiny shared HTML/JS/URL string-escaping helpers, used by both
//! `render.rs` (page chrome) and every component under `item_types/`
//! (form field rendering). Kept dependency-free on purpose — this is
//! the one place that must never get it wrong.

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Escapes a string for embedding inside a single-quoted JS string
/// literal. Callers must still HTML-escape the *result* before splicing
/// it into an HTML attribute (see `item_types::popup`).
pub fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Percent-encodes a query-string value. Used for anything forwarded
/// across pages (row ids, `link:` extra params) so a value containing
/// `&`/`=`/spaces can't corrupt the URL it's embedded in.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
