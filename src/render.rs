//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! a page's component list comes from `RuntimePage`/`RuntimeApp`, not
//! from a per-app template.
//!
//! Markup here only ever uses the fixed `.pgapp-*` class names — the
//! "Theme contract" documented in the README. All actual look-and-feel
//! comes from `/theme.css` (the active theme, see src/theme.rs) plus any
//! app-level override in assets/app.css. A form field's actual input is
//! never built here — `input_for_field` just hands off to whatever
//! component is registered for that field's item type (see
//! `src/item_types.rs`), so adding a new one never touches this file.
//! Component *data fetching* (rows, pagination, resolved choices) is
//! `server.rs`'s job; this module only ever formats what it's handed.

use crate::chart_lib::ChartLib;
use crate::html::{escape, url_encode};
use crate::icons::Icons;
use crate::item_types::{self, RenderArgs};
use crate::meta::{Chrome, LinkColumn, NavNode, RegionRows, RuntimeComponent, RuntimeEntity};
use crate::model::FieldItem;
use std::collections::{BTreeMap, HashMap};

/// Extra `<link>`/`<script>` tags for user-supplied assets, if present —
/// the app-level override layer, on top of the active theme.
pub fn asset_tags() -> String {
    let mut tags = String::new();
    if std::path::Path::new("assets/app.css").exists() {
        tags.push_str("<link rel=\"stylesheet\" href=\"/assets/app.css\">\n");
    }
    if std::path::Path::new("assets/app.js").exists() {
        tags.push_str("<script src=\"/assets/app.js\" defer></script>\n");
    }
    tags
}

