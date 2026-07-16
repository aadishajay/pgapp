//! Upserts a parsed [`AppDef`] into `pgapp_meta.*` and makes sure the
//! physical data table for each entity exists.
//!
//! Syncing happens in phases because pages, page items, link columns
//! and nav items can all reference *other* pages by name, including
//! ones declared later in the file: phase 1 creates entities/fields/
//! tables, phase 2 creates a bare row for every page (so every page has
//! an id), and phase 3 onward resolves everything that points at a page
//! by name. Named queries don't need that phasing themselves (nothing
//! references a query by anything but its plain name, resolved at
//! request time), so they're synced right where they're declared.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;

use crate::item_types::{self, Registry};
use crate::model::{AppDef, EntityDef, FieldDef, FieldItem, FieldType, PageItem, PageKind, QueryDef};

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Seed value for `pgapp_meta.app_runtime_js`, written on an app's first
/// sync only — after that the database row is the one that's served.
const DEFAULT_RUNTIME_JS: &str = include_str!("../runtime.js");

/// Upserts the app/entity/field/page/item/nav/query metadata and makes
/// sure the physical data table for each entity exists. `registry` is
/// used only to validate that every `item ... as <kind>` names a
/// component that actually exists, so a typo (or a kind whose file was
/// never registered) fails at sync time with a clear message instead of
/// silently rendering nothing later.
pub async fn sync_app(pool: &PgPool, app: &AppDef, registry: &Registry) -> Result<()> {
    let app_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.apps (name) values ($1)
         on conflict (name) do update set name = excluded.name
         returning id",
    )
    .bind(&app.name)
    .fetch_one(pool)
    .await?;

    sync_queries(pool, app_id, None, &app.queries).await?;

    // Seed the runtime JS library on first sync only; once a row exists
    // it's the database's to edit, not this binary's to overwrite.
    sqlx::query(
        "insert into pgapp_meta.app_runtime_js (app_id, content) values ($1, $2)
         on conflict (app_id) do nothing",
    )
    .bind(app_id)
    .bind(DEFAULT_RUNTIME_JS)
    .execute(pool)
    .await?;

    // Phase 1: entities, fields, physical data tables.
    let mut entity_ids: HashMap<String, i32> = HashMap::new();
    for entity in &app.entities {
        let table_name = format!("{}_{}", slug(&app.name), slug(&entity.name));

        let entity_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.entities (app_id, name, table_name) values ($1, $2, $3)
             on conflict (app_id, name) do update set table_name = excluded.table_name
             returning id",
        )
        .bind(app_id)
        .bind(&entity.name)
        .bind(&table_name)
        .fetch_one(pool)
        .await?;

        for (ordinal, field) in entity.fields.iter().enumerate() {
            sqlx::query(
                "insert into pgapp_meta.fields
                    (entity_id, name, data_type, is_required, default_value, ordinal)
                 values ($1, $2, $3, $4, $5, $6)
                 on conflict (entity_id, name) do update set
                    data_type = excluded.data_type,
                    is_required = excluded.is_required,
                    default_value = excluded.default_value,
                    ordinal = excluded.ordinal",
            )
            .bind(entity_id)
            .bind(&field.name)
            .bind(field.ty.as_str())
            .bind(field.required)
            .bind(&field.default)
            .bind(ordinal as i32)
            .execute(pool)
            .await?;
        }

        ensure_data_table(pool, &table_name, entity).await?;
        entity_ids.insert(entity.name.clone(), entity_id);
    }

    // Phase 2: a bare row for every page, so every page has an id before
    // anything below tries to link to one by name.
    let mut page_ids: HashMap<String, i32> = HashMap::new();
    for page in &app.pages {
        let entity_id = match &page.entity {
            None => None,
            Some(name) => Some(*entity_ids.get(name).with_context(|| {
                format!("page '{}' references unknown entity '{name}'", page.name)
            })?),
        };

        let page_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.pages (app_id, entity_id, name, page_type)
             values ($1, $2, $3, $4)
             on conflict (app_id, name) do update set
                entity_id = excluded.entity_id,
                page_type = excluded.page_type
             returning id",
        )
        .bind(app_id)
        .bind(entity_id)
        .bind(&page.name)
        .bind(page.kind.as_str())
        .fetch_one(pool)
        .await?;

        page_ids.insert(page.name.clone(), page_id);
    }

    // Phase 3: page-scoped queries, page fields, field item types, link
    // columns, and page items — all of which may reference a page (or,
    // for link columns, a query) by name.
    for page in &app.pages {
        let page_id = page_ids[&page.name];

        sync_queries(pool, app_id, Some(page_id), &page.queries).await?;

        if page.kind == PageKind::List {
            let entity_name = page.entity.as_ref().expect("list page always has an entity");
            let entity_id = entity_ids[entity_name];
            let entity = app.entity(entity_name).expect("resolved above");

            for (ordinal, field_name) in entity.fields.iter().map(|f| &f.name).enumerate() {
                let shown_in_list = page.columns.iter().any(|c| c == field_name);
                let shown_in_form = page.form.iter().any(|c| c == field_name);
                sqlx::query(
                    "insert into pgapp_meta.page_fields
                        (page_id, field_id, shown_in_list, shown_in_form, ordinal)
                     select $1, f.id, $3, $4, $5
                       from pgapp_meta.fields f
                      where f.entity_id = $2 and f.name = $6
                     on conflict (page_id, field_id) do update set
                        shown_in_list = excluded.shown_in_list,
                        shown_in_form = excluded.shown_in_form,
                        ordinal = excluded.ordinal",
                )
                .bind(page_id)
                .bind(entity_id)
                .bind(shown_in_list)
                .bind(shown_in_form)
                .bind(ordinal as i32)
                .bind(field_name)
                .execute(pool)
                .await?;
            }

            // Every form field gets an explicit, resolved item (kind +
            // config), falling back to `item_types::default_kind_for`,
            // so the runtime side never has to re-derive the default
            // itself.
            for field_name in &page.form {
                let field = entity
                    .fields
                    .iter()
                    .find(|f| &f.name == field_name)
                    .with_context(|| format!("page '{}' form references unknown field '{field_name}'", page.name))?;
                let field_item = page.item_types.get(field_name).cloned().unwrap_or_else(|| FieldItem {
                    kind: item_types::default_kind_for(field.ty).to_string(),
                    config: serde_json::json!({}),
                });

                if !registry.contains_key(field_item.kind.as_str()) {
                    let known: Vec<&str> = registry.keys().copied().collect();
                    anyhow::bail!(
                        "page '{}' field '{field_name}' uses unknown item type '{}' (known: {})",
                        page.name,
                        field_item.kind,
                        known.join(", "),
                    );
                }

                sqlx::query(
                    "insert into pgapp_meta.page_field_items (page_id, field_name, item_type, config)
                     values ($1, $2, $3, $4)
                     on conflict (page_id, field_name) do update set
                        item_type = excluded.item_type,
                        config = excluded.config",
                )
                .bind(page_id)
                .bind(field_name)
                .bind(&field_item.kind)
                .bind(&field_item.config)
                .execute(pool)
                .await?;
            }
        }

        let (link_field, link_target_id, link_params) = match &page.link_column {
            None => (None, None, serde_json::Value::Array(Vec::new())),
            Some(lc) => {
                let target_id = *page_ids.get(&lc.target_page).with_context(|| {
                    format!(
                        "page '{}' links to unknown page '{}'",
                        page.name, lc.target_page
                    )
                })?;
                let params = lc
                    .extra_params
                    .iter()
                    .map(|(field, param)| serde_json::json!({ "field": field, "param": param }))
                    .collect();
                (
                    Some(lc.field.clone()),
                    Some(target_id),
                    serde_json::Value::Array(params),
                )
            }
        };
        sqlx::query(
            "update pgapp_meta.pages set
                link_field = $2, link_target_page_id = $3, link_params = $4, source_query_name = $5
             where id = $1",
        )
        .bind(page_id)
        .bind(link_field)
        .bind(link_target_id)
        .bind(link_params)
        .bind(&page.source_query)
        .execute(pool)
        .await?;

        sync_items(
            pool,
            "pgapp_meta.page_items",
            "page_id",
            page_id,
            &page.items,
            &page_ids,
            &format!("page '{}'", page.name),
        )
        .await?;
    }

    // Phase 4: the nav tree, which can also reference any page by name.
    sqlx::query("delete from pgapp_meta.nav_items where app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await?;
    for (ordinal, item) in app.nav.iter().enumerate() {
        sync_nav_item(pool, app_id, None, ordinal as i32, item, &page_ids).await?;
    }

    // Phase 5: the app-wide header/footer chrome.
    sync_items(
        pool,
        "pgapp_meta.header_items",
        "app_id",
        app_id,
        &app.header,
        &page_ids,
        "app header",
    )
    .await?;
    sync_items(
        pool,
        "pgapp_meta.footer_items",
        "app_id",
        app_id,
        &app.footer,
        &page_ids,
        "app footer",
    )
    .await?;

    Ok(())
}

