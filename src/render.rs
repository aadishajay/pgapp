//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! the field/item/nav lists come from `RuntimePage`/`RuntimeApp`, not
//! from a per-app template.
//!
//! Markup here only ever uses the fixed `.pgapp-*` class names — the
//! "Theme contract" documented in the README. All actual look-and-feel
//! comes from `/theme.css` (the active theme, see src/theme.rs) plus any
//! app-level override in assets/app.css.

use crate::meta::{NavNode, RuntimePage, RuntimePageItem};
use crate::model::FieldType;
use std::collections::BTreeMap;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

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

/// Renders a page's `items` list (static text and links to other pages).
fn items_html(items: &[RuntimePageItem]) -> String {
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
        }
    }
    html.push_str("</div>");
    html
}

fn layout(title: &str, nav: &[NavNode], body: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<link rel="stylesheet" href="/theme.css">
{assets}
</head>
<body>
<nav class="pgapp-nav"><a class="pgapp-link" href="/">pgapp</a>{navbar}</nav>
<h1 class="pgapp-title">{title}</h1>
{body}
</body>
</html>"#,
        title = escape(title),
        assets = asset_tags(),
        navbar = nav_html(nav),
        body = body,
    )
}

fn input_for_field(page: &RuntimePage, field_name: &str, value: Option<&str>) -> String {
    let field = page
        .entity
        .as_ref()
        .and_then(|e| e.field(field_name))
        .expect("form field must exist on entity");
    let value = value.unwrap_or("");
    let required = if field.required { " required" } else { "" };

    let input = match field.data_type {
        FieldType::Boolean => {
            let (true_sel, false_sel) = if value == "true" {
                (" selected", "")
            } else {
                ("", " selected")
            };
            format!(
                r#"<select class="pgapp-select" name="{name}"><option value="true"{true_sel}>true</option><option value="false"{false_sel}>false</option></select>"#,
                name = escape(field_name),
            )
        }
        FieldType::Integer => format!(
            r#"<input class="pgapp-input" type="number" name="{name}" value="{value}"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
        FieldType::Timestamp => format!(
            r#"<input class="pgapp-input" type="text" name="{name}" value="{value}" placeholder="YYYY-MM-DD HH:MM:SS"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
        FieldType::Text | FieldType::Id => format!(
            r#"<input class="pgapp-input" type="text" name="{name}" value="{value}"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
    };

    format!(
        r#"<div class="pgapp-field"><label class="pgapp-label">{label}</label>{input}</div>"#,
        label = escape(field_name),
    )
}

pub fn list_page(
    page: &RuntimePage,
    rows: &[BTreeMap<String, Option<String>>],
    error: Option<&str>,
    nav: &[NavNode],
) -> String {
    let mut body = String::new();

    if let Some(err) = error {
        body.push_str(&format!(
            r#"<div class="pgapp-alert pgapp-alert-error"><strong>Error:</strong> {}</div>"#,
            escape(err)
        ));
    }

    body.push_str(&items_html(&page.items));

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
                Some(lc) if lc.field == *col => format!(
                    r#"<a class="pgapp-link" href="/{target}?id={id}">{val}</a>"#,
                    target = escape(&lc.target_page),
                    id = escape(id),
                    val = escape(val),
                ),
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
        body.push_str(&input_for_field(page, field_name, None));
    }
    body.push_str(r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">Create</button></form>"#);

    layout(&page.name, nav, &body)
}

pub fn edit_page(
    page: &RuntimePage,
    id: &str,
    row: &BTreeMap<String, Option<String>>,
    error: Option<&str>,
    nav: &[NavNode],
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
        body.push_str(&input_for_field(page, field_name, value));
    }
    body.push_str(r#"<button class="pgapp-btn pgapp-btn-primary" type="submit">Save</button></form>"#);
    body.push_str(&format!(
        r#"<p><a class="pgapp-link" href="/{}">Back to list</a></p>"#,
        escape(&page.name)
    ));

    layout(&format!("Edit {}", page.name), nav, &body)
}

/// A read-only single-row view for `Detail` pages, selected via `?id=`.
pub fn detail_page(page: &RuntimePage, row: &BTreeMap<String, Option<String>>, nav: &[NavNode]) -> String {
    let entity = page
        .entity
        .as_ref()
        .expect("detail page always has an entity");

    let mut body = String::new();
    body.push_str(&items_html(&page.items));
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

    layout(&page.name, nav, &body)
}

/// A pure page-items page: no entity, no table/form, just `items`.
pub fn static_page(page: &RuntimePage, nav: &[NavNode]) -> String {
    let body = items_html(&page.items);
    layout(&page.name, nav, &body)
}

pub fn index_page(app_name: &str, pages: &[String], nav: &[NavNode]) -> String {
    let mut body = String::from(r#"<ul class="pgapp-list">"#);
    for p in pages {
        body.push_str(&format!(
            r#"<li><a class="pgapp-link" href="/{p}">{p}</a></li>"#,
            p = escape(p)
        ));
    }
    body.push_str("</ul>");
    layout(app_name, nav, &body)
}
