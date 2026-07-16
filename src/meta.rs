//! Syncs a parsed [`AppDef`] into the in-database metadata tables
//! (`pgapp_meta.*`), creates the physical data tables that back each
//! entity, and reloads a [`RuntimeApp`] straight from that metadata.
//!
//! The metadata tables — not the markup file — are the source of truth
//! once the server is running: `load_app` re-derives everything the
//! server needs (table names, column types) from `pgapp_meta`.

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::model::{AppDef, FieldType};

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

/// Upserts the app/entity/field/page metadata and makes sure the
/// physical data table for each entity exists.
pub async fn sync_app(pool: &PgPool, app: &AppDef) -> Result<()> {
    let app_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.apps (name) values ($1)
         on conflict (name) do update set name = excluded.name
         returning id",
    )
    .bind(&app.name)
    .fetch_one(pool)
    .await?;

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

        for page in app.pages.iter().filter(|p| p.entity == entity.name) {
            let page_id: i32 = sqlx::query_scalar(
                "insert into pgapp_meta.pages (app_id, entity_id, name, page_type)
                 values ($1, $2, $3, 'crud')
                 on conflict (app_id, name) do update set entity_id = excluded.entity_id
                 returning id",
            )
            .bind(app_id)
            .bind(entity_id)
            .bind(&page.name)
            .fetch_one(pool)
            .await?;

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
        }
    }

    Ok(())
}

async fn ensure_data_table(
    pool: &PgPool,
    table_name: &str,
    entity: &crate::model::EntityDef,
) -> Result<()> {
    let mut cols = Vec::new();
    for field in &entity.fields {
        let mut col = format!("{} {}", field.name, field.ty.sql_column_type());
        if field.ty != FieldType::Id {
            if field.required {
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
        cols.push(col);
    }

    let sql = format!(
        "create table if not exists pgapp_data.{table_name} ({})",
        cols.join(", ")
    );
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to create data table pgapp_data.{table_name}"))?;
    Ok(())
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

#[derive(Debug, Clone)]
pub struct RuntimePage {
    pub name: String,
    pub entity: RuntimeEntity,
    pub columns: Vec<String>,
    pub form: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeApp {
    pub name: String,
    pub pages: Vec<RuntimePage>,
}

impl RuntimeApp {
    pub fn page(&self, name: &str) -> Option<&RuntimePage> {
        self.pages.iter().find(|p| p.name == name)
    }
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

    let page_rows: Vec<(i32, String, i32)> = sqlx::query_as(
        "select id, name, entity_id from pgapp_meta.pages where app_id = $1 order by id",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut pages = Vec::new();
    for (page_id, page_name, entity_id) in page_rows {
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

        let fields: Vec<RuntimeField> = field_rows
            .into_iter()
            .map(|(name, data_type, required)| RuntimeField {
                name,
                data_type: FieldType::from_str_lossy(&data_type),
                required,
            })
            .collect();

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
            .map(|(name, _, _)| name.clone())
            .collect();
        let form = pf_rows
            .iter()
            .filter(|(_, _, shown_in_form)| *shown_in_form)
            .map(|(name, _, _)| name.clone())
            .collect();

        pages.push(RuntimePage {
            name: page_name,
            entity: RuntimeEntity {
                name: entity_name,
                table_name,
                fields,
            },
            columns,
            form,
        });
    }

    Ok(RuntimeApp {
        name: app_name.to_string(),
        pages,
    })
}
