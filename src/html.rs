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

/// Turns a raw field/column name into a display label: `_` becomes a
/// space and each word is Title Cased (`estimate_hours` -> `Estimate
/// Hours`). Purely a display-time transform — the field's own stored
/// name is unchanged, so this is safe to call anywhere a name is about
/// to be shown as a `<label>` rather than used as an identifier.
pub fn humanize_label(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
mod tests {
    use super::humanize_label;

    #[test]
    fn splits_underscores_and_title_cases_each_word() {
        assert_eq!(humanize_label("estimate_hours"), "Estimate Hours");
        assert_eq!(humanize_label("id"), "Id");
        assert_eq!(humanize_label("name"), "Name");
        assert_eq!(humanize_label("is_done"), "Is Done");
    }

    #[test]
    fn collapses_repeated_or_edge_underscores() {
        assert_eq!(humanize_label("_leading"), "Leading");
        assert_eq!(humanize_label("trailing_"), "Trailing");
        assert_eq!(humanize_label("double__underscore"), "Double Underscore");
    }
}
