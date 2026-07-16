//! Reloads a [`RuntimeApp`](super::RuntimeApp) straight from
//! `pgapp_meta`, proving the database (not the parsed markup struct) is
//! the authority once the server starts handling requests.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;

use super::types::{
    LinkColumn, NavNode, RuntimeApp, RuntimeComponent, RuntimeEntity, RuntimeField, RuntimePage,
    RuntimeQuery,
};
use crate::model::{FieldItem, FieldType};

/// Turns `sql` (which may contain `:name` bind markers) into Postgres
/// positional-parameter SQL plus the ordered list of names each `$N`
/// stands for. Every substitution is `$N::text` — bind values always
/// arrive as text (see `server::query_engine::run_named_query`), so a
/// query comparing against a non-text column needs its own trailing
/// cast, e.g. `where project_id = :project_id::integer`.
///
/// A literal `::` (Postgres's own cast operator) is left untouched, so
/// existing casts in hand-written SQL are never mistaken for a bind
/// marker.
pub fn compile_named_query(sql: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::new();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' && chars.get(i + 1) == Some(&':') {
            out.push_str("::");
            i += 2;
        } else if c == ':' && chars.get(i + 1).is_some_and(|c| c.is_alphabetic() || *c == '_') {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            let idx = names.iter().position(|n| n == &name).unwrap_or_else(|| {
                names.push(name.clone());
                names.len() - 1
            });
            out.push_str(&format!("${}::text", idx + 1));
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    (out, names)
}

/// Loads the current `runtime.js` content for `app_name` — whatever's in
/// `pgapp_meta.app_runtime_js` now, not necessarily the built-in default
/// (the row is only seeded once; edits to it afterward stick).
pub async fn load_runtime_js(pool: &PgPool, app_name: &str) -> Result<String> {
    sqlx::query_scalar(
        "select r.content from pgapp_meta.app_runtime_js r
           join pgapp_meta.apps a on a.id = r.app_id
          where a.name = $1",
    )
    .bind(app_name)
    .fetch_one(pool)
    .await
    .with_context(|| format!("runtime.js not found for app '{app_name}'"))
}

