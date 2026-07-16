//! Syncs a parsed [`AppDef`] into the in-database metadata tables
//! (`pgapp_meta.*`), creates the physical data tables that back each
//! entity, and reloads a [`RuntimeApp`] straight from that metadata.
//!
//! The metadata tables — not the markup file — are the source of truth
//! once the server is running: `load_app` re-derives everything the
//! server needs (table names, column types, links, nav, item widgets,
//! named queries) from `pgapp_meta`.
//!
//! Syncing happens in phases because pages, page items, link columns and
//! nav items can all reference *other* pages by name, including ones
//! declared later in the file: phase 1 creates entities/fields/tables,
//! phase 2 creates a bare row for every page (so every page has an id),
//! and phase 3 onward resolves everything that points at a page by name.
//! Named queries don't need that phasing themselves (nothing references
//! a query by anything but its plain name, resolved at request time),
//! so they're synced right where they're declared.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap};

use crate::model::{AppDef, ChoiceSource, FieldItemType, FieldType, PageItem, PageKind, QueryDef};

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
const DEFAULT_RUNTIME_JS: &str = include_str!("runtime.js");

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_meta schema")?;
    Ok(())
}

/// Upserts the app/entity/field/page/item/nav/query metadata and makes
/// sure the physical data table for each entity exists.
pub async fn sync_app(pool: &PgPool, app: &AppDef) -> Result<()> {
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

            // Every form field gets an explicit, resolved item type
            // (falling back to FieldItemType::default_for), so the
            // runtime side never has to re-derive the default itself.
            for field_name in &page.form {
                let field = entity
                    .fields
                    .iter()
                    .find(|f| &f.name == field_name)
                    .with_context(|| format!("page '{}' form references unknown field '{field_name}'", page.name))?;
                let item_type = page
                    .item_types
                    .get(field_name)
                    .cloned()
                    .unwrap_or_else(|| FieldItemType::default_for(field.ty));

                let (choices, choices_query): (Vec<String>, Option<String>) =
                    match item_type.choice_source() {
                        Some(ChoiceSource::Static(list)) => (list.clone(), None),
                        Some(ChoiceSource::Query(name)) => (Vec::new(), Some(name.clone())),
                        None => (Vec::new(), None),
                    };

                sqlx::query(
                    "insert into pgapp_meta.page_field_items
                        (page_id, field_name, item_type, choices, choices_query)
                     values ($1, $2, $3, $4, $5)
                     on conflict (page_id, field_name) do update set
                        item_type = excluded.item_type,
                        choices = excluded.choices,
                        choices_query = excluded.choices_query",
                )
                .bind(page_id)
                .bind(field_name)
                .bind(item_type.kind_str())
                .bind(&choices)
                .bind(&choices_query)
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

async fn ensure_data_table(
    pool: &PgPool,
    table_name: &str,
    entity: &crate::model::EntityDef,
) -> Result<()> {
    let cols: Vec<String> = entity
        .fields
        .iter()
        .map(|f| column_def(f, true))
        .collect();

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

fn column_def(field: &crate::model::FieldDef, include_not_null: bool) -> String {
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

/// Turns `sql` (which may contain `:name` bind markers) into Postgres
/// positional-parameter SQL plus the ordered list of names each `$N`
/// stands for. Every substitution is `$N::text` — bind values always
/// arrive as text (see `server::run_named_query`), so a query comparing
/// against a non-text column needs its own trailing cast, e.g.
/// `where project_id = :project_id::integer`.
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

/// A named query, compiled once at load time: `sql` already uses
/// positional `$N::text` parameters, and `bind_names[i]` is the bind
/// context key that fills `$(i + 1)`.
#[derive(Debug, Clone)]
pub struct RuntimeQuery {
    pub sql: String,
    pub bind_names: Vec<String>,
}

/// Runtime view of a field, as reloaded from `pgapp_meta` (not from the
/// markup file) — this is what the server uses to build SQL.
#[derive(Debug, Clone)]
pub struct RuntimeField {
    pub name: String,
    pub data_type: FieldType,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeEntity {
    pub name: String,
    pub table_name: String,
    pub fields: Vec<RuntimeField>,
}

impl RuntimeEntity {
    pub fn field(&self, name: &str) -> Option<&RuntimeField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A page item, reloaded from `pgapp_meta.page_items` (or the
/// header/footer equivalents). `Link` targets are resolved back to the
/// target page's *name* (not its id) so rendering never needs another
/// database round trip; `Region` carries the query's name, resolved
/// against the page/app query registry at render time.
#[derive(Debug, Clone)]
pub enum RuntimePageItem {
    Text(String),
    Link { label: String, target_page: String },
    Region { label: String, query: String },
}

#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
    pub extra_params: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct RuntimePage {
    pub name: String,
    pub kind: PageKind,
    pub entity: Option<RuntimeEntity>,
    pub columns: Vec<String>,
    pub form: Vec<String>,
    pub link_column: Option<LinkColumn>,
    pub items: Vec<RuntimePageItem>,
    /// Resolved item type for every field in `form` (never missing —
    /// see the phase-3 sync above).
    pub item_types: HashMap<String, FieldItemType>,
    /// Overrides the page's row source with a named query; see
    /// `model::PageDef::source_query`.
    pub source_query: Option<String>,
    /// Queries visible only on this page.
    pub queries: HashMap<String, RuntimeQuery>,
}

impl RuntimePage {
    /// Looks up a query by name, preferring one scoped to this page over
    /// an app-scoped query of the same name.
    pub fn resolve_query<'a>(&'a self, app: &'a RuntimeApp, name: &str) -> Option<&'a RuntimeQuery> {
        self.queries.get(name).or_else(|| app.queries.get(name))
    }
}

/// One node in the reloaded nav tree; see [`crate::model::NavItem`] for
/// the markup-level equivalent.
#[derive(Debug, Clone)]
pub struct NavNode {
    pub label: String,
    pub target_page: Option<String>,
    pub children: Vec<NavNode>,
}

#[derive(Debug, Clone)]
pub struct RuntimeApp {
    pub name: String,
    pub pages: Vec<RuntimePage>,
    pub nav: Vec<NavNode>,
    pub header: Vec<RuntimePageItem>,
    pub footer: Vec<RuntimePageItem>,
    /// Queries visible from every page.
    pub queries: HashMap<String, RuntimeQuery>,
}

impl RuntimeApp {
    pub fn page(&self, name: &str) -> Option<&RuntimePage> {
        self.pages.iter().find(|p| p.name == name)
    }

    /// Everything site-wide that renderers need alongside a single
    /// page: the nav tree, header/footer chrome, and the resolved rows
    /// for every `Region` item anywhere on the current request (the
    /// page's own items plus the header/footer), keyed by query name.
    pub fn chrome<'a>(&'a self, regions: &'a RegionRows) -> Chrome<'a> {
        Chrome {
            nav: &self.nav,
            header: &self.header,
            footer: &self.footer,
            regions,
        }
    }
}

/// Rows already fetched for each `Region` page item that appears on the
/// current request, keyed by query name. Resolving these is an async DB
/// round trip, so `server.rs` does it up front and hands the result to
/// the (synchronous) render functions.
pub type RegionRows = HashMap<String, Vec<BTreeMap<String, Option<String>>>>;

/// Borrowed bundle of the app-wide chrome (nav/header/footer/regions),
/// passed into every page render function instead of four separate
/// parameters.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    pub nav: &'a [NavNode],
    pub header: &'a [RuntimePageItem],
    pub footer: &'a [RuntimePageItem],
    pub regions: &'a RegionRows,
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

/// Loads the current `runtime.js` content for `app_name` — whatever's in
/// `pgapp_meta.app_runtime_js` now, not necessarily `DEFAULT_RUNTIME_JS`
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

/// Reloads the full runtime model for `app_name` straight from
/// `pgapp_meta`, proving the database (not the parsed markup struct) is
/// the authority once the server starts handling requests.
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

        let item_type_rows: Vec<(String, String, Vec<String>, Option<String>)> = sqlx::query_as(
            "select field_name, item_type, choices, choices_query from pgapp_meta.page_field_items
              where page_id = $1",
        )
        .bind(row.id)
        .fetch_all(pool)
        .await?;
        let item_types = item_type_rows
            .into_iter()
            .map(|(field_name, item_type, choices, choices_query)| {
                (field_name, FieldItemType::from_parts(&item_type, choices, choices_query))
            })
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
