//! Loads a pluggable "chart library" for rendering `Chart` components.
//!
//! Mirrors `theme.rs`'s contract. The built-in backend (`"inline"`,
//! the default) needs no JS at all: bar/line charts are computed into
//! plain inline SVG server-side in `render.rs`, dependency-free. Any
//! other name loads `chart-libs/<name>/chart.js`, served at
//! `/chart-lib.js` and linked from every page; that script reads
//! `<script type="application/json" class="pgapp-chart-data">` blocks
//! this server embeds next to each `<div class="pgapp-chart">`
//! placeholder and renders into it however it likes (canvas, its own
//! SVG, a real charting library, ...) — this server never needs to
//! know how.
//!
//! Selected via `PGAPP_CHART_LIB` (default: "inline").

use std::path::PathBuf;

pub struct ChartLib {
    pub name: String,
    pub js_path: Option<PathBuf>,
}

pub fn load(name: &str) -> anyhow::Result<ChartLib> {
    if name == "inline" {
        return Ok(ChartLib {
            name: name.to_string(),
            js_path: None,
        });
    }

    let js_path = PathBuf::from("chart-libs").join(name).join("chart.js");
    if !js_path.exists() {
        anyhow::bail!(
            "chart library '{name}' not found: expected {} to exist",
            js_path.display()
        );
    }
    Ok(ChartLib {
        name: name.to_string(),
        js_path: Some(js_path),
    })
}
