//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! the field/item/nav lists come from `RuntimePage`/`RuntimeApp`, not
//! from a per-app template.
//!
//! Markup here only ever uses the fixed `.pgapp-*` class names — the
//! "Theme contract" documented in the README. All actual look-and-feel
//! comes from `/theme.css` (the active theme, see src/theme.rs) plus any
//! app-level override in assets/app.css.

use crate::meta::{Chrome, NavNode, RuntimePage, RuntimePageItem};
use crate::model::{FieldItemType, FieldType};
use std::collections::BTreeMap;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Escapes a string for embedding inside a single-quoted JS string
/// literal. Callers must still HTML-escape the *result* before splicing
/// it into an HTML attribute (see `render_popup`).
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
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

/// Renders an item list (page `items`, or the app's header/footer) —
/// static text and links to other pages.
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

fn layout(title: &str, chrome: Chrome, body: &str) -> String {
    let header = if chrome.header.is_empty() {
        String::new()
    } else {
        format!(
            r#"<header class="pgapp-header">{}</header>"#,
            items_html(chrome.header)
        )
    };
    let footer = if chrome.footer.is_empty() {
        String::new()
    } else {
        format!(
            r#"<footer class="pgapp-footer">{}</footer>"#,
            items_html(chrome.footer)
        )
    };

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

/// Renders a "Pop Up LOV": a hidden input holding the actual value, a
/// button showing the current choice, and a native `<dialog>` listing
/// every choice — no JS framework, just `showModal()`/`close()`.
fn render_popup(field_name: &str, value: &str, choices: &[String]) -> String {
    let name = escape(field_name);
    let dialog_id = format!("pgapp-popup-dialog-{name}");
    let value_id = format!("pgapp-popup-value-{name}");
    let display_id = format!("pgapp-popup-display-{name}");

    let mut html = format!(
        r#"<div class="pgapp-popup">
<input type="hidden" id="{value_id}" name="{name}" value="{value}">
<button type="button" class="pgapp-btn pgapp-btn-secondary" onclick="document.getElementById('{dialog_id}').showModal()">
<span id="{display_id}">{display}</span>
</button>
<dialog id="{dialog_id}" class="pgapp-popup-dialog">
<ul class="pgapp-popup-list">"#,
        value = escape(value),
        display = if value.is_empty() {
            "Choose\u{2026}".to_string()
        } else {
            escape(value)
        },
    );

    for choice in choices {
        // JS-escape first (protects the single-quoted JS string), then
        // HTML-escape the result (protects the double-quoted attribute).
        let js_choice = escape(&js_escape(choice));
        html.push_str(&format!(
            r#"<li><button type="button" onclick="document.getElementById('{value_id}').value='{js_choice}'; document.getElementById('{display_id}').textContent='{js_choice}'; document.getElementById('{dialog_id}').close();">{label}</button></li>"#,
            label = escape(choice),
        ));
    }

    html.push_str(&format!(
        r#"</ul><button type="button" class="pgapp-btn" onclick="document.getElementById('{dialog_id}').close()">Cancel</button></dialog></div>"#
    ));
    html
}

fn input_for_field(page: &RuntimePage, field_name: &str, value: Option<&str>) -> String {
    let field = page
        .entity
        .as_ref()
        .and_then(|e| e.field(field_name))
        .expect("form field must exist on entity");
    let value = value.unwrap_or("");
    let required = if field.required { " required" } else { "" };
    let item_type = page.item_types.get(field_name).unwrap_or(&FieldItemType::Text);

    let input = match item_type {
        FieldItemType::Checkbox => {
            let checked = if value == "true" { " checked" } else { "" };
            format!(
                r#"<input class="pgapp-checkbox" type="checkbox" name="{name}" value="true"{checked}>"#,
                name = escape(field_name),
            )
        }
        FieldItemType::ReadOnly => format!(
            r#"<span class="pgapp-readonly">{val}</span><input type="hidden" name="{name}" value="{val}">"#,
            name = escape(field_name),
            val = escape(value),
        ),
        FieldItemType::Radio(choices) => {
            let mut html = String::from(r#"<div class="pgapp-radio-group">"#);
            for choice in choices {
                let checked = if choice == value { " checked" } else { "" };
                html.push_str(&format!(
                    r#"<label class="pgapp-radio-option"><input type="radio" name="{name}" value="{choice}"{checked}> {choice}</label>"#,
                    name = escape(field_name),
                    choice = escape(choice),
                ));
            }
            html.push_str("</div>");
            html
        }
        FieldItemType::Popup(choices) => render_popup(field_name, value, choices),
        FieldItemType::Text => match field.data_type {
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
            FieldType::Text | FieldType::Id | FieldType::Boolean => format!(
                r#"<input class="pgapp-input" type="text" name="{name}" value="{value}"{required}>"#,
                name = escape(field_name),
                value = escape(value),
            ),
        },
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
    chrome: Chrome,
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

    layout(&page.name, chrome, &body)
}

pub fn edit_page(
    page: &RuntimePage,
    id: &str,
    row: &BTreeMap<String, Option<String>>,
    error: Option<&str>,
    chrome: Chrome,
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

    layout(&format!("Edit {}", page.name), chrome, &body)
}

/// A read-only single-row view for `Detail` pages, selected via `?id=`.
pub fn detail_page(page: &RuntimePage, row: &BTreeMap<String, Option<String>>, chrome: Chrome) -> String {
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

    layout(&page.name, chrome, &body)
}

/// A pure page-items page: no entity, no table/form, just `items`.
pub fn static_page(page: &RuntimePage, chrome: Chrome) -> String {
    let body = items_html(&page.items);
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
