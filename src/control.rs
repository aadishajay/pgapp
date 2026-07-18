//! pgapp's own control plane: the durable, database-backed registry of
//! which apps a server process serves and where each one's markup
//! lives on disk (`pgapp_control.apps`) — deliberately its own schema,
//! separate from `pgapp_meta` (an app's synced runtime metadata) and
//! `pgapp_data` (an app's rows). This is what makes the app list
//! survive across restarts and across which `.pgapp` path happens to
//! be on the command line this time: `cargo run -- <path>` registers
//! (or re-points) one slug, but every enabled row in this table gets
//! loaded and served, not just that one.

use anyhow::{Context, Result};
use sqlx::PgPool;

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/control_schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_control schema")?;
    Ok(())
}

/// Upserts a slug's markup path, enabling it if it had been disabled —
/// re-registering an app (e.g. running `cargo run -- <path>` again
/// after `remove`) brings it back.
pub async fn register(pool: &PgPool, slug: &str, markup_path: &str) -> Result<()> {
    sqlx::query(
        "insert into pgapp_control.apps (slug, markup_path)
         values ($1, $2)
         on conflict (slug) do update set
            markup_path = excluded.markup_path,
            enabled = true,
            updated_at = now()",
    )
    .bind(slug)
    .bind(markup_path)
    .execute(pool)
    .await
    .context("failed to register app in pgapp_control.apps")?;
    Ok(())
}

/// (slug, markup_path) for every enabled app, in a stable order — the
/// full set a server process loads and serves on startup.
pub async fn list_enabled(pool: &PgPool) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select slug, markup_path from pgapp_control.apps where enabled order by slug",
    )
    .fetch_all(pool)
    .await
    .context("failed to list registered apps")?;
    Ok(rows)
}

/// (slug, markup_path, enabled) for every registered app, including
/// disabled ones — what `cargo run -- apps` prints.
pub async fn list_all(pool: &PgPool) -> Result<Vec<(String, String, bool)>> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "select slug, markup_path, enabled from pgapp_control.apps order by slug",
    )
    .fetch_all(pool)
    .await
    .context("failed to list registered apps")?;
    Ok(rows)
}

/// Disables a slug so it stops being served (without deleting its row
/// — re-registering it later reactivates it). Returns whether a
/// matching row existed.
pub async fn disable(pool: &PgPool, slug: &str) -> Result<bool> {
    let result = sqlx::query("update pgapp_control.apps set enabled = false, updated_at = now() where slug = $1")
        .bind(slug)
        .execute(pool)
        .await
        .context("failed to disable app in pgapp_control.apps")?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> PgPool {
        let url = std::env::var("PGAPP_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/pgapp_test".to_string());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("PGAPP_TEST_DATABASE_URL must point at a reachable Postgres for this test");
        ensure_schema(&pool).await.unwrap();
        sqlx::query("truncate table pgapp_control.apps restart identity").execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn register_then_list_enabled_roundtrips() {
        let pool = test_pool().await;
        register(&pool, "alpha", "alpha.pgapp").await.unwrap();
        register(&pool, "beta", "beta/").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(enabled, vec![("alpha".to_string(), "alpha.pgapp".to_string()), ("beta".to_string(), "beta/".to_string())]);
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn re_registering_updates_path_and_reenables() {
        let pool = test_pool().await;
        register(&pool, "alpha", "alpha.pgapp").await.unwrap();
        assert!(disable(&pool, "alpha").await.unwrap());
        assert!(list_enabled(&pool).await.unwrap().is_empty());

        register(&pool, "alpha", "alpha2.pgapp").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(enabled, vec![("alpha".to_string(), "alpha2.pgapp".to_string())]);
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn disabling_an_unknown_slug_reports_no_match() {
        let pool = test_pool().await;
        assert!(!disable(&pool, "nope").await.unwrap());
    }
}
