//! pgapp's own control plane: the durable, database-backed registry of
//! which apps a server process serves and where each one's markup
//! lives on disk (`pgapp_control.apps`) — deliberately its own schema,
//! separate from `pgapp_meta` (an app's synced runtime metadata) and
//! `pgapp_data`/a workspace's own schema (an app's rows). This is what
//! makes the app list survive across restarts and across which
//! `.pgapp` path happens to be on the command line this time: `cargo
//! run -- <path>` registers (or re-points) one slug, but every enabled
//! row in this table gets loaded and served, not just that one.
//!
//! [`register`]/[`list_enabled`] serve the classic single-workspace
//! flow (every app's data tables in the global `pgapp_data` schema).
//! [`register_in_workspace`] is the same idea for an app created via
//! `pgapp app create`/`pgapp run --workspace`, whose data tables live
//! in that workspace's own schema instead — see `src/instance.rs` for
//! how a workspace's schema/owning role get set up.

use anyhow::{Context, Result};
use sqlx::PgPool;

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/control_schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_control schema")?;
    Ok(())
}

// ---- apps ----

/// Upserts a slug's markup path, enabling it if it had been disabled —
/// re-registering an app (e.g. running `cargo run -- <path>` again
/// after `remove`) brings it back. Classic flow only: data tables stay
/// in the global `pgapp_data` schema, no workspace attached.
pub async fn register(pool: &PgPool, slug: &str, markup_path: &str, app_name: &str) -> Result<()> {
    sqlx::query(
        "insert into pgapp_control.apps (slug, markup_path, app_name)
         values ($1, $2, $3)
         on conflict (slug) do update set
            markup_path = excluded.markup_path,
            app_name = excluded.app_name,
            enabled = true,
            updated_at = now()",
    )
    .bind(slug)
    .bind(markup_path)
    .bind(app_name)
    .execute(pool)
    .await
    .context("failed to register app in pgapp_control.apps")?;
    Ok(())
}

/// Same as [`register`], but for an app that lives inside a workspace
/// — its data tables are created in `data_schema` (that workspace's own
/// schema) instead of the global `pgapp_data`.
pub async fn register_in_workspace(
    pool: &PgPool,
    slug: &str,
    markup_path: &str,
    app_name: &str,
    workspace_id: i32,
    data_schema: &str,
) -> Result<()> {
    sqlx::query(
        "insert into pgapp_control.apps (slug, markup_path, app_name, workspace_id, data_schema)
         values ($1, $2, $3, $4, $5)
         on conflict (slug) do update set
            markup_path = excluded.markup_path,
            app_name = excluded.app_name,
            workspace_id = excluded.workspace_id,
            data_schema = excluded.data_schema,
            enabled = true,
            updated_at = now()",
    )
    .bind(slug)
    .bind(markup_path)
    .bind(app_name)
    .bind(workspace_id)
    .bind(data_schema)
    .execute(pool)
    .await
    .context("failed to register app in pgapp_control.apps")?;
    Ok(())
}

/// (id, slug, markup_path, data_schema, workspace_id) for every enabled
/// app, in a stable order — the full set a server process loads and
/// serves on startup, classic and workspace-scoped apps alike. `id` and
/// `workspace_id` (this table's own, not `pgapp_meta.apps`') are what
/// `secrets::resolve` scopes a `{{secret...}}` lookup by.
pub async fn list_enabled(pool: &PgPool) -> Result<Vec<(i32, String, String, String, Option<i32>)>> {
    let rows: Vec<(i32, String, String, String, Option<i32>)> = sqlx::query_as(
        "select id, slug, markup_path, data_schema, workspace_id from pgapp_control.apps where enabled order by slug",
    )
    .fetch_all(pool)
    .await
    .context("failed to list registered apps")?;
    Ok(rows)
}

pub struct AppRow {
    pub id: i32,
    pub slug: String,
    pub app_name: String,
    pub markup_path: String,
    pub data_schema: String,
    pub workspace_id: Option<i32>,
    pub workspace_slug: Option<String>,
    pub enabled: bool,
}

