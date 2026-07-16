//! Reloads a [`RuntimeApp`](super::RuntimeApp) straight from
//! `pgapp_meta`, proving the database (not the parsed markup struct) is
//! the authority once the server starts handling requests.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;

use super::types::{
    LinkColumn, NavNode, RuntimeApp, RuntimeEntity, RuntimeField, RuntimePage, RuntimePageItem,
    RuntimeQuery,
};
use crate::model::{FieldItem, FieldType, PageKind};

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

#[derive(sqlx::FromRow)]
struct PageRow {
    id: i32,
    name: String,
    entity_id: Option<i32>,
    page_type: String,
    link_field: Option<String>,
    link_target_page_id: Option<i32>,
    source_query_name: Option<String>,
    link_params: serde_json::Value,
}

pub async fn load_app(pool: &PgPool, app_name: &str) -> Result<RuntimeApp> {
    let app_id: i32 = sqlx::query_scalar("select id from pgapp_meta.apps where name = $1")
        .bind(app_name)
        .fetch_one(pool)
        .await
        .with_context(|| format!("app '{app_name}' not found in pgapp_meta"))?;

    let page_rows: Vec<PageRow> = sqlx::query_as(
        "select id, name, entity_id, page_type, link_field, link_target_page_id,
                source_query_name, link_params
           from pgapp_meta.pages where app_id = $1 order by id",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let page_names: HashMap<i32, String> = page_rows.iter().map(|r| (r.id, r.name.clone())).collect();

    let (app_queries, mut page_queries) = load_queries(pool, app_id).await?;

    let mut pages = Vec::new();
    for row in &page_rows {
        let entity = match row.entity_id {
            None => None,
            Some(entity_id) => {
                let (entity_name, table_name): (String, String) = sqlx::query_as(
                    "select name, table_name from pgapp_meta.entities where id = $1",
                )
                .bind(entity_id)
                .fetch_one(pool)
                .await?;

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

                Some(RuntimeEntity {
                    name: entity_name,
                    table_name,
                    fields,
                })
            }
        };

        let (columns, form) = if entity.is_some() {
            let pf_rows: Vec<(String, bool, bool)> = sqlx::query_as(
                "select f.name, pf.shown_in_list, pf.shown_in_form
                   from pgapp_meta.page_fields pf
                   join pgapp_meta.fields f on f.id = pf.field_id
                  where pf.page_id = $1
                  order by pf.ordinal",
            )
            .bind(row.id)
            .fetch_all(pool)
            .await?;

            let columns = pf_rows
                .iter()
                .filter(|(_, shown_in_list, _)| *shown_in_list)
                .map(|(name, ..)| name.clone())
                .collect();
            let form = pf_rows
                .iter()
                .filter(|(_, _, shown_in_form)| *shown_in_form)
                .map(|(name, ..)| name.clone())
                .collect();
            (columns, form)
        } else {
            (Vec::new(), Vec::new())
        };

        let link_column = match (&row.link_field, row.link_target_page_id) {
            (Some(field), Some(target_id)) => {
                let extra_params = row
                    .link_params
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| {
                        let field = v.get("field")?.as_str()?.to_string();
                        let param = v.get("param")?.as_str()?.to_string();
                        Some((field, param))
                    })
                    .collect();
                Some(LinkColumn {
                    field: field.clone(),
                    target_page: page_names[&target_id].clone(),
                    extra_params,
                })
            }
            _ => None,
        };

        let item_type_rows: Vec<(String, String, serde_json::Value)> = sqlx::query_as(
            "select field_name, item_type, config from pgapp_meta.page_field_items
              where page_id = $1",
        )
        .bind(row.id)
        .fetch_all(pool)
        .await?;
        let item_types = item_type_rows
            .into_iter()
            .map(|(field_name, kind, config)| (field_name, FieldItem { kind, config }))
            .collect();

        let items = load_items(pool, "pgapp_meta.page_items", "page_id", row.id, &page_names).await?;

        pages.push(RuntimePage {
            name: row.name.clone(),
            kind: PageKind::from_str_lossy(&row.page_type),
            entity,
            columns,
            form,
            link_column,
            items,
            item_types,
            source_query: row.source_query_name.clone(),
            queries: page_queries.remove(&row.id).unwrap_or_default(),
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

    let header = load_items(pool, "pgapp_meta.header_items", "app_id", app_id, &page_names).await?;
    let footer = load_items(pool, "pgapp_meta.footer_items", "app_id", app_id, &page_names).await?;

    Ok(RuntimeApp {
        name: app_name.to_string(),
        pages,
        nav,
        header,
        footer,
        queries: app_queries,
    })
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

/// Loads the text/link/region items owned by one row — shared by
/// `page_items`, `header_items`, and `footer_items`.
async fn load_items(
    pool: &PgPool,
    table: &str,
    owner_col: &str,
    owner_id: i32,
    page_names: &HashMap<i32, String>,
) -> Result<Vec<RuntimePageItem>> {
    let rows: Vec<(String, String, Option<i32>, Option<String>)> = sqlx::query_as(&format!(
        "select kind, label, target_page_id, query_name from {table}
          where {owner_col} = $1 order by ordinal"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(kind, label, target_page_id, query_name)| match kind.as_str() {
            "link" => RuntimePageItem::Link {
                label,
                target_page: page_names[&target_page_id.expect("link items always have a target page")].clone(),
            },
            "region" => RuntimePageItem::Region {
                label,
                query: query_name.expect("region items always have a query name"),
            },
            _ => RuntimePageItem::Text(label),
        })
        .collect())
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