/// Replaces the named queries owned by an app (`page_id` null) or one
/// page (`page_id` set).
async fn sync_queries(
    pool: &PgPool,
    app_id: i32,
    page_id: Option<i32>,
    queries: &[QueryDef],
) -> Result<()> {
    match page_id {
        Some(pid) => {
            sqlx::query("delete from pgapp_meta.named_queries where page_id = $1")
                .bind(pid)
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("delete from pgapp_meta.named_queries where app_id = $1 and page_id is null")
                .bind(app_id)
                .execute(pool)
                .await?;
        }
    }

    for q in queries {
        sqlx::query(
            "insert into pgapp_meta.named_queries (app_id, page_id, name, sql_text)
             values ($1, $2, $3, $4)",
        )
        .bind(app_id)
        .bind(page_id)
        .bind(&q.name)
        .bind(&q.sql)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Replaces the text/link/region items owned by one row (a page, or the
/// app itself for header/footer) — shared by `page_items`,
/// `header_items`, and `footer_items`, which only differ in table name
/// and owning column.
async fn sync_items(
    pool: &PgPool,
    table: &str,
    owner_col: &str,
    owner_id: i32,
    items: &[PageItem],
    page_ids: &HashMap<String, i32>,
    owner_label: &str,
) -> Result<()> {
    sqlx::query(&format!("delete from {table} where {owner_col} = $1"))
        .bind(owner_id)
        .execute(pool)
        .await?;

    for (ordinal, item) in items.iter().enumerate() {
        let (kind, label, target_id, query_name) = match item {
            PageItem::Text(text) => ("text", text.clone(), None, None),
            PageItem::Link { label, target_page } => {
                let target_id = *page_ids
                    .get(target_page)
                    .with_context(|| format!("{owner_label} links to unknown page '{target_page}'"))?;
                ("link", label.clone(), Some(target_id), None)
            }
            PageItem::Region { label, query } => ("region", label.clone(), None, Some(query.clone())),
        };
        sqlx::query(&format!(
            "insert into {table} ({owner_col}, kind, label, target_page_id, query_name, ordinal)
             values ($1, $2, $3, $4, $5, $6)"
        ))
        .bind(owner_id)
        .bind(kind)
        .bind(label)
        .bind(target_id)
        .bind(query_name)
        .bind(ordinal as i32)
        .execute(pool)
        .await?;
    }

    Ok(())
}

fn sync_nav_item<'a>(
    pool: &'a PgPool,
    app_id: i32,
    parent_id: Option<i32>,
    ordinal: i32,
    item: &'a crate::model::NavItem,
    page_ids: &'a HashMap<String, i32>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let target_id = match &item.target_page {
            None => None,
            Some(name) => Some(*page_ids.get(name).with_context(|| {
                format!("nav item '{}' links to unknown page '{name}'", item.label)
            })?),
        };

        let nav_id: i32 = sqlx::query_scalar(
            "insert into pgapp_meta.nav_items (app_id, parent_id, label, target_page_id, ordinal)
             values ($1, $2, $3, $4, $5)
             returning id",
        )
        .bind(app_id)
        .bind(parent_id)
        .bind(&item.label)
        .bind(target_id)
        .bind(ordinal)
        .fetch_one(pool)
        .await?;

        for (child_ordinal, child) in item.children.iter().enumerate() {
            sync_nav_item(pool, app_id, Some(nav_id), child_ordinal as i32, child, page_ids).await?;
        }

        Ok(())
    })
}