pub async fn load_app(pool: &PgPool, app_name: &str) -> Result<RuntimeApp> {
    let app_id: i32 = sqlx::query_scalar("select id from pgapp_meta.apps where name = $1")
        .bind(app_name)
        .fetch_one(pool)
        .await
        .with_context(|| format!("app '{app_name}' not found in pgapp_meta"))?;

    let entities = load_entities(pool, app_id).await?;

    let page_rows: Vec<(i32, String)> = sqlx::query_as(
        "select id, name from pgapp_meta.pages where app_id = $1 order by id",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;
    let page_names: HashMap<i32, String> = page_rows.iter().cloned().collect();

    let (app_queries, mut page_queries) = load_queries(pool, app_id).await?;

    let mut pages = Vec::new();
    for (page_id, name) in &page_rows {
        let components = load_components(pool, "page_id", *page_id, &entities).await?;
        pages.push(RuntimePage {
            name: name.clone(),
            components,
            queries: page_queries.remove(page_id).unwrap_or_default(),
        });
    }

    let nav_rows: Vec<(i32, Option<i32>, String, Option<i32>)> = sqlx::query_as(
        "select id, parent_id, label, target_page_id
           from pgapp_meta.nav_items where app_id = $1 order by ordinal",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;
    let nav = build_nav_tree(&nav_rows, None, &page_names);

    let header = load_chrome(pool, app_id, "header", &entities).await?;
    let footer = load_chrome(pool, app_id, "footer", &entities).await?;

    Ok(RuntimeApp {
        name: app_name.to_string(),
        pages,
        nav,
        header,
        footer,
        queries: app_queries,
    })
}

/// Loads every entity for the app, keyed by name, so components can
/// resolve their `entity` config field into a full [`RuntimeEntity`]
/// without another round trip per component.
async fn load_entities(pool: &PgPool, app_id: i32) -> Result<HashMap<String, RuntimeEntity>> {
    let entity_rows: Vec<(i32, String, String)> = sqlx::query_as(
        "select id, name, table_name from pgapp_meta.entities where app_id = $1",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut entities = HashMap::new();
    for (entity_id, name, table_name) in entity_rows {
        let field_rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select name, data_type, is_required from pgapp_meta.fields
              where entity_id = $1 order by ordinal",
        )
        .bind(entity_id)
        .fetch_all(pool)
        .await?;

        let fields = field_rows
            .into_iter()
            .map(|(name, data_type, required)| RuntimeField {
                name,
                data_type: FieldType::from_str_lossy(&data_type),
                required,
            })
            .collect();

        entities.insert(name.clone(), RuntimeEntity { name, table_name, fields });
    }
    Ok(entities)
}

/// Loads and compiles every named query for the app, split into the
/// app-scoped map and a page id -> (name -> query) map for page-scoped
/// ones.
async fn load_queries(
    pool: &PgPool,
    app_id: i32,
) -> Result<(HashMap<String, RuntimeQuery>, HashMap<i32, HashMap<String, RuntimeQuery>>)> {
    let rows: Vec<(Option<i32>, String, String)> = sqlx::query_as(
        "select page_id, name, sql_text from pgapp_meta.named_queries where app_id = $1",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut app_queries = HashMap::new();
    let mut page_queries: HashMap<i32, HashMap<String, RuntimeQuery>> = HashMap::new();
    for (page_id, name, sql_text) in rows {
        let (sql, bind_names) = compile_named_query(&sql_text);
        let rq = RuntimeQuery { sql, bind_names };
        match page_id {
            Some(pid) => {
                page_queries.entry(pid).or_default().insert(name, rq);
            }
            None => {
                app_queries.insert(name, rq);
            }
        }
    }
    Ok((app_queries, page_queries))
}

/// Loads the components owned by one page, in order.
async fn load_components(
    pool: &PgPool,
    owner_col: &str,
    owner_id: i32,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<Vec<RuntimeComponent>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(&format!(
        "select kind, config from pgapp_meta.components
          where {owner_col} = $1 and slot is null order by ordinal"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(kind, config)| decode_component(&kind, config, entities))
        .collect()
}

/// Loads the app-wide header/footer chrome (`slot` = "header"/"footer",
/// `page_id` null) — restricted to Text/Link/Region at sync time, but
/// decoded through the same generic path as page components.
async fn load_chrome(
    pool: &PgPool,
    app_id: i32,
    slot: &str,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<Vec<RuntimeComponent>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "select kind, config from pgapp_meta.components
          where app_id = $1 and slot = $2 and page_id is null order by ordinal",
    )
    .bind(app_id)
    .bind(slot)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(kind, config)| decode_component(&kind, config, entities))
        .collect()
}

fn json_strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

fn decode_item_types(v: &serde_json::Value) -> HashMap<String, FieldItem> {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| {
            let kind = val.get("kind").and_then(|x| x.as_str()).unwrap_or("text").to_string();
            let config = val.get("config").cloned().unwrap_or(serde_json::json!({}));
            (k.clone(), FieldItem { kind, config })
        })
        .collect()
}

fn resolve_entity(
    entities: &HashMap<String, RuntimeEntity>,
    name: &str,
) -> Result<RuntimeEntity> {
    entities
        .get(name)
        .cloned()
        .with_context(|| format!("component references unknown entity '{name}'"))
}

/// Decodes one `(kind, config)` row back into a [`RuntimeComponent`].
/// The inverse of `meta::sync::build_component_config`.
fn decode_component(
    kind: &str,
    config: serde_json::Value,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<RuntimeComponent> {
    match kind {
        "report" => {
            let entity = resolve_entity(entities, &json_str(&config, "entity"))?;
            let link_column = match config.get("link") {
                Some(v) if !v.is_null() => {
                    let extra_params = v
                        .get("extra_params")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|pair| {
                            let arr = pair.as_array()?;
                            Some((arr.first()?.as_str()?.to_string(), arr.get(1)?.as_str()?.to_string()))
                        })
                        .collect();
                    Some(LinkColumn {
                        field: json_str(v, "field"),
                        target_page: json_str(v, "target_page"),
                        extra_params,
                    })
                }
                _ => None,
            };
            Ok(RuntimeComponent::Report {
                title: json_str(&config, "title"),
                entity,
                columns: json_strings(&config["columns"]),
                source_query: config.get("source_query").and_then(|v| v.as_str()).map(String::from),
                link_column,
                page_size: config.get("page_size").and_then(|v| v.as_i64()).unwrap_or(20),
            })
        }
        "form" => Ok(RuntimeComponent::Form {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            fields: json_strings(&config["fields"]),
            item_types: decode_item_types(&config["item_types"]),
        }),
        "editable_table" => Ok(RuntimeComponent::EditableTable {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            columns: json_strings(&config["columns"]),
            item_types: decode_item_types(&config["item_types"]),
        }),
        "chart" => Ok(RuntimeComponent::Chart {
            title: json_str(&config, "title"),
            query: json_str(&config, "query"),
            chart_type: json_str(&config, "chart_type"),
            x: json_str(&config, "x"),
            y: json_str(&config, "y"),
        }),
        "text" => Ok(RuntimeComponent::Text(json_str(&config, "text"))),
        "link" => Ok(RuntimeComponent::Link {
            label: json_str(&config, "label"),
            target_page: json_str(&config, "target_page"),
        }),
        "region" => Ok(RuntimeComponent::Region {
            label: json_str(&config, "label"),
            query: json_str(&config, "query"),
        }),
        other => anyhow::bail!("unknown component kind '{other}' in pgapp_meta.components"),
    }
}

fn build_nav_tree(
    rows: &[(i32, Option<i32>, String, Option<i32>)],
    parent: Option<i32>,
    page_names: &HashMap<i32, String>,
) -> Vec<NavNode> {
    rows.iter()
        .filter(|(_, p, ..)| *p == parent)
        .map(|(id, _, label, target_page_id)| NavNode {
            label: label.clone(),
            target_page: target_page_id.map(|tid| page_names[&tid].clone()),
            children: build_nav_tree(rows, Some(*id), page_names),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_named_query_bind_markers() {
        let (sql, names) = compile_named_query(
            "select id as value, title as label from pgapp_data.demo_tasks
             where project_id = :project_id::integer and status = :status",
        );
        assert_eq!(names, vec!["project_id".to_string(), "status".to_string()]);
        assert!(sql.contains("project_id = $1::text::integer"));
        assert!(sql.contains("status = $2::text"));
    }

    #[test]
    fn compile_named_query_dedupes_repeated_names_and_preserves_casts() {
        let (sql, names) = compile_named_query(
            "select * from t where a = :id::integer or b = :id::integer or c::text = 'x'",
        );
        assert_eq!(names, vec!["id".to_string()]);
        assert_eq!(sql.matches("$1::text::integer").count(), 2);
        assert!(sql.contains("c::text"));
    }
}
