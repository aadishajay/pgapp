//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! a page's component list comes from `RuntimePage`/`RuntimeApp`, not
//! from a per-app template.
//!
//! Markup here only ever uses the fixed `.pgapp-*` class names — the
//! "Theme contract" documented in the README. All actual look-and-feel
//! comes from `/{app}/theme.css` (the active theme, see src/theme.rs) plus
//! any app-level override in assets/app.css. A form field's actual input is
//! never built here — `input_for_field` just hands off to whatever
//! component is registered for that field's item type (see
//! `src/item_types.rs`), so adding a new one never touches this file.
//! Component *data fetching* (rows, pagination, resolved choices) is
//! `server.rs`'s job; this module only ever formats what it's handed.
//!
//! Every route in this app is scoped under `/{app}` (the app's URL
//! slug — see `src/server.rs`), so every function here that builds a
//! path takes `app` and prefixes it on every href/action/src it emits.

use crate::chart_lib::ChartLib;
use crate::html::{escape, url_encode};
use crate::icons::Icons;
use crate::item_types::{self, RenderArgs};
use crate::meta::{Chrome, LinkColumn, NavNode, RegionRows, RuntimeComponent, RuntimeEntity};
use crate::model::{FieldItem, HtmlAttrs};
use std::collections::{BTreeMap, HashMap};

/// Extra `<link>`/`<script>` tags for user-supplied assets, if present —
/// the app-level override layer, on top of the active theme.
pub fn asset_tags(app: &str) -> String {
    let mut tags = String::new();
    if std::path::Path::new("assets/app.css").exists() {
        tags.push_str(&format!("<link rel=\"stylesheet\" href=\"/{app}/assets/app.css\">\n"));
    }
    if std::path::Path::new("assets/app.js").exists() {
        tags.push_str(&format!("<script src=\"/{app}/assets/app.js\" defer></script>\n"));
    }
    tags
}

