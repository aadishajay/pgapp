//! In-database metadata: syncing a parsed [`crate::model::AppDef`] into
//! `pgapp_meta.*` ([`sync`]), and reloading a [`RuntimeApp`] straight
//! from it afterward ([`load`]) — the metadata tables, not the markup
//! file, are the source of truth once the server is running.

mod load;
mod sync;
mod types;

pub use load::{compile_named_query, load_app, load_runtime_js};
pub use sync::{force_refresh_runtime_js, sync_app};
pub use types::*;

use anyhow::{Context, Result};
use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres};

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_meta schema")?;
    Ok(())
}

/// Acquires one connection and pins its `search_path` to `data_schema`
/// before handing it back — every named query an app author writes
/// (`query <name> { sql: "..." }`) is raw SQL `compile_named_query`
/// never rewrites, so a bare (unqualified) table reference in it only
/// resolves to *that app's* tables if the connection running it has
/// this set. One `PgPool` serves every app and workspace in the
/// process, so Postgres's own default `search_path` ("$user", public)
/// can't know which app a given borrowed connection is for.
///
/// Set unconditionally on every call, never left to whatever a
/// connection's previous borrower set it to — the pool recycles
/// connections across apps/workspaces, so anything less would let one
/// app's queries silently see another's schema depending on pool
/// scheduling. An already schema-qualified reference (e.g. `erp.foo`)
/// is unaffected either way, since `search_path` only ever affects
/// *unqualified* names.
pub async fn scoped_conn(pool: &PgPool, data_schema: &str) -> Result<PoolConnection<Postgres>> {
    let mut conn = pool.acquire().await.context("failed to acquire a database connection")?;
    sqlx::query(&format!("set search_path to {data_schema}, public"))
        .execute(&mut *conn)
        .await
        .with_context(|| format!("failed to set search_path to '{data_schema}'"))?;
    Ok(conn)
}
