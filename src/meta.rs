//! Syncs a parsed [`AppDef`] into the in-database metadata tables
//! (`pgapp_meta.*`), creates the physical data tables that back each
//! entity, and reloads a [`RuntimeApp`] straight from that metadata.
//!
//! The metadata tables — not the markup file — are the source of truth
//! once the server is running: `load_app` re-derives everything the
//! server needs (table names, column types, links, nav, item widgets)
//! from `pgapp_meta`.
//!
//! Syncing happens in phases because pages, page items, link columns and
//! nav items can all reference *other* pages by name, including ones
//! declared later in the file: phase 1 creates entities/fields/tables,
//! phase 2 creates a bare row for every page (so every page has an id),
//! and phase 3 onward resolves everything that points at a page by name.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;

use crate::model::{AppDef, FieldItemType, FieldType, PageItem, PageKind};

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

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_meta schema")?;
    Ok(())
}

/// Upserts the app/entity/field/page/item/nav metadata and makes sure
/// the physical data table for each entity exists.
pub async fn sync_app(pool: &PgPool, app: &AppDef) -> Result<()> {
    let app_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.apps (name) values ($1)
         on conflict (name) do update set name = excluded.name
         returning id",
    )
    .bind(&app.name)
    .fetch_one(pool)
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

    // Phase 3: page fields, field item types, link columns, and page
    // items — all of which may reference a page by name.
    for page in &app.pages {
        let page_id = page_ids[&page.name];

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

                sqlx::query(
                    "insert into pgapp_meta.page_field_items (page_id, field_name, item_type, choices)
                     values ($1, $2, $3, $4)
                     on conflict (page_id, field_name) do update set
                        item_type = excluded.item_type,
                        choices = excluded.choices",
                )
                .bind(page_id)
                .bind(field_name)
                .bind(item_type.kind_str())
                .bind(item_type.choices())
                .execute(pool)
                .await?;
            }
        }

        let (link_field, link_target_id) = match &page.link_column {
            None => (None, None),
            Some(lc) => {
                let target_id = *page_ids.get(&lc.target_page).with_context(|| {
                    format!(
                        "page '{}' links to unknown page '{}'",
                        page.name, lc.target_page
                    )
                })?;
                (Some(lc.field.clone()), Some(target_id))
            }
        };
        sqlx::query("update pgapp_meta.pages set link_field = $2, link_target_page_id = $3 where id = $1")
            .bind(page_id)
            .bind(link_field)
            .bind(link_target_id)
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

/// Replaces the text/link items owned by one row (a page, or the app
/// itself for header/footer) — shared by `page_items`, `header_items`,
/// and `footer_items`, which only differ in table name and owning
/// column.
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
        let (kind, label, target_id) = match item {
            PageItem::Text(text) => ("text", text.clone(), None),
            PageItem::Link { label, target_page } => {
                let target_id = *page_ids
                    .get(target_page)
                    .with_context(|| format!("{owner_label} links to unknown page '{target_page}'"))?;
                ("link", label.clone(), Some(target_id))
            }
        };
        sqlx::query(&format!(
            "insert into {table} ({owner_col}, kind, label, target_page_id, ordinal)
             values ($1, $2, $3, $4, $5)"
        ))
        .bind(owner_id)
        .bind(kind)
        .bind(label)
        .bind(target_id)
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
/// database round trip.
#[derive(Debug, Clone)]
pub enum RuntimePageItem {
    Text(String),
    Link { label: String, target_page: String },
}

#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
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
}

impl RuntimeApp {
    pub fn page(&self, name: &str) -> Option<&RuntimePage> {
        self.pages.iter().find(|p| p.name == name)
    }

    /// Everything site-wide that renderers need alongside a single
    /// page: the nav tree plus header/footer chrome.
    pub fn chrome(&self) -> Chrome<'_> {
        Chrome {
            nav: &self.nav,
            header: &self.header,
            footer: &self.footer,
        }
    }
}

/// Borrowed bundle of the app-wide chrome (nav/header/footer), passed
/// into every page render function instead of three separate
/// parameters.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    pub nav: &'a [NavNode],
    pub header: &'a [RuntimePageItem],
    pub footer: &'a [RuntimePageItem],
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

    let page_rows: Vec<(i32, String, Option<i32>, String, Option<String>, Option<i32>)> =
        sqlx::query_as(
            "select id, name, entity_id, page_type, link_field, link_target_page_id
               from pgapp_meta.pages where app_id = $1 order by id",
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?;

    let page_names: HashMap<i32, String> = page_rows
        .iter()
        .map(|(id, name, ..)| (*id, name.clone()))
        .collect();

    let mut pages = Vec::new();
    for (page_id, page_name, entity_id, page_type, link_field, link_target_page_id) in &page_rows {
        let entity = match entity_id {
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
            .bind(page_id)
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

        let link_column = match (link_field, link_target_page_id) {
            (Some(field), Some(target_id)) => Some(LinkColumn {
                field: field.clone(),
                target_page: page_names[target_id].clone(),
            }),
            _ => None,
        };

        let item_type_rows: Vec<(String, String, Vec<String>)> = sqlx::query_as(
            "select field_name, item_type, choices from pgapp_meta.page_field_items
              where page_id = $1",
        )
        .bind(page_id)
        .fetch_all(pool)
        .await?;
        let item_types = item_type_rows
            .into_iter()
            .map(|(field_name, item_type, choices)| (field_name, FieldItemType::from_parts(&item_type, choices)))
            .collect();

        let items = load_items(pool, "pgapp_meta.page_items", "page_id", *page_id, &page_names).await?;

        pages.push(RuntimePage {
            name: page_name.clone(),
            kind: PageKind::from_str_lossy(page_type),
            entity,
            columns,
            form,
            link_column,
            items,
            item_types,
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
    })
}

/// Loads the text/link items owned by one row — shared by `page_items`,
/// `header_items`, and `footer_items`.
async fn load_items(
    pool: &PgPool,
    table: &str,
    owner_col: &str,
    owner_id: i32,
    page_names: &HashMap<i32, String>,
) -> Result<Vec<RuntimePageItem>> {
    let rows: Vec<(String, String, Option<i32>)> = sqlx::query_as(&format!(
        "select kind, label, target_page_id from {table} where {owner_col} = $1 order by ordinal"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(kind, label, target_page_id)| match kind.as_str() {
            "link" => RuntimePageItem::Link {
                label,
                target_page: page_names[&target_page_id.expect("link items always have a target page")].clone(),
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