/// Every registered app, including disabled ones — what `pgapp apps`
/// prints.
pub async fn list_all(pool: &PgPool) -> Result<Vec<AppRow>> {
    let rows: Vec<(i32, String, String, String, String, Option<i32>, Option<String>, bool)> = sqlx::query_as(
        "select a.id, a.slug, a.app_name, a.markup_path, a.data_schema, a.workspace_id, w.slug, a.enabled
           from pgapp_control.apps a
           left join pgapp_control.workspaces w on w.id = a.workspace_id
          order by a.slug",
    )
    .fetch_all(pool)
    .await
    .context("failed to list registered apps")?;
    Ok(rows
        .into_iter()
        .map(|(id, slug, app_name, markup_path, data_schema, workspace_id, workspace_slug, enabled)| AppRow {
            id,
            slug,
            app_name,
            markup_path,
            data_schema,
            workspace_id,
            workspace_slug,
            enabled,
        })
        .collect())
}

pub async fn find_app(pool: &PgPool, slug: &str) -> Result<Option<AppRow>> {
    let row: Option<(i32, String, String, String, String, Option<i32>, Option<String>, bool)> = sqlx::query_as(
        "select a.id, a.slug, a.app_name, a.markup_path, a.data_schema, a.workspace_id, w.slug, a.enabled
           from pgapp_control.apps a
           left join pgapp_control.workspaces w on w.id = a.workspace_id
          where a.slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .context("failed to look up app")?;
    Ok(row.map(|(id, slug, app_name, markup_path, data_schema, workspace_id, workspace_slug, enabled)| AppRow {
        id,
        slug,
        app_name,
        markup_path,
        data_schema,
        workspace_id,
        workspace_slug,
        enabled,
    }))
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

/// Removes an app's registry row outright — the bookkeeping half of a
/// hard delete; the caller is responsible for dropping the app's own
/// `pgapp_meta`/data-table rows first (see `main.rs`'s hard-delete
/// handler, which needs `pgapp_meta` to know every entity table name).
pub async fn delete_app_row(pool: &PgPool, slug: &str) -> Result<bool> {
    let result = sqlx::query("delete from pgapp_control.apps where slug = $1")
        .bind(slug)
        .execute(pool)
        .await
        .context("failed to delete app row from pgapp_control.apps")?;
    Ok(result.rows_affected() > 0)
}

// ---- workspaces ----

pub struct WorkspaceRow {
    pub id: i32,
    pub slug: String,
    pub schema_name: String,
    pub owner_role: Option<String>,
    pub enabled: bool,
}

/// Registers a workspace's schema — `owner_role` is `Some` only when
/// pgapp itself created the schema's owning login role (a brand new
/// workspace); `None` means an existing schema pgapp was just granted
/// access into, so there's no role of pgapp's own to ever drop later.
pub async fn register_workspace(pool: &PgPool, slug: &str, schema_name: &str, owner_role: Option<&str>) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        "insert into pgapp_control.workspaces (slug, schema_name, owner_role)
         values ($1, $2, $3)
         on conflict (slug) do update set
            schema_name = excluded.schema_name,
            owner_role = excluded.owner_role,
            enabled = true,
            updated_at = now()
         returning id",
    )
    .bind(slug)
    .bind(schema_name)
    .bind(owner_role)
    .fetch_one(pool)
    .await
    .context("failed to register workspace in pgapp_control.workspaces")?;
    Ok(id)
}