/// Renders the app's (possibly multi-level) nav bar as nested `<ul>`s;
/// submenus are shown on hover/focus via the theme's CSS, not JS.
fn nav_html(nodes: &[NavNode]) -> String {
    if nodes.is_empty() {
        return String::new();
    }
    let mut html = String::from(r#"<ul class="pgapp-navbar">"#);
    for node in nodes {
        html.push_str(&nav_node_html(node));
    }
    html.push_str("</ul>");
    html
}

fn nav_node_html(node: &NavNode) -> String {
    let mut html = String::from(r#"<li class="pgapp-navbar-item">"#);
    match &node.target_page {
        Some(target) => html.push_str(&format!(
            r#"<a class="pgapp-link" href="/{target}">{label}</a>"#,
            target = escape(target),
            label = escape(&node.label),
        )),
        None => html.push_str(&format!(
            r#"<span class="pgapp-navbar-label">{}</span>"#,
            escape(&node.label)
        )),
    }
    if !node.children.is_empty() {
        html.push_str(r#"<ul class="pgapp-navbar-submenu">"#);
        for child in &node.children {
            html.push_str(&nav_node_html(child));
        }
        html.push_str("</ul>");
    }
    html.push_str("</li>");
    html
}

pub fn text_html(text: &str) -> String {
    format!(r#"<p class="pgapp-text">{}</p>"#, escape(text))
}

pub fn link_html(label: &str, target_page: &str) -> String {
    format!(
        r#"<p><a class="pgapp-link" href="/{target}">{label}</a></p>"#,
        target = escape(target_page),
        label = escape(label),
    )
}

/// Renders one `Region` component: a named query's (already-resolved)
/// rows as a plain table, with column headers taken from the row keys.
pub fn region_html(label: &str, query: &str, regions: &RegionRows) -> String {
    // data-pgapp-region lets a dynamic action's `refresh` op find and
    // replace this container with a freshly fetched fragment.
    let mut html = format!(
        r#"<div class="pgapp-region" data-pgapp-region="{query}"><h3 class="pgapp-region-title">{label}</h3>"#,
        query = escape(query),
        label = escape(label),
    );
    match regions.get(query).filter(|rows| !rows.is_empty()) {
        Some(rows) => {
            let mut cols: Vec<&String> = rows[0].keys().collect();
            cols.sort();

            html.push_str(r#"<table class="pgapp-table"><thead><tr>"#);
            for c in &cols {
                html.push_str(&format!("<th>{}</th>", escape(c)));
            }
            html.push_str("</tr></thead><tbody>");
            for row in rows {
                html.push_str("<tr>");
                for c in &cols {
                    let val = row.get(*c).and_then(|v| v.as_deref()).unwrap_or("");
                    html.push_str(&format!("<td>{}</td>", escape(val)));
                }
                html.push_str("</tr>");
            }
            html.push_str("</tbody></table>");
        }
        None => html.push_str(r#"<p class="pgapp-text">No results.</p>"#),
    }
    html.push_str("</div>");
    html
}

/// Renders a header/footer chrome list — restricted at sync time to
/// Text/Link/Region, so those are the only variants handled here.
fn chrome_items_html(items: &[RuntimeComponent], regions: &RegionRows) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut html = String::from(r#"<div class="pgapp-items">"#);
    for item in items {
        match item {
            RuntimeComponent::Text(text) => html.push_str(&text_html(text)),
            RuntimeComponent::Link { label, target_page } => html.push_str(&link_html(label, target_page)),
            RuntimeComponent::Region { label, query } => html.push_str(&region_html(label, query, regions)),
            _ => {}
        }
    }
    html.push_str("</div>");
    html
}

/// A dependency-free bar/line chart rendered straight to inline SVG —
/// the built-in `PGAPP_CHART_LIB=inline` backend (see
/// `src/chart_lib.rs`). No JS, no network fetch.
fn inline_svg_chart(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>]) -> String {
    let (width, height, pad) = (480.0_f64, 220.0_f64, 30.0_f64);
    let values: Vec<f64> = rows
        .iter()
        .map(|r| r.get(y).and_then(|v| v.as_deref()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0))
        .collect();
    let labels: Vec<String> = rows.iter().map(|r| r.get(x).and_then(|v| v.as_deref()).unwrap_or("").to_string()).collect();
    let max = values.iter().cloned().fold(1.0_f64, f64::max).max(1.0);
    let n = (values.len().max(1)) as f64;
    let bar_w = (width - pad * 2.0) / n;
    let baseline = height - pad;

    let mut svg = format!(
        r#"<svg class="pgapp-chart-svg" viewBox="0 0 {width} {height}" role="img" aria-label="{}">"#,
        escape(title)
    );
    svg.push_str(&format!(
        r#"<line x1="{pad}" y1="{baseline}" x2="{}" y2="{baseline}" stroke="currentColor" stroke-opacity="0.3"/>"#,
        width - pad
    ));

    if chart_type == "line" {
        let points: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let px = pad + bar_w * (i as f64 + 0.5);
                let py = baseline - (v / max) * (height - pad * 2.0);
                format!("{px:.1},{py:.1}")
            })
            .collect();
        svg.push_str(&format!(
            r#"<polyline points="{}" fill="none" stroke="currentColor" stroke-width="2"/>"#,
            points.join(" ")
        ));
        for (i, v) in values.iter().enumerate() {
            let px = pad + bar_w * (i as f64 + 0.5);
            let py = baseline - (v / max) * (height - pad * 2.0);
            svg.push_str(&format!(r#"<circle cx="{px:.1}" cy="{py:.1}" r="3" fill="currentColor"/>"#));
        }
    } else {
        for (i, v) in values.iter().enumerate() {
            let bar_h = (v / max) * (height - pad * 2.0);
            let bx = pad + bar_w * (i as f64) + 2.0;
            let by = baseline - bar_h;
            svg.push_str(&format!(
                r#"<rect x="{bx:.1}" y="{by:.1}" width="{:.1}" height="{bar_h:.1}" fill="currentColor"/>"#,
                (bar_w - 4.0).max(1.0)
            ));
        }
    }

    for (i, label) in labels.iter().enumerate() {
        let px = pad + bar_w * (i as f64 + 0.5);
        svg.push_str(&format!(
            r#"<text x="{px:.1}" y="{:.1}" font-size="9" text-anchor="middle">{}</text>"#,
            baseline + 12.0,
            escape(label)
        ));
    }
    svg.push_str("</svg>");
    format!(
        r#"<div class="pgapp-chart"><h3 class="pgapp-region-title">{}</h3>{svg}</div>"#,
        escape(title)
    )
}

/// A JSON-in-`<script>` placeholder for a pluggable chart library (see
/// `src/chart_lib.rs`): the library's JS (served at `/chart-lib.js`)
/// reads this data and renders into the surrounding `.pgapp-chart` div
/// however it likes.
fn pluggable_chart_placeholder(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>]) -> String {
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
        r#"<div class="pgapp-chart"><h3 class="pgapp-region-title">{}</h3><script type="application/json" class="pgapp-chart-data">{safe_json}</script></div>"#,
        escape(title)
    )
}

pub fn chart_html(title: &str, chart_type: &str, x: &str, y: &str, rows: &[BTreeMap<String, Option<String>>], chart_lib: &ChartLib) -> String {
    match &chart_lib.js_path {
        None => inline_svg_chart(title, chart_type, x, y, rows),
        Some(_) => pluggable_chart_placeholder(title, chart_type, x, y, rows),
    }
}

/// The signed-in user's corner of the nav bar: a Users link for
/// admins, the username, and a sign-out button. `user` is
/// (username, is_admin), or None when nobody is signed in (or the app
/// has no auth at all) — in which case nothing renders.
fn nav_user_html(user: Option<(&str, bool)>) -> String {
    match user {
        None => String::new(),
        Some((username, is_admin)) => {
            let users_link = if is_admin {
                r#"<a class="pgapp-link" href="/users">Users</a>"#.to_string()
            } else {
                String::new()
            };
            format!(
                r#"<span class="pgapp-nav-user">{users_link}<span class="pgapp-nav-username">{username}</span><form class="pgapp-inline-form" method="post" action="/logout"><button class="pgapp-btn pgapp-btn-secondary" type="submit">Sign out</button></form></span>"#,
                username = escape(username),
            )
        }
    }
}

fn layout(
    title: &str,
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
    body: &str,
) -> String {
    let header = if chrome.header.is_empty() {
        String::new()
    } else {
        format!(
            r#"<header class="pgapp-header">{}</header>"#,
            chrome_items_html(chrome.header, chrome.regions)
        )
    };
    let footer = if chrome.footer.is_empty() {
        String::new()
    } else {
        format!(
            r#"<footer class="pgapp-footer">{}</footer>"#,
            chrome_items_html(chrome.footer, chrome.regions)
        )
    };

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<link rel="stylesheet" href="/theme.css">
{icons_stylesheet}
<script src="/runtime.js" defer></script>
{chart_lib_script}
{assets}
</head>
<body>
{header}
<nav class="pgapp-nav"><a class="pgapp-link" href="/">pgapp</a>{navbar}{nav_user}</nav>
<h1 class="pgapp-title">{title}</h1>
{body}
{footer}
</body>
</html>"#,
        title = escape(title),
        icons_stylesheet = icons.stylesheet_tag(),
        chart_lib_script = chart_lib
            .js_path
            .as_ref()
            .map(|_| r#"<script src="/chart-lib.js" defer></script>"#)
            .unwrap_or(""),
        assets = asset_tags(),
        navbar = nav_html(chrome.nav),
        nav_user = nav_user_html(user),
        body = body,
    )
}

/// A minimal, chrome-free page shell for auth screens: the login page
/// renders before there's a session, so it can't show nav/regions —
/// but it still links /theme.css, so it wears the app's theme.
fn bare_layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<link rel="stylesheet" href="/theme.css">
</head>
<body>
<h1 class="pgapp-title">{title}</h1>
{body}
</body>
</html>"#,
        title = escape(title),
    )
}

/// The /login screen. In `setup` mode (the app has no users yet) it
/// becomes the one-time "create the admin account" form instead.
pub fn login_page(app_name: &str, error: Option<&str>, setup: bool) -> String {
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
            "/setup",
            "Create admin account",
        )
    } else {
        ("Sign in", "", "/login", "Sign in")
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

    bare_layout(app_name, &body)
}

