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

/// One entry in `list_themes()` — enough for a picker: the directory
/// name (what `theme: <name>` in markup refers to) and a human label
/// (from `theme.json`, falling back to the name itself).
pub struct ThemeInfo {
    pub name: String,
    pub label: String,
}

/// Every theme actually on disk under `themes/` — the source of truth
/// for every theme picker (the App Builder's own, `pgapp new`'s CLI
/// prompt), replacing what used to be a hardcoded name list in each of
/// those places. A hand-dropped or cloned theme directory shows up
/// here (and so everywhere) the moment it exists, with no separate
/// registration step — a subdirectory missing `theme.css` (not a real
/// theme, or mid-clone) is silently skipped rather than erroring the
/// whole list.
pub fn list_themes() -> Vec<ThemeInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("themes") else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join("theme.css").exists() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let label = load(name).ok().map(|t| t.meta.label).filter(|l| !l.is_empty()).unwrap_or_else(|| name.to_string());
        out.push(ThemeInfo { name: name.to_string(), label });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