pub async fn find_workspace(pool: &PgPool, slug: &str) -> Result<Option<WorkspaceRow>> {
    let row: Option<(i32, String, String, Option<String>, bool)> = sqlx::query_as(
        "select id, slug, schema_name, owner_role, enabled from pgapp_control.workspaces where slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .context("failed to look up workspace")?;
    Ok(row.map(|(id, slug, schema_name, owner_role, enabled)| WorkspaceRow { id, slug, schema_name, owner_role, enabled }))
}

pub async fn list_workspaces(pool: &PgPool) -> Result<Vec<WorkspaceRow>> {
    let rows: Vec<(i32, String, String, Option<String>, bool)> = sqlx::query_as(
        "select id, slug, schema_name, owner_role, enabled from pgapp_control.workspaces order by slug",
    )
    .fetch_all(pool)
    .await
    .context("failed to list workspaces")?;
    Ok(rows
        .into_iter()
        .map(|(id, slug, schema_name, owner_role, enabled)| WorkspaceRow { id, slug, schema_name, owner_role, enabled })
        .collect())
}

/// How many enabled apps currently live in a workspace — a hard delete
/// of the workspace should refuse (or the caller should warn loudly)
/// while this is nonzero, since dropping the schema out from under
/// them destroys their data too.
pub async fn workspace_app_count(pool: &PgPool, workspace_id: i32) -> Result<i64> {
    sqlx::query_scalar("select count(*) from pgapp_control.apps where workspace_id = $1")
        .bind(workspace_id)
        .fetch_one(pool)
        .await
        .context("failed to count workspace apps")
}

pub async fn disable_workspace(pool: &PgPool, slug: &str) -> Result<bool> {
    let result = sqlx::query("update pgapp_control.workspaces set enabled = false, updated_at = now() where slug = $1")
        .bind(slug)
        .execute(pool)
        .await
        .context("failed to disable workspace")?;
    Ok(result.rows_affected() > 0)
}

/// Removes a workspace's registry row outright — the bookkeeping half
/// of a hard delete; the caller drops the actual schema/role first
/// (see `main.rs`, which needs the schema/owner_role this row names).
pub async fn delete_workspace_row(pool: &PgPool, slug: &str) -> Result<bool> {
    let result = sqlx::query("delete from pgapp_control.workspaces where slug = $1")
        .bind(slug)
        .execute(pool)
        .await
        .context("failed to delete workspace row")?;
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
        sqlx::query("truncate table pgapp_control.apps restart identity cascade").execute(&pool).await.unwrap();
        sqlx::query("truncate table pgapp_control.workspaces restart identity cascade").execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn register_then_list_enabled_roundtrips() {
        let pool = test_pool().await;
        register(&pool, "alpha", "alpha.pgapp", "Alpha").await.unwrap();
        register(&pool, "beta", "beta/", "Beta").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(
            enabled,
            vec![
                (1, "alpha".to_string(), "alpha.pgapp".to_string(), "pgapp_data".to_string(), None),
                (2, "beta".to_string(), "beta/".to_string(), "pgapp_data".to_string(), None)
            ]
        );
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn re_registering_updates_path_and_reenables() {
        let pool = test_pool().await;
        register(&pool, "alpha", "alpha.pgapp", "Alpha").await.unwrap();
        assert!(disable(&pool, "alpha").await.unwrap());
        assert!(list_enabled(&pool).await.unwrap().is_empty());

        register(&pool, "alpha", "alpha2.pgapp", "Alpha").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(enabled, vec![(1, "alpha".to_string(), "alpha2.pgapp".to_string(), "pgapp_data".to_string(), None)]);
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn disabling_an_unknown_slug_reports_no_match() {
        let pool = test_pool().await;
        assert!(!disable(&pool, "nope").await.unwrap());
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn workspace_register_find_and_app_scoping_roundtrips() {
        let pool = test_pool().await;
        let id = register_workspace(&pool, "acme", "acme_schema", Some("acme_schema")).await.unwrap();
        let found = find_workspace(&pool, "acme").await.unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.schema_name, "acme_schema");
        assert_eq!(found.owner_role.as_deref(), Some("acme_schema"));
        assert!(found.enabled);

        assert_eq!(workspace_app_count(&pool, id).await.unwrap(), 0);
        register_in_workspace(&pool, "widgets", "widgets.pgapp", "Widgets", id, "acme_schema").await.unwrap();
        assert_eq!(workspace_app_count(&pool, id).await.unwrap(), 1);

        let app = find_app(&pool, "widgets").await.unwrap().unwrap();
        assert_eq!(app.data_schema, "acme_schema");
        assert_eq!(app.workspace_slug.as_deref(), Some("acme"));

        assert!(disable_workspace(&pool, "acme").await.unwrap());
        assert!(!find_workspace(&pool, "acme").await.unwrap().unwrap().enabled);

        assert!(delete_workspace_row(&pool, "acme").await.unwrap());
        assert!(find_workspace(&pool, "acme").await.unwrap().is_none());
    }
}