/// The built-in /users admin page: every account, an add-user form,
/// and per-row delete (except your own account — see
/// `server::auth::users_delete`).
#[allow(clippy::too_many_arguments)]
pub fn users_page(
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

    body.push_str(r#"<div class="pgapp-report"><h2 class="pgapp-subtitle">Accounts</h2><table class="pgapp-table"><thead><tr><th>username</th><th>role</th><th></th></tr></thead><tbody>"#);
    for (id, username, role) in users {
        let action = if *id == current_user_id {
            r#"<span class="pgapp-text">(you)</span>"#.to_string()
        } else {
            format!(
                r#"<form class="pgapp-inline-form" method="post" action="/users/{id}/delete" onsubmit="return confirm('Delete this account?')"><button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{}</button></form>"#,
                icons.render("delete"),
            )
        };
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"pgapp-row-actions\">{action}</td></tr>",
            escape(username),
            escape(role),
        ));
    }
    body.push_str("</tbody></table></div>");

    body.push_str(
        r#"<div class="pgapp-form-panel"><h2 class="pgapp-subtitle">Add user</h2>
<form class="pgapp-form" method="post" action="/users">
<div class="pgapp-field"><label class="pgapp-label">username</label><input class="pgapp-input" type="text" name="username" required></div>
<div class="pgapp-field"><label class="pgapp-label">password (min 8 chars)</label><input class="pgapp-input" type="password" name="password" required></div>
<div class="pgapp-field"><label class="pgapp-label">role</label><input class="pgapp-input" type="text" name="role" placeholder="user, admin, or any role your pages require"></div>
<button class="pgapp-btn pgapp-btn-primary" type="submit">Create user</button>
</form></div>"#,
    );

    layout("Users", chrome, icons, chart_lib, user, &body)
}

