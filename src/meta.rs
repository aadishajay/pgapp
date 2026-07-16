//! In-database metadata: syncing a parsed [`crate::model::AppDef`] into
//! `pgapp_meta.*` ([`sync`]), and reloading a [`RuntimeApp`] straight
//! from it afterward ([`load`]) — the metadata tables, not the markup
//! file, are the source of truth once the server is running.

mod load;
mod sync;
mod types;

pub use load::{load_app, load_runtime_js};
pub use sync::sync_app;
pub use types::*;

use anyhow::{Context, Result};
use sqlx::PgPool;

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_meta schema")?;
    Ok(())
}
