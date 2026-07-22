//! pgapp's own control plane: the durable, database-backed registry of
//! which apps a server process serves and where each one's markup
//! lives on disk (`pgapp_control.apps`) — deliberately its own schema,
//! separate from `pgapp_meta` (an app's synced runtime metadata) and
//! a workspace's own schema (an app's rows). This is what makes the
//! app list survive across restarts and across which `.pgapp` path
//! happens to be on the command line this time: `pgapp run` registers
//! (or re-points) one slug, but every enabled row in this table gets
//! loaded and served, not just that one.
//!
//! Every app belongs to exactly one workspace — see
//! [`register_in_workspace`] and `src/instance.rs` for how a
//! workspace's schema/owning role get set up.

use anyhow::{Context, Result};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("../db/control_schema.sql"))
        .execute(pool)
        .await
        .context("failed to apply pgapp_control schema")?;
    Ok(())
}

// ---- apps ----

/// Upserts a slug's markup path, enabling it if it had been disabled,
/// scoping it to a workspace's own schema — an app registered via
/// `pgapp app create`/`pgapp run --workspace`.
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
         on conflict (workspace_id, slug) do update set
            markup_path = excluded.markup_path,
            app_name = excluded.app_name,
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

/// (id, slug, markup_path, data_schema, workspace_id, workspace_slug)
/// for every enabled app, in a stable order — the full set a server
/// process loads and serves on startup. `id` and `workspace_id` (this
/// table's own, not `pgapp_meta.apps`') are what `secrets::resolve`
/// scopes a `{{secret...}}` lookup by; `workspace_slug` is what
/// `main.rs` prefixes onto the app's own slug to build its URL path
/// (`/<workspace_slug>/<slug>/...`), so two apps of the same name in
/// different workspaces don't collide there either.
pub async fn list_enabled(pool: &PgPool) -> Result<Vec<(i32, String, String, String, Option<i32>, Option<String>)>> {
    let rows: Vec<(i32, String, String, String, Option<i32>, Option<String>)> = sqlx::query_as(
        "select a.id, a.slug, a.markup_path, a.data_schema, a.workspace_id, w.slug
           from pgapp_control.apps a
           left join pgapp_control.workspaces w on w.id = a.workspace_id
          where a.enabled
          order by a.slug",
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

/// Looks up an app by its slug — `workspace_slug` disambiguates when
/// more than one workspace happens to register an app under the same
/// slug (allowed since slug is only unique *per workspace*, not
/// instance-wide; see `db/control_schema.sql`'s `apps_workspace_slug`
/// index). Omitting it works as long as `slug` is unambiguous right
/// now; once two workspaces share it, every command that identifies an
/// app by bare slug alone needs `--workspace` to say which one.
pub async fn find_app(pool: &PgPool, slug: &str, workspace_slug: Option<&str>) -> Result<Option<AppRow>> {
    let rows: Vec<(i32, String, String, String, String, Option<i32>, Option<String>, bool)> = sqlx::query_as(
        "select a.id, a.slug, a.app_name, a.markup_path, a.data_schema, a.workspace_id, w.slug, a.enabled
           from pgapp_control.apps a
           left join pgapp_control.workspaces w on w.id = a.workspace_id
          where a.slug = $1 and ($2::text is null or w.slug = $2)",
    )
    .bind(slug)
    .bind(workspace_slug)
    .fetch_all(pool)
    .await
    .context("failed to look up app")?;

    if workspace_slug.is_none() && rows.len() > 1 {
        let workspaces: Vec<&str> = rows.iter().filter_map(|r| r.6.as_deref()).collect();
        anyhow::bail!(
            "slug '{slug}' is registered in more than one workspace ({}) — pass --workspace to say which one",
            workspaces.join(", ")
        );
    }

    Ok(rows
        .into_iter()
        .next()
        .map(|(id, slug, app_name, markup_path, data_schema, workspace_id, workspace_slug, enabled)| AppRow {
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

/// Disables an app (by its `pgapp_control.apps.id`, not slug — a slug
/// alone can now name more than one row across different workspaces)
/// so it stops being served, without deleting its row — re-registering
/// it later reactivates it. Returns whether a matching row existed.
pub async fn disable(pool: &PgPool, id: i32) -> Result<bool> {
    let result = sqlx::query("update pgapp_control.apps set enabled = false, updated_at = now() where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to disable app in pgapp_control.apps")?;
    Ok(result.rows_affected() > 0)
}

/// Removes an app's registry row outright, by `id` (same reasoning as
/// [`disable`]) — the bookkeeping half of a hard delete; the caller is
/// responsible for dropping the app's own `pgapp_meta`/data-table rows
/// first (see `main.rs`'s hard-delete handler, which needs
/// `pgapp_meta` to know every entity table name).
pub async fn delete_app_row(pool: &PgPool, id: i32) -> Result<bool> {
    let result = sqlx::query("delete from pgapp_control.apps where id = $1")
        .bind(id)
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

// ---- workspace provisioning ----
//
// The shared implementation behind `pgapp workspace create` (CLI,
// interactive) and the App Builder's "New Workspace" web form (see
// `actions::create_workspace`) — moved here from `main.rs` so both can
// call the exact same DDL rather than keeping two copies in sync.

/// Creates (or, if it already exists, re-passwords) a Postgres login
/// role — used both for `pgapp_admin` itself (see `instance.rs`) and
/// for a new workspace's own schema-owning role. `role` must already
/// be validated (see `instance::valid_identifier`); Postgres has no
/// bind-parameter form for identifiers or for the PASSWORD clause's
/// literal.
pub async fn ensure_role(pool: &PgPool, role: &str, password: &str) -> Result<()> {
    if !crate::instance::valid_identifier(role) {
        anyhow::bail!("'{role}' is not a valid role name");
    }
    let exists: bool = sqlx::query_scalar("select exists(select 1 from pg_roles where rolname = $1)")
        .bind(role)
        .fetch_one(pool)
        .await?;
    let escaped = password.replace('\'', "''");
    if exists {
        sqlx::raw_sql(&format!("alter role {role} password '{escaped}'"))
            .execute(pool)
            .await
            .with_context(|| format!("failed to update role '{role}'"))?;
    } else {
        sqlx::raw_sql(&format!("create role {role} login password '{escaped}'"))
            .execute(pool)
            .await
            .with_context(|| format!("failed to create role '{role}'"))?;
    }
    Ok(())
}

/// Grants pgapp_admin everything it needs to operate inside `schema`:
/// USAGE/CREATE on the schema itself, plus full privileges on whatever
/// tables/sequences already live there — schema-level GRANT alone
/// doesn't reach pre-existing objects. The default-privileges lines
/// cover the bootstrap role creating more tables here later without
/// needing another manual grant.
pub async fn grant_admin_on_schema(pool: &PgPool, schema: &str) -> Result<()> {
    if !crate::instance::valid_identifier(schema) {
        anyhow::bail!("'{schema}' is not a valid schema name");
    }
    let role = crate::instance::ADMIN_ROLE;
    for sql in [
        format!("grant usage, create on schema {schema} to {role}"),
        format!("grant all privileges on all tables in schema {schema} to {role}"),
        format!("grant all privileges on all sequences in schema {schema} to {role}"),
        format!("alter default privileges in schema {schema} grant all on tables to {role}"),
        format!("alter default privileges in schema {schema} grant all on sequences to {role}"),
    ] {
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .with_context(|| format!("failed to grant {role} access to schema '{schema}'"))?;
    }
    Ok(())
}

async fn schema_exists(pool: &PgPool, schema_name: &str) -> Result<bool> {
    // pg_namespace, not information_schema.schemata: the latter only
    // lists schemas the connecting role already has some privilege on,
    // which pgapp_admin by definition doesn't yet for a schema it's
    // about to ask permission to use.
    sqlx::query_scalar("select exists(select 1 from pg_catalog.pg_namespace where nspname = $1)")
        .bind(schema_name)
        .fetch_one(pool)
        .await
        .context("failed to check whether the schema already exists")
}

fn validate_new_workspace_names(schema_name: &str, slug: &str) -> Result<()> {
    if !crate::instance::valid_identifier(schema_name) {
        anyhow::bail!("'{schema_name}' must start with a letter/underscore and contain only letters, digits, underscores");
    }
    if !crate::instance::valid_identifier(slug) {
        anyhow::bail!("'{slug}' must start with a letter/underscore and contain only letters, digits, underscores");
    }
    Ok(())
}

/// The "brand new schema" half of workspace creation: creates a
/// dedicated login role + a schema it owns, grants `pgapp_admin`
/// access, and registers the workspace.
pub async fn create_workspace_new_schema(pool: &PgPool, slug: &str, schema_name: &str, password: &str) -> Result<()> {
    validate_new_workspace_names(schema_name, slug)?;
    if find_workspace(pool, slug).await?.is_some() {
        anyhow::bail!("workspace '{slug}' already exists");
    }
    if schema_exists(pool, schema_name).await? {
        anyhow::bail!("schema '{schema_name}' already exists — use \"attach to an existing schema\" instead");
    }
    ensure_role(pool, schema_name, password).await?;
    // CREATE SCHEMA ... AUTHORIZATION <role> requires being able to SET
    // ROLE to it (Postgres won't let you authorize a schema to a role
    // you aren't a member of) — pgapp_admin needs membership in the
    // workspace role it just created before it can do this.
    sqlx::raw_sql(&format!("grant {schema_name} to {}", crate::instance::ADMIN_ROLE))
        .execute(pool)
        .await
        .with_context(|| format!("failed to grant membership in '{schema_name}' to {}", crate::instance::ADMIN_ROLE))?;
    sqlx::raw_sql(&format!("create schema if not exists {schema_name} authorization {schema_name}"))
        .execute(pool)
        .await
        .with_context(|| format!("failed to create schema '{schema_name}'"))?;
    grant_admin_on_schema(pool, schema_name).await?;
    register_workspace(pool, slug, schema_name, Some(schema_name)).await?;
    Ok(())
}

/// The "attach to an already-existing schema" half: connects with a
/// caller-supplied, superuser-capable connection string just long
/// enough to grant `pgapp_admin` access, then closes that connection.
/// The string itself is never written anywhere by this function — not
/// to a table, and (since every error here is a plain `anyhow::bail!`/
/// `.context(...)`, never the connection error's own possibly-detailed
/// source) not into any message this function can return either. See
/// `actions::create_workspace`'s doc for why that matters.
pub async fn create_workspace_existing_schema(pool: &PgPool, slug: &str, schema_name: &str, grantor_conn: &str) -> Result<()> {
    validate_new_workspace_names(schema_name, slug)?;
    if find_workspace(pool, slug).await?.is_some() {
        anyhow::bail!("workspace '{slug}' already exists");
    }
    if !schema_exists(pool, schema_name).await? {
        anyhow::bail!("schema '{schema_name}' does not exist — use \"create a new schema\" instead");
    }
    let opts: PgConnectOptions = grantor_conn.parse().context("not a valid Postgres connection string")?;
    let grantor_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .context("failed to connect with the given connection string")?;
    let result = grant_admin_on_schema(&grantor_pool, schema_name).await;
    grantor_pool.close().await;
    result?;
    register_workspace(pool, slug, schema_name, None).await?;
    Ok(())
}

/// Best-effort `drop <sql>` — logs and swallows a failure rather than
/// aborting the rest of a teardown over one already-gone object, same
/// tolerance `main.rs`'s own `try_drop` (CLI) has always had.
async fn try_drop(pool: &PgPool, sql: &str, what: &str) {
    if let Err(e) = sqlx::raw_sql(sql).execute(pool).await {
        println!("pgapp: warning: failed to drop {what}: {e}");
    }
}

// ---- teardown ----

/// Hard-deletes one app: drops its own physical entity tables (never a
/// query-backed entity's, which has none), then its `pgapp_meta.apps`
/// row (cascading to entities/fields/pages/components/etc. — see
/// `db/schema.sql`), then its `pgapp_control.apps` row. No
/// superuser-capable connection is needed — `pgapp_admin` already owns
/// every table it created via `create table`, in any workspace schema
/// it's been granted into. Shared by `pgapp app destroy --hard` and the
/// App Builder's own "Delete App" (hard) button, so the two can't
/// drift.
pub async fn hard_delete_app(pool: &PgPool, app: &AppRow) -> Result<()> {
    let app_id: Option<i32> = sqlx::query_scalar("select id from pgapp_meta.apps where name = $1")
        .bind(&app.app_name)
        .fetch_optional(pool)
        .await
        .context("failed to look up the app's own pgapp_meta row")?;
    if let Some(app_id) = app_id {
        let tables: Vec<String> =
            sqlx::query_scalar("select table_name from pgapp_meta.entities where app_id = $1 and source_query is null")
                .bind(app_id)
                .fetch_all(pool)
                .await
                .context("failed to list the app's own data tables")?;
        for table in tables {
            try_drop(
                pool,
                &format!("drop table if exists {}.{table} cascade", app.data_schema),
                &format!("table '{}.{table}'", app.data_schema),
            )
            .await;
        }
        sqlx::query("delete from pgapp_meta.apps where id = $1")
            .bind(app_id)
            .execute(pool)
            .await
            .context("failed to delete the app's own pgapp_meta row")?;
    }
    delete_app_row(pool, app.id).await?;
    Ok(())
}

/// Hard-deletes a workspace: drops its schema (cascading to every
/// table in it, including any app's data tables still inside) and its
/// owning role (if pgapp created one), then its `pgapp_control.workspaces`
/// row. Unlike [`hard_delete_app`], this genuinely needs a fresh
/// superuser-capable connection — a workspace attached via "attach to
/// an existing schema" was never owned by `pgapp_admin` to begin with,
/// and `DROP ROLE` on the schema's own owning role (when pgapp created
/// one) needs privilege beyond a schema-level grant either way.
/// `grantor_conn` is used only for the `DROP SCHEMA`/`DROP ROLE`
/// themselves and is never persisted, same "used once, in memory only"
/// contract as `create_workspace_existing_schema`'s own connection
/// string.
pub async fn hard_delete_workspace(pool: &PgPool, ws: &WorkspaceRow, grantor_conn: &str) -> Result<()> {
    let opts: PgConnectOptions = grantor_conn.parse().context("not a valid Postgres connection string")?;
    let grantor_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .context("failed to connect with the given connection string")?;

    try_drop(&grantor_pool, &format!("drop schema if exists {} cascade", ws.schema_name), &format!("schema '{}'", ws.schema_name)).await;
    if let Some(role) = &ws.owner_role {
        try_drop(&grantor_pool, &format!("drop role if exists {role}"), &format!("role '{role}'")).await;
    }
    grantor_pool.close().await;

    delete_workspace_row(pool, &ws.slug).await?;
    Ok(())
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
        let ws = register_workspace(&pool, "acme", "acme_schema", Some("acme_schema")).await.unwrap();
        register_in_workspace(&pool, "alpha", "alpha.pgapp", "Alpha", ws, "acme_schema").await.unwrap();
        register_in_workspace(&pool, "beta", "beta/", "Beta", ws, "acme_schema").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(
            enabled,
            vec![
                (1, "alpha".to_string(), "alpha.pgapp".to_string(), "acme_schema".to_string(), Some(ws), Some("acme".to_string())),
                (2, "beta".to_string(), "beta/".to_string(), "acme_schema".to_string(), Some(ws), Some("acme".to_string()))
            ]
        );
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn re_registering_updates_path_and_reenables() {
        let pool = test_pool().await;
        let ws = register_workspace(&pool, "acme", "acme_schema", Some("acme_schema")).await.unwrap();
        register_in_workspace(&pool, "alpha", "alpha.pgapp", "Alpha", ws, "acme_schema").await.unwrap();
        assert!(disable(&pool, 1).await.unwrap());
        assert!(list_enabled(&pool).await.unwrap().is_empty());

        register_in_workspace(&pool, "alpha", "alpha2.pgapp", "Alpha", ws, "acme_schema").await.unwrap();
        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(
            enabled,
            vec![(1, "alpha".to_string(), "alpha2.pgapp".to_string(), "acme_schema".to_string(), Some(ws), Some("acme".to_string()))]
        );
    }

    #[tokio::test]
    #[ignore = "needs a live Postgres; run with `cargo test -- --ignored` and PGAPP_TEST_DATABASE_URL set"]
    async fn disabling_an_unknown_id_reports_no_match() {
        let pool = test_pool().await;
        assert!(!disable(&pool, 999999).await.unwrap());
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

        let app = find_app(&pool, "widgets", None).await.unwrap().unwrap();
        assert_eq!(app.data_schema, "acme_schema");
        assert_eq!(app.workspace_slug.as_deref(), Some("acme"));

        // Two different workspaces can each register an app under the
        // same slug now — only unique per workspace, not instance-wide.
        let acme2 = register_workspace(&pool, "acme2", "acme2_schema", Some("acme2_schema")).await.unwrap();
        register_in_workspace(&pool, "widgets", "widgets2.pgapp", "Widgets Two", acme2, "acme2_schema").await.unwrap();
        assert!(find_app(&pool, "widgets", None).await.is_err(), "ambiguous slug across workspaces should error");
        let disambiguated = find_app(&pool, "widgets", Some("acme2")).await.unwrap().unwrap();
        assert_eq!(disambiguated.data_schema, "acme2_schema");

        assert!(disable_workspace(&pool, "acme").await.unwrap());
        assert!(!find_workspace(&pool, "acme").await.unwrap().unwrap().enabled);

        assert!(delete_workspace_row(&pool, "acme").await.unwrap());
        assert!(find_workspace(&pool, "acme").await.unwrap().is_none());
    }
}
