//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! the field/item/nav lists come from `RuntimePage`/`RuntimeApp`, not
//! from a per-app template.
//!
//! Markup here only ever uses the fixed `.pgapp-*` class names — the
//! "Theme contract" documented in the README. All actual look-and-feel
//! comes from `/theme.css` (the active theme, see src/theme.rs) plus any
//! app-level override in assets/app.css. A form field's actual input is
//! never built here — `input_for_field` just hands off to whatever
//! component is registered for that field's item type (see
//! `src/item_types.rs`), so adding a new one never touches this file.

use crate::html::{escape, url_encode};
use crate::item_types::{self, RenderArgs};
use crate::meta::{Chrome, NavNode, RegionRows, RuntimePage, RuntimePageItem};
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

/// Renders an item list (page `items`, or the app's header/footer):
/// static text, links to other pages, and regions rendering a named
/// query's already-resolved rows (see
/// `server::query_engine::resolve_regions`).
fn items_html(items: &[RuntimePageItem], regions: &RegionRows) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut html = String::from(r#"<div class="pgapp-items">"#);
    for item in items {
        match item {
            RuntimePageItem::Text(text) => {
                html.push_str(&format!(r#"<p class="pgapp-text">{}</p>"#, escape(text)));
            }
            RuntimePageItem::Link { label, target_page } => {
                html.push_str(&format!(
                    r#"<p><a class="pgapp-link" href="/{target}">{label}</a></p>"#,
                    target = escape(target_page),
                    label = escape(label),
                ));
            }
            RuntimePageItem::Region { label, query } => {
                html.push_str(&render_region(label, query, regions));
            }
        }
    }
    html.push_str("</div>");
    html
}

/// Renders one `Region` item: a named query's rows as a table, with
/// column headers taken from the (already-resolved) row keys.
fn render_region(label: &str, query: &str, regions: &RegionRows) -> String {
    let mut html = format!(
        r#"<div class="pgapp-region"><h3 class="pgapp-region-title">{}</h3>"#,
        escape(label)
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

fn layout(title: &str, chrome: Chrome, body: &str) -> String {
    let header = if chrome.header.is_empty() {
        String::new()
    } else {
        format!(
            r#"<header class="pgapp-header">{}</header>"#,
            items_html(chrome.header, chrome.regions)
        )
    };
    let footer = if chrome.footer.is_empty() {
        String::new()
    } else {
        format!(
            r#"<footer class="pgapp-footer">{}</footer>"#,
            items_html(chrome.footer, chrome.regions)
        )
    };

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<link rel="stylesheet" href="/theme.css">
<script src="/runtime.js" defer></script>
{assets}
</head>
<body>
{header}
<nav class="pgapp-nav"><a class="pgapp-link" href="/">pgapp</a>{navbar}</nav>
<h1 class="pgapp-title">{title}</h1>
{body}
{footer}
</body>
</html>"#,
        title = escape(title),
        assets = asset_tags(),
        navbar = nav_html(chrome.nav),
        body = body,
    )
}

/// Renders one field's input by looking up its registered item type
/// (`page.item_types[field_name].kind`) and calling that component's
/// `render`. `resolved_choices` carries whatever
/// `query_engine::resolve_field_choices` already fetched for fields
/// whose config uses the `choices`/`query` convention.
fn input_for_field(
    page: &RuntimePage,
    field_name: &str,
    value: Option<&str>,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    registry: &item_types::Registry,
) -> String {
    let field = page
        .entity
        .as_ref()
        .and_then(|e| e.field(field_name))
        .expect("form field must exist on entity");
    let value = value.unwrap_or("");
    let field_item = page
        .item_types
        .get(field_name)
        .expect("every form field has a resolved item type (see meta::sync_app)");
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

    format!(
        r#"<div class="pgapp-field"><label class="pgapp-label">{label}</label>{input}</div>"#,
        label = escape(field_name),
    )
}

pub fn list_page(
    page: &RuntimePage,
    rows: &[BTreeMap<String, Option<String>>],
    error: Option<&str>,
    chrome: Chrome,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    registry: &item_types::Registry,
) -> String {
    let mut body = String::new();

    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }

    body.push_str(&items_html(&page.items, chrome.regions));

    body.push_str(r#"<table class="pgapp-table"><thead><tr>"#);
    for col in &page.columns {
        body.push_str(&format!("<th>{}</th>", escape(col)));
    }
    body.push_str("<th></th></tr></thead><tbody>");

    for row in rows {
        body.push_str("<tr>");
        let id = row.get("id").and_then(|v| v.as_deref()).unwrap_or("");
        for col in &page.columns {
            let val = row.get(col).and_then(|v| v.as_deref()).unwrap_or("");
            let cell = match &page.link_column {
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
        body.push_str(&format!(
            r#"<td>
<a class="pgapp-link" href="/{page}/{id}/edit">Edit</a>
<form class="pgapp-inline-form" method="post" action="/{page}/{id}/delete" onsubmit="return confirm('Delete this row?')">
<button class="pgapp-btn pgapp-btn-destructive" type="submit">Delete</button>
</form>
</td>"#,
            page = escape(&page.name),
            id = escape(id),
        ));
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table>");

    body.push_str(&format!(
        r#"<h2 class="pgapp-subtitle">Add new</h2><form class="pgapp-form" method="post" action="/{}">"#,
        escape(&page.name)
    ));
    for field_name in &page.form {
        body.push_str(&input_for_field(page, field_name, None, resolved_choices, registry));
    }
    body.push_str(r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">Create</button></form>"#);

    layout(&page.name, chrome, &body)
}

pub fn edit_page(
    page: &RuntimePage,
    id: &str,
    row: &BTreeMap<String, Option<String>>,
    error: Option<&str>,
    chrome: Chrome,
    resolved_choices: &HashMap<String, Vec<(String, String)>>,
    registry: &item_types::Registry,
) -> String {
    let mut body = String::new();
    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }
    body.push_str(&format!(
        r#"<form class="pgapp-form" method="post" action="/{page}/{id}/update">"#,
        page = escape(&page.name),
        id = escape(id),
    ));
    for field_name in &page.form {
        let value = row.get(field_name).and_then(|v| v.as_deref());
        body.push_str(&input_for_field(page, field_name, value, resolved_choices, registry));
    }
    body.push_str(r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">Save</button></form>"#);
    body.push_str(&format!(
        r#"<p><a class="pgapp-link" href="/{}">Back to list</a></p>"#,
        escape(&page.name)
    ));

    layout(&format!("Edit {}", page.name), chrome, &body)
}

/// A read-only single-row view for `Detail` pages, selected via `?id=`.
pub fn detail_page(page: &RuntimePage, row: &BTreeMap<String, Option<String>>, chrome: Chrome) -> String {
    let entity = page
        .entity
        .as_ref()
        .expect("detail page always has an entity");

    let mut body = String::new();
    body.push_str(&items_html(&page.items, chrome.regions));
    body.push_str(r#"<table class="pgapp-table"><tbody>"#);
    for field in &entity.fields {
        let val = row.get(&field.name).and_then(|v| v.as_deref()).unwrap_or("");
        body.push_str(&format!(
            "<tr><th>{name}</th><td>{val}</td></tr>",
            name = escape(&field.name),
            val = escape(val),
        ));
    }
    body.push_str("</tbody></table>");

    layout(&page.name, chrome, &body)
}

/// A pure page-items page: no entity, no table/form, just `items`.
pub fn static_page(page: &RuntimePage, chrome: Chrome) -> String {
    let body = items_html(&page.items, chrome.regions);
    layout(&page.name, chrome, &body)
}

pub fn index_page(app_name: &str, pages: &[String], chrome: Chrome) -> String {
    let mut body = String::from(r#"<ul class="pgapp-list">"#);
    for p in pages {
        body.push_str(&format!(
            r#"<li><a class="pgapp-link" href="/{p}">{p}</a></li>"#,
            p = escape(p)
        ));
    }
    body.push_str("</ul>");
    layout(app_name, chrome, &body)
}
