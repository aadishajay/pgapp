//! Loads a pluggable "icon pack" from `icons/<name>/`.
//!
//! Mirrors `theme.rs`'s contract, extended to a second shape:
//! - the built-in pack (`name == "builtin"`, the default — no
//!   directory needed) renders icon names as inline SVG hard-coded
//!   below, so the framework never depends on a network fetch or a
//!   font file just to show an edit/delete glyph.
//! - any other pack supplies `icons/<name>/pack.json`: `{"stylesheet":
//!   "<url>", "icons": {"edit": {"class": "...", "content": "..."}}}` —
//!   icon names render as `<i class="pgapp-icon <class>">content</i>`,
//!   and `stylesheet` is linked once in the page `<head>`. `content` is
//!   optional and covers both flavors of font-based icon system: Font
//!   Awesome-style packs give every icon its own class and leave
//!   `content` empty, while ligature-style packs (Material Icons) share
//!   one class across all icons and put the icon's name in `content`.
//!   Either way nothing is fetched by this server at render time — it
//!   only ever emits a class name (and maybe a word of text).
//!
//! Selected via `PGAPP_ICONS` (default: "builtin").

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
struct IconSpec {
    class: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PackFile {
    #[serde(default)]
    stylesheet: Option<String>,
    #[serde(default)]
    icons: HashMap<String, IconSpec>,
}

pub struct Icons {
    pub name: String,
    stylesheet: Option<String>,
    icons: HashMap<String, IconSpec>,
}

pub fn load(name: &str) -> Result<Icons> {
    if name == "builtin" {
        return Ok(Icons {
            name: name.to_string(),
            stylesheet: None,
            icons: HashMap::new(),
        });
    }

    let path = PathBuf::from("icons").join(name).join("pack.json");
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!("icon pack '{name}' not found: expected {} to exist", path.display())
    })?;
    let pack: PackFile =
        serde_json::from_str(&raw).with_context(|| format!("invalid pack.json for icon pack '{name}'"))?;

    Ok(Icons {
        name: name.to_string(),
        stylesheet: pack.stylesheet,
        icons: pack.icons,
    })
}

impl Icons {
    /// A `<link>` tag for the pack's stylesheet, if it has one (the
    /// builtin pack doesn't — inline SVG never needs one).
    pub fn stylesheet_tag(&self) -> String {
        match &self.stylesheet {
            Some(href) => format!(r#"<link rel="stylesheet" href="{href}">"#, href = crate::html::escape(href)),
            None => String::new(),
        }
    }

    /// Renders one named icon (e.g. "edit", "delete"). A pack-based
    /// icon whose name isn't declared falls back to the bare name as
    /// both class and content, so a typo shows up as a missing glyph
    /// rather than silently rendering nothing.
    pub fn render(&self, name: &str) -> String {
        if self.stylesheet.is_none() && self.icons.is_empty() {
            builtin_svg(name)
        } else {
            let (class, content) = match self.icons.get(name) {
                Some(spec) => (spec.class.as_str(), spec.content.as_str()),
                None => (name, ""),
            };
            format!(
                r#"<i class="pgapp-icon {}" aria-hidden="true">{}</i>"#,
                crate::html::escape(class),
                crate::html::escape(content),
            )
        }
    }
}

/// The two built-in icons: a pencil (used for anything but "delete")
/// and a trash can (used for "delete"). Both are plain inline SVG paths
/// — no external font, no network fetch.
fn builtin_svg(name: &str) -> String {
    match name {
        "delete" => concat!(
            r#"<svg class="pgapp-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">"#,
            r#"<path fill="currentColor" d="M9 3h6l1 2h4v2H4V5h4l1-2Zm-2 6h10l-1 12H8L7 9Z"/></svg>"#
        )
        .to_string(),
        _ => concat!(
            r#"<svg class="pgapp-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">"#,
            r#"<path fill="currentColor" d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25ZM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83Z"/></svg>"#
        )
        .to_string(),
    }
}
