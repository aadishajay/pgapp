//! Minimal, dependency-free HTML rendering. Every page is metadata-driven:
//! the field list comes from `RuntimePage`, not from a per-app template.

use crate::meta::RuntimePage;
use crate::model::FieldType;
use std::collections::BTreeMap;

const BASE_CSS: &str = r#"
body { font-family: system-ui, sans-serif; max-width: 720px; margin: 2rem auto; color: #222; }
table { border-collapse: collapse; width: 100%; margin-bottom: 1.5rem; }
th, td { border: 1px solid #ddd; padding: 0.4rem 0.6rem; text-align: left; }
th { background: #f5f5f5; }
form.inline { display: inline; }
.field { margin-bottom: 0.6rem; }
label { display: block; font-weight: 600; margin-bottom: 0.2rem; }
button, input[type=submit] { cursor: pointer; }
nav { margin-bottom: 1.5rem; }
"#;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Extra `<link>`/`<script>` tags for user-supplied assets, if present —
/// the pluggable css/js extension point.
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

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>{BASE_CSS}</style>
{assets}
</head>
<body>
<nav><a href="/">pgapp</a></nav>
<h1>{title}</h1>
{body}
</body>
</html>"#,
        title = escape(title),
        assets = asset_tags(),
        body = body,
    )
}

fn input_for_field(page: &RuntimePage, field_name: &str, value: Option<&str>) -> String {
    let field = page
        .entity
        .field(field_name)
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
                r#"<select name="{name}"><option value="true"{true_sel}>true</option><option value="false"{false_sel}>false</option></select>"#,
                name = escape(field_name),
            )
        }
        FieldType::Integer => format!(
            r#"<input type="number" name="{name}" value="{value}"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
        FieldType::Timestamp => format!(
            r#"<input type="text" name="{name}" value="{value}" placeholder="YYYY-MM-DD HH:MM:SS"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
        FieldType::Text | FieldType::Id => format!(
            r#"<input type="text" name="{name}" value="{value}"{required}>"#,
            name = escape(field_name),
            value = escape(value),
        ),
    };

    format!(
        r#"<div class="field"><label>{label}</label>{input}</div>"#,
        label = escape(field_name),
    )
}

pub fn list_page(
    page: &RuntimePage,
    rows: &[BTreeMap<String, Option<String>>],
    error: Option<&str>,
) -> String {
    let mut body = String::new();

    if let Some(err) = error {
        body.push_str(&format!(
            r#"<p style="color:#b00"><strong>Error:</strong> {}</p>"#,
            escape(err)
        ));
    }

    body.push_str("<table><thead><tr>");
    for col in &page.columns {
        body.push_str(&format!("<th>{}</th>", escape(col)));
    }
    body.push_str("<th></th></tr></thead><tbody>");

    for row in rows {
        body.push_str("<tr>");
        for col in &page.columns {
            let val = row.get(col).and_then(|v| v.as_deref()).unwrap_or("");
            body.push_str(&format!("<td>{}</td>", escape(val)));
        }
        let id = row.get("id").and_then(|v| v.as_deref()).unwrap_or("");
        body.push_str(&format!(
            r#"<td>
<a href="/{page}/{id}/edit">Edit</a>
<form class="inline" method="post" action="/{page}/{id}/delete" onsubmit="return confirm('Delete this row?')">
<button type="submit">Delete</button>
</form>
</td>"#,
            page = escape(&page.name),
            id = escape(id),
        ));
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table>");

    body.push_str(&format!(
        r#"<h2>Add new</h2><form method="post" action="/{}">"#,
        escape(&page.name)
    ));
    for field_name in &page.form {
        body.push_str(&input_for_field(page, field_name, None));
    }
    body.push_str(r#"<button type="submit">Create</button></form>"#);

    layout(&page.name, &body)
}

pub fn edit_page(
    page: &RuntimePage,
    id: &str,
    row: &BTreeMap<String, Option<String>>,
    error: Option<&str>,
) -> String {
    let mut body = String::new();
    if let Some(err) = error {
        body.push_str(&format!(
            r#"<p style="color:#b00"><strong>Error:</strong> {}</p>"#,
            escape(err)
        ));
    }
    body.push_str(&format!(
        r#"<form method="post" action="/{page}/{id}/update">"#,
        page = escape(&page.name),
        id = escape(id),
    ));
    for field_name in &page.form {
        let value = row.get(field_name).and_then(|v| v.as_deref());
        body.push_str(&input_for_field(page, field_name, value));
    }
    body.push_str(r#"<button type="submit">Save</button></form>"#);
    body.push_str(&format!(r#"<p><a href="/{}">Back to list</a></p>"#, escape(&page.name)));

    layout(&format!("Edit {}", page.name), &body)
}

pub fn index_page(app_name: &str, pages: &[String]) -> String {
    let mut body = String::from("<ul>");
    for p in pages {
        body.push_str(&format!(r#"<li><a href="/{p}">{p}</a></li>"#, p = escape(p)));
    }
    body.push_str("</ul>");
    layout(app_name, &body)
}