/// Renders one field's input by looking up its registered item type
/// and calling that component's `render`. `resolved_choices` carries
/// whatever `query_engine::resolve_field_choices` already fetched for
/// fields whose config uses the `choices`/`query` convention.
fn input_for_field(
    entity: &RuntimeEntity,
    item_types: &HashMap<String, FieldItem>,
    field_name: &str,
    value: Option<&str>,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    registry: &item_types::Registry,
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

    // data-pgapp-item lets dynamic actions show/hide/toggle the whole
    // field (label included), not just its input.
    format!(
        r#"<div class="pgapp-field" data-pgapp-item="{label}"><label class="pgapp-label">{label}</label>{input}</div>"#,
        label = escape(field_name),
    )
}

/// A server-side action component: a button posting to the action's
/// run route. The outcome comes back as a notice/error banner.
pub fn action_html(page_name: &str, idx: usize, label: &str, module: &str) -> String {
    format!(
        r#"<form class="pgapp-action" method="post" action="/{page}/c/{idx}/run" title="runs the '{module}' module"><button class="pgapp-btn pgapp-btn-primary" type="submit">{label}</button></form>"#,
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
/// visible to this user.
pub struct ReportExtras {
    pub q: String,
    pub fcol: String,
    pub fval: String,
    pub views: Vec<ReportViewLink>,
}

/// A read-only, paginated table — the `Report` component. Edit/delete
/// row actions appear only when `sibling_form_idx` is `Some` (a `Form`
/// bound to the same entity exists on this page); `prev_href`/
/// `next_href` are `None` at either end of the result set.
#[allow(clippy::too_many_arguments)]
pub fn report_html(
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
) -> String {
    let mut body = format!(r#"<div class="pgapp-report"><h2 class="pgapp-subtitle">{}</h2>"#, escape(title));

    // Search toolbar: a GET form back to the page, so filters live in
    // the URL (shareable, and exactly what a saved view bookmarks).
    body.push_str(&format!(
        r#"<form class="pgapp-report-toolbar" method="get" action="/{page}">
<input class="pgapp-input" type="search" name="r{idx}_q" value="{q}" placeholder="Search all columns">
<select class="pgapp-select" name="r{idx}_col"><option value="">column&hellip;</option>"#,
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
<a class="pgapp-link" href="/{page}">Clear</a>
</form>"#,
        idx = idx,
        val = escape(&extras.fval),
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
                r#"<form class="pgapp-inline-form" method="post" action="/{page}/c/{idx}/views/{id}/delete"><button class="pgapp-btn-viewdel" type="submit" title="Delete view">&times;</button></form>"#,
                page = escape(page_name),
                idx = idx,
                id = view.id,
            ));
        }
        body.push_str("</span>");
    }
    body.push_str(&format!(
        r#"<form class="pgapp-view-save" method="post" action="/{page}/c/{idx}/views">
<input type="hidden" name="r{idx}_q" value="{q}">
<input type="hidden" name="r{idx}_col" value="{col}">
<input type="hidden" name="r{idx}_val" value="{val}">
<input class="pgapp-input" type="text" name="name" placeholder="Save view as&hellip;">
<label class="pgapp-view-public"><input type="checkbox" name="is_public"> public</label>
<button class="pgapp-btn pgapp-btn-secondary" type="submit">Save</button>
</form></div>"#,
        page = escape(page_name),
        idx = idx,
        q = escape(&extras.q),
        col = escape(&extras.fcol),
        val = escape(&extras.fval),
    ));

    body.push_str(r#"<table class="pgapp-table"><thead><tr>"#);
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
                    let mut href = format!("/{}?id={}", escape(&lc.target_page), url_encode(id));
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
<a class="pgapp-link" href="/{page}?edit_{form_idx}={id}" title="Edit">{edit_icon}</a>
<form class="pgapp-inline-form" method="post" action="/{page}/c/{form_idx}/delete/{id}" onsubmit="return confirm('Delete this row?')">
<button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{delete_icon}</button>
</form>
</td>"#,
                page = escape(page_name),
                form_idx = form_idx,
                id = escape(id),
                edit_icon = icons.render("edit"),
                delete_icon = icons.render("delete"),
            ));
        }
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table>");

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
/// pre-filled with `row` and carrying a Delete button when `Some`.
#[allow(clippy::too_many_arguments)]
pub fn form_html(
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
) -> String {
    let mut body = format!(r#"<div class="pgapp-form-panel"><h2 class="pgapp-subtitle">{}</h2>"#, escape(title));

    let action = match edit_id {
        Some(id) => format!("/{}/c/{idx}/update/{}", escape(page_name), escape(id)),
        None => format!("/{}/c/{idx}/create", escape(page_name)),
    };
    body.push_str(&format!(r#"<form class="pgapp-form" method="post" action="{action}">"#));
    for field_name in fields {
        let value = row.get(field_name).and_then(|v| v.as_deref());
        body.push_str(&input_for_field(entity, item_types, field_name, value, resolved_choices, registry));
    }
    let submit_label = if edit_id.is_some() { "Save" } else { "Create" };
    body.push_str(&format!(
        r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">{submit_label}</button></form>"#
    ));

    if let Some(id) = edit_id {
        body.push_str(&format!(
            r#"<form class="pgapp-inline-form" method="post" action="/{page}/c/{idx}/delete/{id}" onsubmit="return confirm('Delete this row?')">
<button class="pgapp-btn pgapp-btn-destructive" type="submit">Delete</button></form>
<a class="pgapp-link" href="/{page}">Cancel</a>"#,
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
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
) -> String {
    let mut body = format!(r#"<div class="pgapp-editable-table"><h2 class="pgapp-subtitle">{}</h2>"#, escape(title));

    for row in rows {
        let id = row.get("id").and_then(|v| v.as_deref()).unwrap_or("");
        body.push_str(r#"<div class="pgapp-editable-row-wrap">"#);
        body.push_str(&format!(
            r#"<form class="pgapp-editable-row" method="post" action="/{page}/c/{idx}/update/{id}">"#,
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
        for col in columns {
            let value = row.get(col).and_then(|v| v.as_deref());
            body.push_str(&input_for_field(entity, item_types, col, value, resolved_choices, registry));
        }
        body.push_str(&format!(
            r#"<button class="pgapp-btn pgapp-btn-primary" type="submit" title="Save">{save_icon}</button></form>
<form class="pgapp-inline-form" method="post" action="/{page}/c/{idx}/delete/{id}" onsubmit="return confirm('Delete this row?')">
<button class="pgapp-btn pgapp-btn-destructive" type="submit" title="Delete">{delete_icon}</button></form>"#,
            save_icon = icons.render("edit"),
            delete_icon = icons.render("delete"),
            page = escape(page_name),
            idx = idx,
            id = escape(id),
        ));
        body.push_str("</div>");
    }

    body.push_str(&format!(
        r#"<h3 class="pgapp-region-title">Add new</h3><div class="pgapp-editable-row-wrap"><form class="pgapp-form pgapp-editable-row" method="post" action="/{page}/c/{idx}/create">"#,
        page = escape(page_name),
        idx = idx,
    ));
    for col in columns {
        body.push_str(&input_for_field(entity, item_types, col, None, resolved_choices, registry));
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
    layout(title, chrome, icons, chart_lib, user, &full)
}

pub fn index_page(
    app_name: &str,
    pages: &[String],
    chrome: Chrome,
    icons: &Icons,
    chart_lib: &ChartLib,
    user: Option<(&str, bool)>,
) -> String {
    let mut body = String::from(r#"<ul class="pgapp-list">"#);
    for p in pages {
        body.push_str(&format!(
            r#"<li><a class="pgapp-link" href="/{p}">{p}</a></li>"#,
            p = escape(p)
        ));
    }
    body.push_str("</ul>");
    layout(app_name, chrome, icons, chart_lib, user, &body)
}