async fn ensure_data_table(pool: &PgPool, table_name: &str, entity: &EntityDef) -> Result<()> {
    let cols: Vec<String> = entity.fields.iter().map(|f| column_def(f, true)).collect();

    let sql = format!(
        "create table if not exists pgapp_data.{table_name} ({})",
        cols.join(", ")
    );
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to create data table pgapp_data.{table_name}"))?;

    // The table may already have existed before a field was added to
    // the entity — CREATE TABLE IF NOT EXISTS doesn't add columns to an
    // existing table, so bring existing tables up to date too. Skipping
    // NOT NULL here: enforcing it on a table that may already have rows
    // is a real migration (backfill or a default) that this vertical
    // slice doesn't attempt.
    for field in &entity.fields {
        if field.ty == FieldType::Id {
            continue; // the primary key only ever comes from CREATE TABLE
        }
        let alter_sql = format!(
            "alter table pgapp_data.{table_name} add column if not exists {}",
            column_def(field, false)
        );
        sqlx::raw_sql(&alter_sql)
            .execute(pool)
            .await
            .with_context(|| {
                format!("failed to add column '{}' to pgapp_data.{table_name}", field.name)
            })?;
    }

    Ok(())
}

fn column_def(field: &FieldDef, include_not_null: bool) -> String {
    let mut col = format!("{} {}", field.name, field.ty.sql_column_type());
    if field.ty != FieldType::Id {
        if include_not_null && field.required {
            col.push_str(" not null");
        }
        if let Some(default) = &field.default {
            match field.ty {
                FieldType::Boolean => col.push_str(&format!(" default {default}")),
                FieldType::Timestamp if default == "now" => col.push_str(" default now()"),
                FieldType::Integer => col.push_str(&format!(" default {default}")),
                _ => col.push_str(&format!(" default '{default}'")),
            }
        }
    }
    col
}