/// Renders the app's (possibly multi-level) nav bar as nested `<ul>`s;
/// submenus are shown on hover/focus via the theme's CSS, not JS.
fn nav_html(app: &str, nodes: &[NavNode]) -> String {
    if nodes.is_empty() {
        return String::new();
    }
    let mut html = String::from(r#"<ul class="pgapp-navbar">"#);
    for node in nodes {
        html.push_str(&nav_node_html(app, node));
    }
    html.push_str("</ul>");
    html
}

fn nav_node_html(app: &str, node: &NavNode) -> String {
    let has_children = !node.children.is_empty();
    let mut html = String::from(r#"<li class="pgapp-navbar-item">"#);
    html.push_str(r#"<span class="pgapp-navbar-row">"#);
    match &node.target_page {
        Some(target) => html.push_str(&format!(
            r#"<a class="pgapp-link" href="/{app}/{target}">{label}</a>"#,
            app = escape(app),
            target = escape(target),
            label = escape(&node.label),
        )),
        None => html.push_str(&format!(
            r#"<span class="pgapp-navbar-label">{}</span>"#,
            escape(&node.label)
        )),
    }
    if has_children {
        // A dedicated toggle, not the label/link itself: on touch
        // devices there's no hover, so runtime.js binds a click on this
        // button to show/hide the submenu without stealing clicks from
        // a parent that's also a real link.
        html.push_str(
            r#"<button type="button" class="pgapp-navbar-toggle" aria-expanded="false" aria-label="Show submenu">&#9662;</button>"#,
        );
    }
    html.push_str("</span>");
    if has_children {
        html.push_str(r#"<ul class="pgapp-navbar-submenu">"#);
        for child in &node.children {
            html.push_str(&nav_node_html(app, child));
        }
        html.push_str("</ul>");
    }
    html.push_str("</li>");
    html
}

/// `class="pgapp-x extra"` — merges a component's own required
/// wrapper class(es) with any user override from a trailing
/// `attrs (class: "...")` suffix in the markup.
fn merged_class(base: &str, html: &HtmlAttrs) -> String {
    match &html.class {
        Some(extra) => format!("{base} {extra}"),
        None => base.to_string(),
    }
}

/// `id="..." data-foo="bar"` — the id plus any extra (non-class/id)
/// attributes from `attrs (...)`, ready to splice right after a
/// wrapper tag's class attribute.
fn extra_attrs(html: &HtmlAttrs) -> String {
    let mut out = String::new();
    if let Some(id) = &html.id {
        out.push_str(&format!(r#" id="{}""#, escape(id)));
    }
    for (k, v) in &html.attrs {
        out.push_str(&format!(r#" {}="{}""#, escape(k), escape(v)));
    }
    out
}

pub fn text_html(text: &str, html: &HtmlAttrs) -> String {
    format!(
        r#"<p class="{class}"{extra}>{text}</p>"#,
        class = merged_class("pgapp-text", html),
        extra = extra_attrs(html),
        text = escape(text),
    )
}

pub fn link_html(app: &str, label: &str, target_page: &str, html: &HtmlAttrs) -> String {
    format!(
        r#"<p><a class="{class}" href="/{app}/{target}"{extra}>{label}</a></p>"#,
        class = merged_class("pgapp-link", html),
        extra = extra_attrs(html),
        app = escape(app),
        target = escape(target_page),
        label = escape(label),
    )
}

/// Renders one `Region` component: a named query's (already-resolved)
/// rows as a plain table. `columns` narrows/orders which of the row's
/// keys are shown; empty shows every column, alphabetically (the only
/// order available when it's not spelled out, since a query's result
/// columns have no inherent display order).
pub fn region_html(label: &str, query: &str, regions: &RegionRows, columns: &[String], html: &HtmlAttrs) -> String {
    // data-pgapp-region lets a dynamic action's `refresh` op find and
    // replace this container with a freshly fetched fragment.
    let mut out = format!(
        r#"<div class="{class}" data-pgapp-region="{query}"{extra}><h3 class="pgapp-region-title">{label}</h3>"#,
        class = merged_class("pgapp-region", html),
        extra = extra_attrs(html),
        query = escape(query),
        label = escape(label),
    );
    match regions.get(query).filter(|rows| !rows.is_empty()) {
        Some(rows) => {
            let cols: Vec<&String> = if columns.is_empty() {
                let mut cols: Vec<&String> = rows[0].keys().collect();
                cols.sort();
                cols
            } else {
                columns.iter().collect()
            };

            out.push_str(r#"<div class="pgapp-table-wrap"><table class="pgapp-table"><thead><tr>"#);
            for c in &cols {
                out.push_str(&format!("<th>{}</th>", escape(c)));
            }
            out.push_str("</tr></thead><tbody>");
            for row in rows {
                out.push_str("<tr>");
                for c in &cols {
                    let val = row.get(*c).and_then(|v| v.as_deref()).unwrap_or("");
                    out.push_str(&format!("<td>{}</td>", escape(val)));
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table></div>");
        }
        None => out.push_str(r#"<p class="pgapp-text">No results.</p>"#),
    }
    out.push_str("</div>");
    out
}

/// Renders a header/footer chrome list — restricted at sync time to
/// Text/Link/Region, so those are the only variants handled here.
fn chrome_items_html(app: &str, items: &[RuntimeComponent], regions: &RegionRows) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut html = String::from(r#"<div class="pgapp-items">"#);
    for item in items {
        match item {
            RuntimeComponent::Text { text, html: attrs } => html.push_str(&text_html(text, attrs)),
            RuntimeComponent::Link { label, target_page, html: attrs } => {
                html.push_str(&link_html(app, label, target_page, attrs))
            }
            RuntimeComponent::Region { label, query, columns, html: attrs } => {
                html.push_str(&region_html(label, query, regions, columns, attrs))
            }
            _ => {}
        }
    }
    html.push_str("</div>");
    html
}

/// Fixed-order categorical palette for per-category marks (bar/pie/donut
/// slices) — the validated 8-hue reference order from the dataviz
/// skill's `references/palette.md` (worst adjacent CVD ΔE 9.1 light /
/// 8.4 dark, worst adjacent normal-vision ΔE 19.6 / 19.3). Each theme
/// may override a slot by defining the matching `--chart-N` custom
/// property (see themes/*/theme.css); the hex here is only the
/// `var(..., fallback)` fallback for a theme that doesn't, so a
/// third-party theme with no chart palette still renders distinguishable
/// categories instead of one flat fill.
const CHART_PALETTE: [&str; 8] =
    ["#2a78d6", "#008300", "#e87ba4", "#eda100", "#1baf7a", "#eb6834", "#4a3aa7", "#e34948"];

/// The fill for the `i`th category in a multi-category chart (bar/pie/donut) —
/// cycles through `CHART_PALETTE` past 8 categories rather than repeating a
/// single accent color for every one of them.
fn chart_slice_fill(i: usize) -> String {
    format!("var(--chart-{}, {})", (i % CHART_PALETTE.len()) + 1, CHART_PALETTE[i % CHART_PALETTE.len()])
}

/// `bar`/`line`/`area`/`scatter` all plot `rows` against a shared x
/// (category) axis and y (value) baseline — this draws that shared
/// axis/grid and hands back the plot area's geometry for the per-type
/// mark drawing in `inline_svg_chart`.
fn cartesian_chart_svg(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>]) -> String {
    let (width, height, pad) = (480.0_f64, 220.0_f64, 30.0_f64);
    let values: Vec<f64> = rows
        .iter()
        .map(|r| r.get(y).and_then(|v| v.as_deref()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0))
        .collect();
    let labels: Vec<String> = rows.iter().map(|r| r.get(x).and_then(|v| v.as_deref()).unwrap_or("").to_string()).collect();
    // The raw (unparsed) value text for tooltips — keeps whatever
    // formatting the query returned instead of `values`' re-serialized
    // f64 (which drops trailing zeros, thousands separators, etc.).
    let raw_values: Vec<String> = rows.iter().map(|r| r.get(y).and_then(|v| v.as_deref()).unwrap_or("").to_string()).collect();
    let max = values.iter().cloned().fold(1.0_f64, f64::max).max(1.0);
    let n = (values.len().max(1)) as f64;
    let bar_w = (width - pad * 2.0) / n;
    let baseline = height - pad;
    let point = |i: usize, v: f64| {
        let px = pad + bar_w * (i as f64 + 0.5);
        let py = baseline - (v / max) * (height - pad * 2.0);
        (px, py)
    };

    let mut svg = format!(
        r#"<svg class="pgapp-chart-svg" viewBox="0 0 {width} {height}" role="img" aria-label="{}">"#,
        escape(title)
    );
    svg.push_str(&format!(
        r#"<line x1="{pad}" y1="{baseline}" x2="{}" y2="{baseline}" stroke="currentColor" stroke-opacity="0.3"/>"#,
        width - pad
    ));

    match chart_type {
        "line" | "area" | "scatter" => {
            let points: Vec<(f64, f64)> = values.iter().enumerate().map(|(i, v)| point(i, *v)).collect();
            if chart_type == "area" {
                let mut path = format!("M {:.1},{baseline:.1} ", points.first().map(|p| p.0).unwrap_or(pad));
                for (px, py) in &points {
                    path.push_str(&format!("L {px:.1},{py:.1} "));
                }
                path.push_str(&format!("L {:.1},{baseline:.1} Z", points.last().map(|p| p.0).unwrap_or(width - pad)));
                svg.push_str(&format!(
                    r#"<path d="{path}" fill="currentColor" fill-opacity="0.25" stroke="currentColor" stroke-width="2"/>"#
                ));
            } else if chart_type == "line" {
                let poly = points.iter().map(|(px, py)| format!("{px:.1},{py:.1}")).collect::<Vec<_>>().join(" ");
                svg.push_str(&format!(r#"<polyline points="{poly}" fill="none" stroke="currentColor" stroke-width="2"/>"#));
            }
            for (i, (px, py)) in points.iter().enumerate() {
                svg.push_str(&format!(
                    r#"<circle cx="{px:.1}" cy="{py:.1}" r="3" fill="currentColor"><title>{}: {}</title></circle>"#,
                    escape(&labels[i]),
                    escape(&raw_values[i])
                ));
            }
        }
        _ => {
            // "bar" (also the fallback — markup.rs's CHART_TYPES check
            // already rejects anything else at sync time). Each bar gets
            // its own categorical color (see `chart_slice_fill`) since a
            // bar chart's bars are as much distinct categories as a pie
            // chart's slices are — a single flat fill leaves nothing but
            // height to tell them apart.
            for (i, v) in values.iter().enumerate() {
                let bar_h = (v / max) * (height - pad * 2.0);
                let bx = pad + bar_w * (i as f64) + 2.0;
                let by = baseline - bar_h;
                svg.push_str(&format!(
                    r#"<rect x="{bx:.1}" y="{by:.1}" width="{:.1}" height="{bar_h:.1}" fill="{}"><title>{}: {}</title></rect>"#,
                    (bar_w - 4.0).max(1.0),
                    chart_slice_fill(i),
                    escape(&labels[i]),
                    escape(&raw_values[i])
                ));
            }
        }
    }

    for (i, label) in labels.iter().enumerate() {
        let px = pad + bar_w * (i as f64 + 0.5);
        // fill="currentColor": SVG text ignores the page's CSS `color`
        // and defaults to black unless told otherwise, unlike every
        // other mark here — without this it's unreadable in dark mode
        // (black on a dark card).
        svg.push_str(&format!(
            r#"<text x="{px:.1}" y="{:.1}" font-size="9" text-anchor="middle" fill="currentColor">{}</text>"#,
            baseline + 12.0,
            escape(label)
        ));
    }
    svg.push_str("</svg>");
    svg
}

/// `pie`/`donut`: each row becomes one slice, swept clockwise from 12
/// o'clock, sized by its share of the total; `donut` punches a hole in
/// the middle. A simple side legend lists label + share since slice
/// labels don't fit reliably inside thin wedges.
fn radial_chart_svg(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>]) -> String {
    let (width, height) = (480.0_f64, 220.0_f64);
    let (cx, cy, r) = (150.0_f64, height / 2.0, height / 2.0 - 20.0);
    let values: Vec<f64> = rows
        .iter()
        .map(|row| row.get(y).and_then(|v| v.as_deref()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0).max(0.0))
        .collect();
    let labels: Vec<String> = rows.iter().map(|row| row.get(x).and_then(|v| v.as_deref()).unwrap_or("").to_string()).collect();
    let raw_values: Vec<String> = rows.iter().map(|row| row.get(y).and_then(|v| v.as_deref()).unwrap_or("").to_string()).collect();
    let total = values.iter().sum::<f64>().max(1e-9);

    let mut svg = format!(
        r#"<svg class="pgapp-chart-svg" viewBox="0 0 {width} {height}" role="img" aria-label="{}">"#,
        escape(title)
    );

    let mut angle = -std::f64::consts::FRAC_PI_2;
    for (i, v) in values.iter().enumerate() {
        let frac = v / total;
        let end_angle = angle + frac * std::f64::consts::TAU;
        let fill = chart_slice_fill(i);
        let pct = frac * 100.0;
        let title = format!("<title>{}: {} ({pct:.0}%)</title>", escape(&labels[i]), escape(&raw_values[i]));
        if frac >= 0.9999 {
            svg.push_str(&format!(r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" fill="{fill}" fill-opacity="0.85">{title}</circle>"#));
        } else if frac > 0.0 {
            let (x0, y0) = (cx + r * angle.cos(), cy + r * angle.sin());
            let (x1, y1) = (cx + r * end_angle.cos(), cy + r * end_angle.sin());
            let large_arc = if end_angle - angle > std::f64::consts::PI { 1 } else { 0 };
            // No theme-shared "card background" CSS variable exists to
            // punch a true hole through, so slices are separated with a
            // plain white seam — a close enough approximation on every
            // built-in theme's light chart background.
            svg.push_str(&format!(
                r#"<path d="M {cx:.1},{cy:.1} L {x0:.1},{y0:.1} A {r:.1},{r:.1} 0 {large_arc} 1 {x1:.1},{y1:.1} Z" fill="{fill}" fill-opacity="0.85" stroke="white" stroke-width="1.5">{title}</path>"#
            ));
        }
        angle = end_angle;
    }
    if chart_type == "donut" {
        // A class, not an inline fill: needs to match the chart card's
        // own background (theme- and light/dark-mode-dependent, so
        // only the theme's own CSS knows the right color — see
        // `.pgapp-chart-donut-hole` in each theme.css).
        let hole_r = r * 0.55;
        svg.push_str(&format!(r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{hole_r:.1}" class="pgapp-chart-donut-hole"/>"#));
    }

    let legend_x = cx + r + 30.0;
    for (i, (label, v)) in labels.iter().zip(values.iter()).enumerate() {
        let ly = 20.0 + (i as f64) * 16.0;
        if ly > height - 10.0 {
            break;
        }
        let pct = (v / total) * 100.0;
        // The swatch uses the same per-slice color as the wedge itself
        // (chart_slice_fill(i)) — it used to be a flat currentColor, which
        // both matched the (also flat) wedge fill and told you nothing
        // about which color was which. fill="currentColor" on the <text>
        // itself: see cartesian_chart_svg's axis labels for why this
        // can't be left to inherit.
        svg.push_str(&format!(
            r#"<rect x="{legend_x:.1}" y="{:.1}" width="9" height="9" fill="{}" fill-opacity="0.85"/><text x="{:.1}" y="{:.1}" font-size="9" fill="currentColor">{} ({pct:.0}%)</text>"#,
            ly - 8.0,
            chart_slice_fill(i),
            legend_x + 13.0,
            ly,
            escape(label)
        ));
    }
    svg.push_str("</svg>");
    svg
}

/// A dependency-free chart rendered straight to inline SVG — the
/// built-in `PGAPP_CHART_LIB=inline` backend (see `src/chart_lib.rs`).
/// No JS, no network fetch. Supports every type in `model::CHART_TYPES`.
fn inline_svg_chart(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>], html: &HtmlAttrs) -> String {
    let svg = match chart_type {
        "pie" | "donut" => radial_chart_svg(title, chart_type, x, y, rows),
        _ => cartesian_chart_svg(title, chart_type, x, y, rows),
    };
    format!(
        r#"<div class="{class}"{extra}><h3 class="pgapp-region-title">{title}</h3>{svg}</div>"#,
        class = merged_class("pgapp-chart", html),
        extra = extra_attrs(html),
        title = escape(title),
    )
}

/// A JSON-in-`<script>` placeholder for a pluggable chart library (see
/// `src/chart_lib.rs`): the library's JS (served at `/{app}/chart-lib.js`)
/// reads this data and renders into the surrounding `.pgapp-chart` div
/// however it likes.
fn pluggable_chart_placeholder(
    title: &str,
    chart_type: &str,
    x: &str,
    y: &str,
    rows: &[BTreeMap<String, Option<String>>],
    html: &HtmlAttrs,
) -> String {
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::Value::Object(
                r.iter()
                    .map(|(k, v)| (k.clone(), v.clone().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null)))
                    .collect(),
            )
        })
        .collect();
    let data = serde_json::json!({ "rows": json_rows, "x": x, "y": y, "type": chart_type });
    // `</` can't appear literally inside a <script> body without ending
    // it early, regardless of the script's declared type.
    let safe_json = data.to_string().replace("</", "<\\/");
    format!(
        r#"<div class="{class}"{extra}><h3 class="pgapp-region-title">{title}</h3><script type="application/json" class="pgapp-chart-data">{safe_json}</script></div>"#,
        class = merged_class("pgapp-chart", html),
        extra = extra_attrs(html),
        title = escape(title),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn chart_html(
    title: &str,
    chart_type: &str,
    x: &str,
    y: &str,
    rows: &[BTreeMap<String, Option<String>>],
    chart_lib: &ChartLib,
    html: &HtmlAttrs,
) -> String {
    match &chart_lib.js_path {
        None => inline_svg_chart(title, chart_type, x, y, rows, html),
        Some(_) => pluggable_chart_placeholder(title, chart_type, x, y, rows, html),
    }
}

/// The signed-in user's corner of the nav bar: a Users link for
/// admins, the username, and a sign-out button. `user` is
/// (username, is_admin), or None when nobody is signed in (or the app
/// has no auth at all) — in which case nothing renders.
fn nav_user_html(app: &str, user: Option<(&str, bool)>) -> String {
    match user {
        None => String::new(),
        Some((username, is_admin)) => {
            let admin_links = if is_admin {
                format!(
                    r#"<a class="pgapp-link" href="/{app}/users">Users</a><a class="pgapp-link" href="/{app}/admin/reload">Reload</a>"#,
                    app = escape(app),
                )
            } else {
                String::new()
            };
            format!(
                r#"<span class="pgapp-nav-user">{admin_links}<span class="pgapp-nav-username">{username}</span><form class="pgapp-inline-form" method="post" action="/{app}/logout"><button class="pgapp-btn pgapp-btn-secondary" type="submit">Sign out</button></form></span>"#,
                app = escape(app),
                username = escape(username),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn layout(
    app: &str,
    app_name: &str,
    title: &str,
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
    body: &str,
) -> String {
    // The app's own name/brand always lives in the header, not the nav
    // bar — the nav is for navigating *within* the app; saying what the
    // app *is* isn't one of its items. Any custom `header { }` chrome
    // from the markup renders right after the brand, inside the same
    // <header>.
    let header = format!(
        r#"<header class="pgapp-header"><a class="pgapp-link pgapp-brand" href="/{app_esc}">{brand}</a>{custom_header}</header>"#,
        app_esc = escape(app),
        brand = escape(app_name),
        custom_header = chrome_items_html(app, chrome.header, chrome.regions),
    );
    let footer = if chrome.footer.is_empty() {
        String::new()
    } else {
        format!(
            r#"<footer class="pgapp-footer">{}</footer>"#,
            chrome_items_html(app, chrome.footer, chrome.regions)
        )
    };

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="/{app_esc}/theme.css">
{icons_stylesheet}
<script src="/{app_esc}/runtime.js" defer></script>
{chart_lib_script}
{assets}
</head>
<body>
{header}
<nav class="pgapp-nav">
<button type="button" class="pgapp-nav-toggle" aria-expanded="false" aria-controls="pgapp-nav-collapse" aria-label="Toggle navigation">&#9776;</button>
<div class="pgapp-nav-collapse" id="pgapp-nav-collapse">{navbar}{nav_user}</div>
</nav>
<h1 class="pgapp-title">{title}</h1>
{body}
{footer}
</body>
</html>"#,
        app_esc = escape(app),
        title = escape(title),
        icons_stylesheet = icons.stylesheet_tag(),
        chart_lib_script = chart_lib
            .js_path
            .as_ref()
            .map(|_| format!(r#"<script src="/{}/chart-lib.js" defer></script>"#, escape(app)))
            .unwrap_or_default(),
        assets = asset_tags(app),
        navbar = nav_html(app, chrome.nav),
        nav_user = nav_user_html(app, user),
        header = header,
        body = body,
    )
}

/// A minimal, chrome-free page shell for auth screens: the login page
/// renders before there's a session, so it can't show nav/regions —
/// but it still links /{app}/theme.css, so it wears the app's theme.
fn bare_layout(app: &str, title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="/{app}/theme.css">
</head>
<body>
<h1 class="pgapp-title">{title}</h1>
{body}
</body>
</html>"#,
        app = escape(app),
        title = escape(title),
    )
}

/// The /{app}/login screen. In `setup` mode (the app has no users yet) it
/// becomes the one-time "create the admin account" form instead.
pub fn login_page(app: &str, app_name: &str, error: Option<&str>, setup: bool) -> String {
    let mut body = String::new();
    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }

    let (heading, note, action, button) = if setup {
        (
            "Create the admin account",
            "This app has no users yet. The account you create now becomes the administrator; after that, new users are added from the Users page.",
            format!("/{}/setup", escape(app)),
            "Create admin account",
        )
    } else {
        ("Sign in", "", format!("/{}/login", escape(app)), "Sign in")
    };

    body.push_str(&format!(
        r#"<div class="pgapp-form-panel"><h2 class="pgapp-subtitle">{heading}</h2>"#
    ));
    if !note.is_empty() {
        body.push_str(&format!(r#"<p class="pgapp-text">{}</p>"#, escape(note)));
    }
    body.push_str(&format!(
        r#"<form class="pgapp-form" method="post" action="{action}">
<div class="pgapp-field"><label class="pgapp-label">username</label><input class="pgapp-input" type="text" name="username" required autofocus></div>
<div class="pgapp-field"><label class="pgapp-label">password</label><input class="pgapp-input" type="password" name="password" required></div>
<button class="pgapp-btn pgapp-btn-primary" type="submit">{button}</button>
</form></div>"#
    ));

    bare_layout(app, app_name, &body)
}

/// The built-in /{app}/users admin page: every account, an add-user form,
/// and per-row delete (except your own account — see
/// `server::auth::users_delete`).
#[allow(clippy::too_many_arguments)]
pub fn users_page(
    app: &str,
    app_name: &str,
    users: &[(i32, String, String)],
    current_user_id: i32,
    error: Option<&str>,
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
) -> String {
    let mut body = String::new();
    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }

    body.push_str(r#"<div class="pgapp-report"><h2 class="pgapp-subtitle">Accounts</h2><div class="pgapp-table-wrap"><table class="pgapp-table"><thead><tr><th>username</th><th>role</th><th></th></tr></thead><tbody>"#);
    for (id, username, role) in users {
        let action = if *id == current_user_id {
            r#"<span class="pgapp-text">(you)</span>"#.to_string()
        } else {
            format!(
                r#"<form class="pgapp-inline-form" method="post" action="/{app}/users/{id}/delete" data-pgapp-confirm="Delete this account?"><button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{}</button></form>"#,
                icons.render("delete"),
                app = escape(app),
            )
        };
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"pgapp-row-actions\">{action}</td></tr>",
            escape(username),
            escape(role),
        ));
    }
    body.push_str("</tbody></table></div></div>");

    body.push_str(&format!(
        r#"<div class="pgapp-form-panel"><h2 class="pgapp-subtitle">Add user</h2>
<form class="pgapp-form" method="post" action="/{app}/users">
<div class="pgapp-field"><label class="pgapp-label">username</label><input class="pgapp-input" type="text" name="username" required></div>
<div class="pgapp-field"><label class="pgapp-label">password (min 8 chars)</label><input class="pgapp-input" type="password" name="password" required></div>
<div class="pgapp-field"><label class="pgapp-label">role</label><input class="pgapp-input" type="text" name="role" placeholder="user, admin, or any role your pages require"></div>
<button class="pgapp-btn pgapp-btn-primary" type="submit">Create user</button>
</form></div>"#,
        app = escape(app),
    ));

    layout(app, app_name, "Users", chrome, icons, chart_lib, user, &body)
}

/// The built-in /{app}/admin/reload page: re-parses the markup file and
/// re-syncs it into `pgapp_meta` and its workspace schema without
/// restarting the process (see `server::AppEntry::reload`). A single `.pgapp` file
/// can be edited in place here; a directory-based app (multiple
/// files merged, see `src/source.rs`) can only be re-read from disk,
/// since there's no one file to hand back to the browser.
#[allow(clippy::too_many_arguments)]
pub fn reload_page(
    app: &str,
    app_name: &str,
    markup_path: &str,
    markup_text: Option<&str>,
    error: Option<&str>,
    notice: Option<&str>,
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
) -> String {
    let mut body = String::new();
    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }
    if let Some(msg) = notice {
        body.push_str(&format!(r#"<div class="pgapp-alert pgapp-alert-success">{}</div>"#, escape(msg)));
    }

    body.push_str(&format!(
        r#"<div class="pgapp-form-panel"><h2 class="pgapp-subtitle">Reload metadata</h2>
<p class="pgapp-text">Markup file: <code>{}</code></p>"#,
        escape(markup_path)
    ));

    match markup_text {
        Some(text) => {
            body.push_str(&format!(
                r#"<form class="pgapp-form" method="post" action="/{app}/admin/reload">
<div class="pgapp-field"><textarea class="pgapp-input" name="markup" rows="20" spellcheck="false" style="font-family:monospace;white-space:pre;">{}</textarea></div>
<button class="pgapp-btn pgapp-btn-primary" type="submit" name="do" value="save">Save &amp; reload</button>
<button class="pgapp-btn pgapp-btn-secondary" type="submit" name="do" value="reload">Reload from disk (discard edits above)</button>
</form>"#,
                escape(text),
                app = escape(app),
            ));
        }
        None => {
            body.push_str(&format!(
                r#"<p class="pgapp-text">This app's markup is a directory of files — edit them on disk, then reload.</p>
<form class="pgapp-form" method="post" action="/{app}/admin/reload">
<button class="pgapp-btn pgapp-btn-primary" type="submit" name="do" value="reload">Reload from disk</button>
</form>"#,
                app = escape(app),
            ));
        }
    }
    body.push_str("</div>");

    layout(app, app_name, "Reload metadata", chrome, icons, chart_lib, user, &body)
}

/// Renders one field's input by looking up its registered item type
/// and calling that component's `render`. `resolved_choices` carries
/// whatever `query_engine::resolve_field_choices` already fetched for
/// fields whose config uses the `choices`/`query` convention.
#[allow(clippy::too_many_arguments)]
fn input_for_field(
    entity: &RuntimeEntity,
    item_types: &HashMap<String, FieldItem>,
    field_name: &str,
    value: Option<&str>,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    registry: &item_types::Registry,
    field_html: &HashMap<String, HtmlAttrs>,
) -> String {
    let field = entity.field(field_name).expect("field must exist on entity");
    let value = value.unwrap_or("");
    let field_item = item_types
        .get(field_name)
        .expect("every declared field has a resolved item type (see meta::sync_app)");
    let component = registry
        .get(field_item.kind.as_str())
        .unwrap_or_else(|| panic!("unknown item type '{}' for field '{field_name}'", field_item.kind));
    let empty_choices = Vec::new();
    let choices = resolved_choices.get(field_name).unwrap_or(&empty_choices);

    let input = component.render(RenderArgs {
        field_name,
        value,
        required: field.required,
        field_type: field.data_type,
        config: &field_item.config,
        choices,
    });

    static EMPTY_HTML: HtmlAttrs = HtmlAttrs { id: None, class: None, attrs: Vec::new() };
    let html = field_html.get(field_name).unwrap_or(&EMPTY_HTML);

    // data-pgapp-item lets dynamic actions show/hide/toggle the whole
    // field (label included), not just its input.
    format!(
        r#"<div class="{class}" data-pgapp-item="{label}"{extra}><label class="pgapp-label">{label}</label>{input}</div>"#,
        class = merged_class("pgapp-field", html),
        extra = extra_attrs(html),
        label = escape(field_name),
    )
}

/// A server-side action component: a button posting to the action's
/// run route. The outcome comes back as a notice/error banner.
pub fn action_html(app: &str, page_name: &str, idx: usize, label: &str, module: &str, html: &HtmlAttrs) -> String {
    format!(
        r#"<form class="{class}" method="post" action="/{app}/{page}/c/{idx}/run" title="runs the '{module}' module"{extra}><button class="pgapp-btn pgapp-btn-primary" type="submit">{label}</button></form>"#,
        class = merged_class("pgapp-action", html),
        extra = extra_attrs(html),
        app = escape(app),
        page = escape(page_name),
        idx = idx,
        module = escape(module),
        label = escape(label),
    )
}

/// All of a page's dynamic actions as one JSON script for the
/// runtime.js dispatcher (`pgapp` binds them on DOMContentLoaded).
pub fn dynamic_actions_script(actions: &[&serde_json::Value]) -> String {
    let json = serde_json::Value::Array(actions.iter().map(|v| (*v).clone()).collect());
    // `</` inside a <script> body would end it early regardless of type.
    let safe = json.to_string().replace("</", "<\\/");
    format!(r#"<script type="application/json" class="pgapp-dynamic-actions">{safe}</script>"#)
}

/// One saved-view chip on a report's toolbar.
pub struct ReportViewLink {
    pub id: i32,
    pub name: String,
    pub href: String,
    pub can_delete: bool,
}

/// A report's toolbar state: current filter values plus the saved views
/// visible to this user. `warning` carries a `before_load` action's
/// error text, if it failed on this request — non-fatal, shown inline
/// above the table rather than blocking the report from rendering.
pub struct ReportExtras {
    pub q: String,
    pub fcol: String,
    pub fval: String,
    pub views: Vec<ReportViewLink>,
    pub warning: Option<String>,
}

/// A read-only, paginated table — the `Report` component. Edit/delete
/// row actions appear only when `sibling_form_idx` is `Some` (a `Form`
/// bound to the same entity exists on this page); `prev_href`/
/// `next_href` are `None` at either end of the result set.
#[allow(clippy::too_many_arguments)]
pub fn report_html(
    app: &str,
    page_name: &str,
    idx: usize,
    title: &str,
    columns: &[String],
    rows: &[BTreeMap<String, Option<String>>],
    link_column: Option<&LinkColumn>,
    prev_href: Option<&str>,
    next_href: Option<&str>,
    sibling_form_idx: Option<usize>,
    icons: &Icons,
    extras: &ReportExtras,
    html: &HtmlAttrs,
) -> String {
    let mut body = format!(
        r#"<div class="{class}"{extra}><div class="pgapp-report-header"><h2 class="pgapp-subtitle">{title}</h2>"#,
        class = merged_class("pgapp-report", html),
        extra = extra_attrs(html),
        title = escape(title),
    );
    if let Some(form_idx) = sibling_form_idx {
        body.push_str(&format!(
            r#"<a class="pgapp-link pgapp-btn pgapp-btn-primary" href="/{app}/{page}?new_{form_idx}=1#pgapp-c{idx}">+ New</a>"#,
            app = escape(app),
            page = escape(page_name),
        ));
    }
    body.push_str("</div>");

    if let Some(warning) = &extras.warning {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error">{}</div>"#,
            escape(warning)
        ));
    }

    // Search toolbar: a GET form back to the page, so filters live in
    // the URL (shareable, and exactly what a saved view bookmarks). The
    // `#pgapp-c{idx}` fragment on the action survives a GET submission
    // (only the query is replaced), so Apply/Clear land back on this
    // report instead of resetting scroll to the page top.
    body.push_str(&format!(
        r#"<form class="pgapp-report-toolbar" method="get" action="/{app}/{page}#pgapp-c{idx}">
<input class="pgapp-input" type="search" name="r{idx}_q" value="{q}" placeholder="Search all columns">
<select class="pgapp-select" name="r{idx}_col"><option value="">column&hellip;</option>"#,
        app = escape(app),
        page = escape(page_name),
        idx = idx,
        q = escape(&extras.q),
    ));
    for col in columns {
        let selected = if *col == extras.fcol { " selected" } else { "" };
        body.push_str(&format!(
            r#"<option value="{c}"{selected}>{c}</option>"#,
            c = escape(col)
        ));
    }
    body.push_str(&format!(
        r#"</select>
<input class="pgapp-input" type="text" name="r{idx}_val" value="{val}" placeholder="contains&hellip;">
<button class="pgapp-btn pgapp-btn-secondary" type="submit">Apply</button>
<a class="pgapp-link" href="/{app}/{page}#pgapp-c{idx}">Clear</a>
</form>"#,
        idx = idx,
        val = escape(&extras.fval),
        app = escape(app),
        page = escape(page_name),
    ));

    // Saved views: chips applying a bookmarked filter state, plus the
    // save-current-state form.
    body.push_str(r#"<div class="pgapp-report-views">"#);
    for view in &extras.views {
        body.push_str(&format!(
            r#"<span class="pgapp-view-chip"><a class="pgapp-link" href="{href}">{name}</a>"#,
            href = escape(&view.href),
            name = escape(&view.name),
        ));
        if view.can_delete {
            body.push_str(&format!(
                r#"<form class="pgapp-inline-form" method="post" action="/{app}/{page}/c/{idx}/views/{id}/delete"><button class="pgapp-btn-viewdel" type="submit" title="Delete view">&times;</button></form>"#,
                app = escape(app),
                page = escape(page_name),
                idx = idx,
                id = view.id,
            ));
        }
        body.push_str("</span>");
    }
    body.push_str(&format!(
        r#"<form class="pgapp-view-save" method="post" action="/{app}/{page}/c/{idx}/views">
<input type="hidden" name="r{idx}_q" value="{q}">
<input type="hidden" name="r{idx}_col" value="{col}">
<input type="hidden" name="r{idx}_val" value="{val}">
<input class="pgapp-input" type="text" name="name" placeholder="Save view as&hellip;">
<label class="pgapp-view-public"><input type="checkbox" name="is_public"> public</label>
<button class="pgapp-btn pgapp-btn-secondary" type="submit">Save</button>
</form></div>"#,
        app = escape(app),
        page = escape(page_name),
        idx = idx,
        q = escape(&extras.q),
        col = escape(&extras.fcol),
        val = escape(&extras.fval),
    ));

    body.push_str(r#"<div class="pgapp-table-wrap"><table class="pgapp-table"><thead><tr>"#);
    for col in columns {
        body.push_str(&format!("<th>{}</th>", escape(col)));
    }
    if sibling_form_idx.is_some() {
        body.push_str("<th></th>");
    }
    body.push_str("</tr></thead><tbody>");

    for row in rows {
        body.push_str("<tr>");
        let id = row.get("id").and_then(|v| v.as_deref()).unwrap_or("");
        for col in columns {
            let val = row.get(col).and_then(|v| v.as_deref()).unwrap_or("");
            let cell = match link_column {
                Some(lc) if lc.field == *col => {
                    let mut href = format!("/{}/{}?id={}", escape(app), escape(&lc.target_page), url_encode(id));
                    for (field, param) in &lc.extra_params {
                        let pval = row.get(field).and_then(|v| v.as_deref()).unwrap_or("");
                        href.push_str(&format!("&{}={}", escape(param), url_encode(pval)));
                    }
                    format!(r#"<a class="pgapp-link" href="{href}">{val}</a>"#, val = escape(val))
                }
                _ => escape(val),
            };
            body.push_str(&format!("<td>{cell}</td>"));
        }
        if let Some(form_idx) = sibling_form_idx {
            body.push_str(&format!(
                r#"<td class="pgapp-row-actions">
<a class="pgapp-link" href="/{app}/{page}?edit_{form_idx}={id}#pgapp-c{idx}" title="Edit">{edit_icon}</a>
<form class="pgapp-inline-form" method="post" action="/{app}/{page}/c/{form_idx}/delete/{id}" data-pgapp-confirm="Delete this row?">
<button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{delete_icon}</button>
</form>
</td>"#,
                app = escape(app),
                page = escape(page_name),
                form_idx = form_idx,
                id = escape(id),
                edit_icon = icons.render("edit"),
                delete_icon = icons.render("delete"),
            ));
        }
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table></div>");

    if prev_href.is_some() || next_href.is_some() {
        body.push_str(r#"<div class="pgapp-pagination">"#);
        match prev_href {
            Some(href) => body.push_str(&format!(
                r#"<a class="pgapp-link pgapp-btn pgapp-btn-secondary" href="{}">&laquo; Prev</a>"#,
                escape(href)
            )),
            None => body.push_str(r#"<span class="pgapp-btn pgapp-btn-secondary pgapp-btn-disabled">&laquo; Prev</span>"#),
        }
        match next_href {
            Some(href) => body.push_str(&format!(
                r#"<a class="pgapp-link pgapp-btn pgapp-btn-secondary" href="{}">Next &raquo;</a>"#,
                escape(href)
            )),
            None => body.push_str(r#"<span class="pgapp-btn pgapp-btn-secondary pgapp-btn-disabled">Next &raquo;</span>"#),
        }
        body.push_str("</div>");
    }

    body.push_str("</div>");
    body
}

/// A `Form` component: blank (create mode) when `edit_id` is `None`,
/// pre-filled with `row` and carrying a Delete button when `Some`. When
/// `floating` is true (a Report's edit/create companion), it renders as
/// a fixed-position popup rather than a block sitting in page flow — see
/// `.pgapp-form-floating` in the theme CSS — with a close control going
/// back to `close_href` instead of a plain page-top navigation.
#[allow(clippy::too_many_arguments)]
pub fn form_html(
    app: &str,
    page_name: &str,
    idx: usize,
    title: &str,
    fields: &[String],
    entity: &RuntimeEntity,
    row: &BTreeMap<String, Option<String>>,
    edit_id: Option<&str>,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    item_types: &HashMap<String, FieldItem>,
    registry: &item_types::Registry,
    floating: bool,
    close_href: &str,
    field_html: &HashMap<String, HtmlAttrs>,
    html: &HtmlAttrs,
) -> String {
    let panel_class = if floating {
        "pgapp-form-panel pgapp-form-floating"
    } else {
        "pgapp-form-panel"
    };
    let mut body = format!(
        r#"<div class="{class}"{extra}>"#,
        class = merged_class(panel_class, html),
        extra = extra_attrs(html),
    );
    if floating {
        body.push_str(&format!(
            r#"<a class="pgapp-form-floating-close" href="{href}" title="Close" aria-label="Close">&times;</a>"#,
            href = escape(close_href),
        ));
    }
    body.push_str(&format!(r#"<h2 class="pgapp-subtitle">{}</h2>"#, escape(title)));

    let action = match edit_id {
        Some(id) => format!("/{}/{}/c/{idx}/update/{}", escape(app), escape(page_name), escape(id)),
        None => format!("/{}/{}/c/{idx}/create", escape(app), escape(page_name)),
    };
    body.push_str(&format!(r#"<form class="pgapp-form" method="post" action="{action}">"#));
    for field_name in fields {
        let value = row.get(field_name).and_then(|v| v.as_deref());
        body.push_str(&input_for_field(entity, item_types, field_name, value, resolved_choices, registry, field_html));
    }
    let submit_label = if edit_id.is_some() { "Save" } else { "Create" };
    body.push_str(&format!(
        r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">{submit_label}</button></form>"#
    ));

    if let Some(id) = edit_id {
        body.push_str(&format!(
            r#"<form class="pgapp-inline-form" method="post" action="/{app}/{page}/c/{idx}/delete/{id}" data-pgapp-confirm="Delete this row?">
<button class="pgapp-btn pgapp-btn-destructive" type="submit">Delete</button></form>"#,
            app = escape(app),
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
    }
    if floating || edit_id.is_some() {
        body.push_str(&format!(r#"<a class="pgapp-link" href="{}">Cancel</a>"#, escape(close_href)));
    }

    body.push_str("</div>");
    body
}

/// An `EditableTable` component: every row rendered as its own inline
/// form (fields laid out horizontally via CSS to look table-like),
/// plus an "add new" form at the bottom. Deliberately not a literal
/// `<table>` — a `<form>` can't wrap `<tr>`/`<td>` — but styled to read
/// as one.
#[allow(clippy::too_many_arguments)]
pub fn editable_table_html(
    app: &str,
    page_name: &str,
    idx: usize,
    title: &str,
    columns: &[String],
    entity: &RuntimeEntity,
    rows: &[BTreeMap<String, Option<String>>],
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    item_types: &HashMap<String, FieldItem>,
    registry: &item_types::Registry,
    icons: &Icons,
    field_html: &HashMap<String, HtmlAttrs>,
    html: &HtmlAttrs,
) -> String {
    let mut body = format!(
        r#"<div class="{class}"{extra}><h2 class="pgapp-subtitle">{title}</h2>"#,
        class = merged_class("pgapp-editable-table", html),
        extra = extra_attrs(html),
        title = escape(title),
    );

    for row in rows {
        let id = row.get("id").and_then(|v| v.as_deref()).unwrap_or("");
        body.push_str(r#"<div class="pgapp-editable-row-wrap">"#);
        body.push_str(&format!(
            r#"<form class="pgapp-editable-row" method="post" action="/{app}/{page}/c/{idx}/update/{id}">"#,
            app = escape(app),
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
        for col in columns {
            let value = row.get(col).and_then(|v| v.as_deref());
            body.push_str(&input_for_field(entity, item_types, col, value, resolved_choices, registry, field_html));
        }
        body.push_str(&format!(
            r#"<button class="pgapp-btn pgapp-btn-primary" type="submit" title="Save">{save_icon}</button></form>
<form class="pgapp-inline-form" method="post" action="/{app}/{page}/c/{idx}/delete/{id}" data-pgapp-confirm="Delete this row?">
<button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{delete_icon}</button></form>"#,
            save_icon = icons.render("edit"),
            delete_icon = icons.render("delete"),
            app = escape(app),
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
        body.push_str("</div>");
    }

    body.push_str(&format!(
        r#"<h3 class="pgapp-region-title">Add new</h3><div class="pgapp-editable-row-wrap"><form class="pgapp-form pgapp-editable-row" method="post" action="/{app}/{page}/c/{idx}/create">"#,
        app = escape(app),
        page = escape(page_name),
        idx = idx,
    ));
    for col in columns {
        body.push_str(&input_for_field(entity, item_types, col, None, resolved_choices, registry, field_html));
    }
    body.push_str(r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">Add</button></form></div>"#);

    body.push_str("</div>");
    body
}

/// Wraps a page's already-rendered component bodies in the standard
/// layout, with an optional page-level error banner (surfaced via the
/// `?error=` query parameter after a failed create/update — see
/// `server.rs`).
#[allow(clippy::too_many_arguments)]
pub fn page_layout(
    app: &str,
    app_name: &str,
    title: &str,
    body: &str,
    error: Option<&str>,
    notice: Option<&str>,
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
) -> String {
    let mut full = String::new();
    if let Some(err) = error {
        full.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }
    if let Some(msg) = notice {
        full.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-success">{}</div>"#,
            escape(msg)
        ));
    }
    full.push_str(body);
    layout(app, app_name, title, chrome, icons, chart_lib, user, &full)
}

/// The landing page at bare `/` when a server serves more than one
/// app — a plain list of links into each app's own `/{slug}`. A
/// single-app server never renders this: `/` redirects straight to
/// the one app instead (see `server::landing`).
pub fn workspace_landing(apps: &[String]) -> String {
    let mut items = String::new();
    for slug in apps {
        items.push_str(&format!(
            r#"<li><a class="pgapp-link" href="/{slug}">{slug}</a></li>"#,
            slug = escape(slug),
        ));
    }
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>pgapp</title>
</head>
<body>
<h1 class="pgapp-title">Apps on this server</h1>
<ul class="pgapp-navbar">{items}</ul>
</body>
</html>"#
    )
}
