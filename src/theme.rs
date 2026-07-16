//! Loads a design-system "theme" from `themes/<name>/`.
//!
//! A theme is just a directory containing:
//! - `theme.css` (required) — styles pgapp's fixed `.pgapp-*` class
//!   contract (see README) using whatever design-system conventions it
//!   wants (CSS variables, utility classes, etc).
//! - `theme.json` (optional) — `{ "label": ..., "description": ... }`,
//!   metadata for humans/tooling. Rendering doesn't depend on it.
//!
//! pgapp ships `shadcn` (default) and `plain`; any directory following
//! the same contract works the same way.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ThemeMeta {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
}

pub struct Theme {
    pub name: String,
    pub css_path: PathBuf,
    pub meta: ThemeMeta,
}

pub fn load(name: &str) -> Result<Theme> {
    let dir = PathBuf::from("themes").join(name);
    let css_path = dir.join("theme.css");
    if !css_path.exists() {
        anyhow::bail!(
            "theme '{name}' not found: expected {} to exist (see the \
             'Theme contract' section of the README)",
            css_path.display()
        );
    }

    let meta = match std::fs::read_to_string(dir.join("theme.json")) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("invalid theme.json for theme '{name}'"))?,
        Err(_) => ThemeMeta::default(),
    };

    Ok(Theme {
        name: name.to_string(),
        css_path,
        meta,
    })
}
