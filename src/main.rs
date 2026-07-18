use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

use pgapp::{actions, chart_lib, control, icons, instance, item_types, meta, scaffold, server, source, theme};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    match cli_args.get(1).map(|s| s.as_str()) {
        Some("new") | Some("create") => return scaffold::run(&cli_args[2..]).await,
        Some("instance") => return cmd_instance(&cli_args[2..]).await,
        Some("workspace") => return cmd_workspace(&cli_args[2..]).await,
        Some("app") => return cmd_app(&cli_args[2..]).await,
        Some("run") => return cmd_run(&cli_args[2..]).await,
        _ => {}
    }

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/pgapp".to_string());

    match cli_args.get(1).map(|s| s.as_str()) {
        Some("apps") => return list_registered_apps(&database_url).await,
        Some("remove") => {
            let slug = cli_args
                .get(2)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp remove <slug> (see `pgapp apps` for slugs)"))?;
            return remove_app(&database_url, &slug).await;
        }
        _ => {}
    }

    // ---- classic mode: one shared global pgapp_meta/pgapp_data, no instance/workspace ceremony ----
    let markup_path = cli_args.get(1).cloned().unwrap_or_else(|| "examples/todo.pgapp".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(instance::max_connections())
        .connect(&database_url)
        .await
        .with_context(|| format!("failed to connect to database '{database_url}'"))?;

    // `pgapp_control` is pgapp's own control plane (which apps this
    // server serves, and where each one's markup lives — see
    // src/control.rs); `pgapp_meta`/`pgapp_data` are the per-app
    // runtime metadata/rows synced from each app's own markup.
    control::ensure_schema(&pool).await?;
    meta::ensure_schema(&pool).await?;

    // A single file/directory is one app; a directory of subdirectories
    // is a workspace of several (see source::load_workspace). Either
    // way, every app found this run gets (re-)registered — but what
    // actually gets *served* below is every enabled row in the control
    // table, not just these, so a previous `cargo run -- other.pgapp`
    // keeps serving alongside whatever's registered this time.
    let discovered = source::load_workspace(&markup_path)
        .with_context(|| format!("failed to load '{markup_path}'"))?;
    for (slug, path, app_def) in &discovered {
        control::register(&pool, slug, path, &app_def.name).await?;
    }

    serve_registered_apps(pool, &bind_addr).await
}

/// Parses, syncs, and loads one app into a fresh [`server::AppEntry`] —
/// exactly what every app registered in `pgapp_control.apps` goes
/// through on every server start (and again, for just this one app, on
/// its own `/{app}/admin/reload`).
async fn load_one_app(
    pool: &PgPool,
    markup_path: &str,
    data_schema: &str,
    item_types: &item_types::Registry,
    action_registry: &actions::Registry,
) -> anyhow::Result<server::AppEntry> {
    let app_def = source::load(markup_path)?;
    meta::sync_app(pool, &app_def, item_types, action_registry, data_schema).await?;
    let runtime_app = meta::load_app(pool, &app_def.name).await?;
    let runtime_js = meta::load_runtime_js(pool, &app_def.name).await?;
    let theme = theme::load(runtime_app.theme.as_deref().unwrap_or("shadcn"))?;
    let icons = icons::load(runtime_app.icons.as_deref().unwrap_or("builtin"))?;
    let chart_lib = chart_lib::load(runtime_app.chart_lib.as_deref().unwrap_or("inline"))?;
    Ok(server::AppEntry {
        markup_path: markup_path.to_string(),
        data: std::sync::RwLock::new(Arc::new(server::AppData {
            app: runtime_app,
            theme,
            runtime_js,
            icons,
            chart_lib,
        })),
    })
}

/// The shared tail of every server start, classic or instance-mode:
/// load every enabled row in `pgapp_control.apps` (each already knows
/// its own `data_schema`), print the banner, and serve. One bad app is
/// skipped with a warning rather than taking the whole process down.
async fn serve_registered_apps(pool: PgPool, bind_addr: &str) -> anyhow::Result<()> {
    let item_types = item_types::registry();
    let action_registry = actions::registry();

    let registered = control::list_enabled(&pool).await?;
    let mut apps: HashMap<String, server::AppEntry> = HashMap::new();
    for (slug, path, data_schema) in registered {
        match load_one_app(&pool, &path, &data_schema, &item_types, &action_registry).await {
            Ok(entry) => {
                apps.insert(slug, entry);
            }
            Err(e) => {
                println!("pgapp: warning: skipping app '{slug}' at '{path}' — {e:#}");
            }
        }
    }
    if apps.is_empty() {
        anyhow::bail!("no registered app could be loaded — see the warnings above");
    }

    print_banner(bind_addr, &apps).await;

    let state = Arc::new(server::AppState { pool, apps, item_types, actions: action_registry });
    let router = server::build_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn print_banner(bind_addr: &str, apps: &HashMap<String, server::AppEntry>) {
    let mut slugs: Vec<&String> = apps.keys().collect();
    slugs.sort();

    if slugs.len() > 1 {
        println!("pgapp: serving {} apps from one shared connection pool", slugs.len());
        println!("  http://{bind_addr}/  (lists every app below)");
    }

    for slug in slugs {
        let entry = &apps[slug];
        let data = entry.data();
        println!("pgapp: serving '{}' at http://{bind_addr}/{slug} (from {})", data.app.name, entry.markup_path);
        println!(
            "  theme: {} ({}) - {}",
            data.theme.name,
            if data.theme.meta.label.is_empty() { "no label" } else { &data.theme.meta.label },
            data.theme.meta.description
        );
        println!("  icons: {}", data.icons.name);
        println!("  chart library: {}", data.chart_lib.name);
        println!("  data schema: {}", data.app.data_schema);
        println!(
            "  auth: {}",
            if data.app.auth_enabled {
                "enabled (first visit to /login creates the admin account)"
            } else {
                "disabled (no `auth { }` block in the markup)"
            }
        );
        println!("  hot reload: http://{bind_addr}/{slug}/admin/reload (re-syncs the markup file without restarting)");
        for page in &data.app.pages {
            let kinds: Vec<&str> = page
                .components
                .iter()
                .map(|c| match c {
                    meta::RuntimeComponent::Report { .. } => "report",
                    meta::RuntimeComponent::Form { .. } => "form",
                    meta::RuntimeComponent::EditableTable { .. } => "editable_table",
                    meta::RuntimeComponent::Chart { .. } => "chart",
                    meta::RuntimeComponent::Text { .. } => "text",
                    meta::RuntimeComponent::Link { .. } => "link",
                    meta::RuntimeComponent::Region { .. } => "region",
                    meta::RuntimeComponent::Action { .. } => "action",
                    meta::RuntimeComponent::DynamicAction { .. } => "dynamic_action",
                })
                .collect();
            println!("  http://{bind_addr}/{slug}/{}  [{}]", page.name, kinds.join(", "));
        }
    }
}

async fn control_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .with_context(|| format!("failed to connect to database '{database_url}'"))?;
    control::ensure_schema(&pool).await?;
    Ok(pool)
}

/// `pgapp apps` — lists every app registered in `pgapp_control.apps`
/// (including disabled ones), for a server this database has already
/// been serving apps from.
async fn list_registered_apps(database_url: &str) -> anyhow::Result<()> {
    let pool = control_pool(database_url).await?;
    let apps = control::list_all(&pool).await?;
    if apps.is_empty() {
        println!("no apps registered yet — `cargo run -- <path>` registers one");
        return Ok(());
    }
    for a in apps {
        let ws = a.workspace_slug.as_deref().unwrap_or("-");
        println!(
            "{}\t{}\t{}\tworkspace={ws}\tschema={}\t{}",
            a.slug,
            if a.enabled { "enabled" } else { "disabled" },
            a.app_name,
            a.data_schema,
            a.markup_path,
        );
    }
    Ok(())
}

/// `pgapp remove <slug>` — disables an app so the next server start
/// stops serving it (its markup/rows aren't touched; re-running
/// `cargo run -- <its path>` re-enables it).
async fn remove_app(database_url: &str, slug: &str) -> anyhow::Result<()> {
    let pool = control_pool(database_url).await?;
    if control::disable(&pool, slug).await? {
        println!("removed '{slug}' — it will no longer be served (re-register it by running against its markup path again)");
    } else {
        println!("no app registered under slug '{slug}' (see `pgapp apps`)");
    }
    Ok(())
}

// ============================================================
// Instance mode: `pgapp instance/workspace/app/run` — a durable,
// database-backed multi-workspace deployment with a dedicated
// `pgapp_admin` Postgres role, on top of the classic single-schema
// flow above (which stays completely unchanged). See README's
// "Instance mode" section for the full picture.
// ============================================================

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

async fn try_drop(pool: &PgPool, sql: &str, what: &str) {
    if let Err(e) = sqlx::raw_sql(sql).execute(pool).await {
        println!("pgapp: warning: failed to drop {what}: {e}");
    }
}

/// Creates (or, if it already exists, re-passwords) a Postgres login
/// role — used both for `pgapp_admin` itself and for a new workspace's
/// own schema-owning role. `role` must already be validated (see
/// `instance::valid_identifier`); Postgres has no bind-parameter form
/// for identifiers or for the PASSWORD clause's literal.
async fn ensure_role(pool: &PgPool, role: &str, password: &str) -> anyhow::Result<()> {
    if !instance::valid_identifier(role) {
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
/// doesn't reach pre-existing objects (e.g. the pgapp_meta/pgapp_control
/// tables `ensure_schema` just created as the bootstrap role, or
/// whatever an "existing schema" workspace already had). The default-
/// privileges line covers the bootstrap role creating more tables here
/// later (a future `ensure_schema` migration) without needing another
/// manual grant.
async fn grant_admin_on_schema(pool: &PgPool, schema: &str) -> anyhow::Result<()> {
    if !instance::valid_identifier(schema) {
        anyhow::bail!("'{schema}' is not a valid schema name");
    }
    let role = instance::ADMIN_ROLE;
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

/// Connects `opts` and, if the target database doesn't exist yet
/// (Postgres error 3D000), creates it via the same host/credentials
/// against the `postgres` maintenance database and retries once — the
/// same auto-create behavior `pgapp create`'s interactive scaffold
/// already gives the classic flow, extended to instance init.
async fn connect_with_auto_create(opts: PgConnectOptions) -> anyhow::Result<PgPool> {
    match PgPoolOptions::new().max_connections(5).connect_with(opts.clone()).await {
        Ok(pool) => Ok(pool),
        Err(e) if scaffold::is_missing_database_error(&e) => {
            let db_name = opts.get_database().unwrap_or_default().to_string();
            println!("Database '{db_name}' doesn't exist yet — creating it...");
            let maintenance = opts.clone().database("postgres");
            let admin_pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(maintenance)
                .await
                .context("failed to connect to the 'postgres' maintenance database to create it")?;
            sqlx::query(&format!("create database \"{}\"", db_name.replace('"', "\"\"")))
                .execute(&admin_pool)
                .await
                .with_context(|| format!("failed to create database '{db_name}'"))?;
            PgPoolOptions::new()
                .max_connections(5)
                .connect_with(opts)
                .await
                .context("failed to connect after creating the database")
        }
        Err(e) => Err(e).context("failed to connect"),
    }
}

async fn cmd_instance(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("init") => instance_init().await,
        Some("destroy") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp instance destroy <dbname>"))?;
            instance_destroy(&dbname).await
        }
        _ => {
            println!("usage: pgapp instance init | pgapp instance destroy <dbname>");
            Ok(())
        }
    }
}

/// `pgapp instance init` — one pgapp instance per target database:
/// creates a dedicated `pgapp_admin` Postgres role the server operates
/// as from then on, and writes the local instance file
/// (`~/.pgapp/instances/<dbname>.json`) that gates future
/// instance/workspace/app commands. See `src/instance.rs` for exactly
/// what is and isn't persisted.
async fn instance_init() -> anyhow::Result<()> {
    println!("Let's set up a pgapp instance.");
    println!();
    let conn = scaffold::prompt(
        "Postgres connection string (a superuser-capable role)",
        "postgres://postgres:postgres@localhost:5432/postgres",
    )?;
    let dbname = scaffold::prompt_required("Database name for this pgapp instance")?;

    let opts: PgConnectOptions = conn.parse().context("not a valid Postgres connection string")?;
    let opts = opts.database(&dbname);
    let pool = connect_with_auto_create(opts.clone()).await?;

    let admin_password = scaffold::prompt_required(&format!("Password to set for the new '{}' Postgres role", instance::ADMIN_ROLE))?;
    ensure_role(&pool, instance::ADMIN_ROLE, &admin_password).await?;
    // CREATEROLE so `pgapp workspace create`'s "new schema" path can
    // provision that workspace's own owning role day-to-day, without
    // needing a fresh superuser connection for a perfectly routine
    // (non-destructive) operation — unlike DROP SCHEMA/ROLE at destroy
    // time, which always asks for one fresh.
    sqlx::raw_sql(&format!("alter role {} createrole", instance::ADMIN_ROLE))
        .execute(&pool)
        .await
        .with_context(|| format!("failed to grant CREATEROLE to {}", instance::ADMIN_ROLE))?;
    // CREATE ON DATABASE: schema creation itself (CREATE SCHEMA) is a
    // database-level privilege, separate from anything GRANT ON SCHEMA
    // covers — without this, `workspace create`'s "new schema" branch
    // couldn't create one.
    sqlx::raw_sql(&format!("grant create on database \"{dbname}\" to {}", instance::ADMIN_ROLE))
        .execute(&pool)
        .await
        .with_context(|| format!("failed to grant CREATE ON DATABASE to {}", instance::ADMIN_ROLE))?;

    control::ensure_schema(&pool).await?;
    meta::ensure_schema(&pool).await?;
    for schema in ["pgapp_meta", "pgapp_data", "pgapp_control"] {
        grant_admin_on_schema(&pool, schema).await?;
    }

    let cli_password = scaffold::prompt_required("Set a local pgapp CLI admin password (gates future instance/workspace/app commands)")?;
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();
    let instance_file = instance::InstanceFile {
        dbname: dbname.clone(),
        host: opts.get_host().to_string(),
        port: opts.get_port(),
        admin_role: instance::ADMIN_ROLE.to_string(),
        admin_password_hash: instance::hash_password(&cli_password)?,
        created_at,
    };
    instance::save(&instance_file)?;

    println!();
    println!("pgapp instance '{dbname}' is ready.");
    println!("Every future instance/workspace/app/run command against it needs:");
    println!("  export PGAPP_ADMIN_DB_PASSWORD=<the password you just set for '{}'>", instance::ADMIN_ROLE);
    println!("Next: `pgapp workspace create {dbname}`, or `pgapp run <file>.pgapp --instance {dbname}` to serve an app in the classic global schema.");
    Ok(())
}

/// `pgapp instance destroy <dbname>` — always a hard delete: drops
/// every workspace schema/role pgapp itself created, pgapp_meta/
/// pgapp_data/pgapp_control, the `pgapp_admin` role, and the local
/// instance file. Needs a superuser-capable connection supplied fresh
/// (never the stored `pgapp_admin` credential, which can't drop its
/// own role or schemas it doesn't own).
async fn instance_destroy(dbname: &str) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;

    println!("This permanently destroys pgapp instance '{dbname}':");
    println!("every workspace schema/role pgapp created, pgapp_meta/pgapp_data/pgapp_control, and the '{}' role.", inst.admin_role);
    let conn = scaffold::prompt(
        "Superuser-capable connection string to perform this (never stored)",
        &format!("postgres://postgres:postgres@{}:{}/{}", inst.host, inst.port, inst.dbname),
    )?;
    let opts: PgConnectOptions = conn.parse().context("not a valid Postgres connection string")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .context("failed to connect with the given credentials")?;

    let confirm = scaffold::prompt_required(&format!("Type the database name '{dbname}' to confirm"))?;
    if confirm != dbname {
        anyhow::bail!("confirmation did not match '{dbname}' — aborted, nothing was destroyed");
    }

    for ws in control::list_workspaces(&pool).await.unwrap_or_default() {
        try_drop(&pool, &format!("drop schema if exists {} cascade", ws.schema_name), &format!("schema '{}'", ws.schema_name)).await;
        if let Some(role) = ws.owner_role {
            try_drop(&pool, &format!("drop role if exists {role}"), &format!("role '{role}'")).await;
        }
    }
    try_drop(&pool, "drop schema if exists pgapp_control cascade", "schema 'pgapp_control'").await;
    try_drop(&pool, "drop schema if exists pgapp_meta cascade", "schema 'pgapp_meta'").await;
    try_drop(&pool, "drop schema if exists pgapp_data cascade", "schema 'pgapp_data'").await;
    // DROP ROLE refuses while the role still holds *any* privilege
    // anywhere in this database (e.g. the CREATE ON DATABASE grant from
    // instance init) — not just object ownership, which the schema
    // drops above already cleared. DROP OWNED BY revokes all of that in
    // one step, the standard idiom for actually being able to drop a
    // role afterward.
    try_drop(
        &pool,
        &format!("drop owned by {}", inst.admin_role),
        &format!("privileges owned by '{}'", inst.admin_role),
    )
    .await;
    try_drop(&pool, &format!("drop role if exists {}", inst.admin_role), &format!("role '{}'", inst.admin_role)).await;

    instance::delete_file(dbname)?;
    println!("Instance '{dbname}' destroyed.");
    Ok(())
}

async fn cmd_workspace(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("create") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp workspace create <dbname>"))?;
            workspace_create(&dbname).await
        }
        Some("destroy") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp workspace destroy <dbname> <slug> [--hard|--soft]"))?;
            let slug = args
                .get(2)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp workspace destroy <dbname> <slug> [--hard|--soft]"))?;
            let hard = args.iter().any(|a| a == "--hard");
            let soft = args.iter().any(|a| a == "--soft");
            workspace_destroy(&dbname, &slug, hard, soft).await
        }
        Some("list") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp workspace list <dbname>"))?;
            workspace_list(&dbname).await
        }
        _ => {
            println!("usage: pgapp workspace create|destroy|list <dbname> ...");
            Ok(())
        }
    }
}

async fn workspace_create(dbname: &str) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let slug = scaffold::prompt_required("Workspace name")?;
    if !instance::valid_identifier(&slug) {
        anyhow::bail!("'{slug}' must start with a letter/underscore and contain only letters, digits, underscores");
    }
    if control::find_workspace(&pool, &slug).await?.is_some() {
        anyhow::bail!("workspace '{slug}' already exists");
    }

    let kind = scaffold::prompt("Existing schema, or create a new one? [new/existing]", "new")?;
    if kind.eq_ignore_ascii_case("existing") {
        let schema_name = scaffold::prompt_required("Existing schema name")?;
        // pg_namespace, not information_schema.schemata: the latter only
        // lists schemas the connecting role already has some privilege
        // on, which pgapp_admin by definition doesn't yet for a schema
        // it's about to ask permission to use.
        let exists: bool = sqlx::query_scalar("select exists(select 1 from pg_catalog.pg_namespace where nspname = $1)")
            .bind(&schema_name)
            .fetch_one(&pool)
            .await?;
        if !exists {
            anyhow::bail!("schema '{schema_name}' does not exist");
        }
        let grant = scaffold::prompt_yes_no(
            &format!("Grant {} USAGE+CREATE access to schema '{schema_name}'?", instance::ADMIN_ROLE),
            true,
        )?;
        if grant {
            // pgapp_admin has no privileges of its own on a schema it
            // didn't create — granting them requires whoever *does*
            // own/administer that schema, supplied fresh here (never
            // stored), the same as every other elevated operation.
            let conn = scaffold::prompt(
                &format!("A connection that can GRANT on schema '{schema_name}' (never stored)"),
                &format!("postgres://postgres:postgres@{}:{}/{}", inst.host, inst.port, inst.dbname),
            )?;
            let opts: PgConnectOptions = conn.parse().context("not a valid Postgres connection string")?;
            let grantor_pool = PgPoolOptions::new()
                .max_connections(2)
                .connect_with(opts)
                .await
                .context("failed to connect with the given credentials")?;
            grant_admin_on_schema(&grantor_pool, &schema_name).await?;
        }
        control::register_workspace(&pool, &slug, &schema_name, None).await?;
        println!("Workspace '{slug}' registered against existing schema '{schema_name}'.");
    } else {
        let password = scaffold::prompt_required(&format!("Password for the new schema-owning role '{slug}'"))?;
        ensure_role(&pool, &slug, &password).await?;
        // CREATE SCHEMA ... AUTHORIZATION <role> requires being able to
        // SET ROLE to it (Postgres won't let you authorize a schema to
        // a role you aren't a member of) — pgapp_admin needs membership
        // in the workspace role it just created before it can do this.
        sqlx::raw_sql(&format!("grant {slug} to {}", instance::ADMIN_ROLE))
            .execute(&pool)
            .await
            .with_context(|| format!("failed to grant membership in '{slug}' to {}", instance::ADMIN_ROLE))?;
        sqlx::raw_sql(&format!("create schema if not exists {slug} authorization {slug}"))
            .execute(&pool)
            .await
            .with_context(|| format!("failed to create schema '{slug}'"))?;
        grant_admin_on_schema(&pool, &slug).await?;
        control::register_workspace(&pool, &slug, &slug, Some(&slug)).await?;
        println!("Workspace '{slug}' created with its own schema and role.");
    }
    Ok(())
}

async fn workspace_destroy(dbname: &str, slug: &str, hard: bool, soft: bool) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let ws = control::find_workspace(&pool, slug)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered"))?;

    let do_hard = if hard {
        true
    } else if soft {
        false
    } else {
        scaffold::prompt("Hard delete (drop schema+role) or soft disable? [hard/soft]", "soft")?.eq_ignore_ascii_case("hard")
    };

    if !do_hard {
        control::disable_workspace(&pool, slug).await?;
        println!("Workspace '{slug}' disabled (soft) — its schema and data are untouched.");
        return Ok(());
    }

    let app_count = control::workspace_app_count(&pool, ws.id).await?;
    if app_count > 0 {
        println!("Workspace '{slug}' still has {app_count} app(s) registered — destroying it destroys their data too.");
        let confirm = scaffold::prompt_required(&format!("Type '{slug}' to confirm"))?;
        if confirm != slug {
            anyhow::bail!("confirmation did not match — aborted");
        }
    }

    let conn = scaffold::prompt(
        "Superuser-capable connection string to drop the schema/role (never stored)",
        &format!("postgres://postgres:postgres@{}:{}/{}", inst.host, inst.port, inst.dbname),
    )?;
    let opts: PgConnectOptions = conn.parse().context("not a valid Postgres connection string")?;
    let admin_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .context("failed to connect with the given credentials")?;

    try_drop(&admin_pool, &format!("drop schema if exists {} cascade", ws.schema_name), &format!("schema '{}'", ws.schema_name)).await;
    if let Some(role) = &ws.owner_role {
        try_drop(&admin_pool, &format!("drop role if exists {role}"), &format!("role '{role}'")).await;
    }
    control::delete_workspace_row(&pool, slug).await?;
    println!("Workspace '{slug}' destroyed.");
    Ok(())
}

async fn workspace_list(dbname: &str) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let workspaces = control::list_workspaces(&pool).await?;
    if workspaces.is_empty() {
        println!("no workspaces registered yet — `pgapp workspace create {dbname}`");
        return Ok(());
    }
    for ws in workspaces {
        println!(
            "{}\t{}\tschema={}\towner_role={}",
            ws.slug,
            if ws.enabled { "enabled" } else { "disabled" },
            ws.schema_name,
            ws.owner_role.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

/// The "if no clue, list available workspaces and ask to choose"
/// picker — used by both `pgapp app create` and `pgapp run` when
/// `--workspace` wasn't given.
async fn pick_workspace(pool: &PgPool, dbname: &str) -> anyhow::Result<control::WorkspaceRow> {
    let workspaces = control::list_workspaces(pool).await?;
    let enabled: Vec<_> = workspaces.into_iter().filter(|w| w.enabled).collect();
    if enabled.is_empty() {
        anyhow::bail!("no workspaces registered yet — run `pgapp workspace create {dbname}` first");
    }
    println!("Available workspaces:");
    for (i, ws) in enabled.iter().enumerate() {
        println!("  {}. {} (schema: {})", i + 1, ws.slug, ws.schema_name);
    }
    let choice = scaffold::prompt_required("Which workspace? (number or name)")?;
    if let Ok(idx) = choice.parse::<usize>() {
        if idx >= 1 && idx <= enabled.len() {
            return Ok(enabled.into_iter().nth(idx - 1).expect("bounds checked above"));
        }
    }
    enabled
        .into_iter()
        .find(|w| w.slug == choice)
        .ok_or_else(|| anyhow::anyhow!("'{choice}' isn't one of the listed workspaces"))
}

async fn cmd_app(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("create") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp app create <dbname> [--workspace <slug>]"))?;
            app_create(&dbname, flag(args, "--workspace")).await
        }
        Some("destroy") => {
            let dbname = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp app destroy <dbname> <slug> [--hard|--soft]"))?;
            let slug = args
                .get(2)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp app destroy <dbname> <slug> [--hard|--soft]"))?;
            let hard = args.iter().any(|a| a == "--hard");
            let soft = args.iter().any(|a| a == "--soft");
            app_destroy(&dbname, &slug, hard, soft).await
        }
        _ => {
            println!("usage: pgapp app create|destroy <dbname> ...");
            Ok(())
        }
    }
}

async fn app_create(dbname: &str, workspace_arg: Option<String>) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let ws = match workspace_arg {
        Some(slug) => control::find_workspace(&pool, &slug)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered (see `pgapp workspace list {dbname}`)"))?,
        None => pick_workspace(&pool, dbname).await?,
    };

    println!("Let's scaffold a new app in workspace '{}'.", ws.slug);
    let name = scaffold::prompt_required("App name")?;
    let theme = scaffold::prompt("Theme (plain/shadcn/vivid/google_m3)", "shadcn")?;
    let as_dir = scaffold::prompt_yes_no("Scaffold as a directory of files instead of one?", false)?;
    let slug = scaffold::slugify(&name);
    let default_target = if as_dir { slug.clone() } else { format!("{slug}.pgapp") };
    let target = scaffold::prompt("Path to write it to", &default_target)?;

    if as_dir {
        scaffold::scaffold_dir(&target, &name, &theme)?;
    } else {
        scaffold::scaffold_file(&target, &name, &theme)?;
    }
    println!("Created {target}");

    let app_def = source::load(&target)?;
    let item_types = item_types::registry();
    let action_registry = actions::registry();
    meta::sync_app(&pool, &app_def, &item_types, &action_registry, &ws.schema_name).await?;
    control::register_in_workspace(&pool, &slug, &target, &app_def.name, ws.id, &ws.schema_name).await?;

    println!("App '{slug}' registered in workspace '{}'. Run it with:", ws.slug);
    println!("  export PGAPP_ADMIN_DB_PASSWORD=<the '{}' role's password>", instance::ADMIN_ROLE);
    println!("  pgapp run {target} --instance {dbname} --workspace {}", ws.slug);
    Ok(())
}

async fn app_destroy(dbname: &str, slug: &str, hard: bool, soft: bool) -> anyhow::Result<()> {
    let inst = instance::load(dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let app = control::find_app(&pool, slug)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no app '{slug}' registered"))?;

    let do_hard = if hard {
        true
    } else if soft {
        false
    } else {
        scaffold::prompt("Hard delete (drop its data tables) or soft disable? [hard/soft]", "soft")?.eq_ignore_ascii_case("hard")
    };

    if !do_hard {
        control::disable(&pool, slug).await?;
        println!("App '{slug}' disabled (soft) — its tables and rows are untouched.");
        return Ok(());
    }

    let confirm = scaffold::prompt_required(&format!("Type '{slug}' to confirm permanently dropping its data tables"))?;
    if confirm != slug {
        anyhow::bail!("confirmation did not match — aborted");
    }

    // pgapp_control.apps is keyed by slug; pgapp_meta.apps is keyed by
    // the app's declared name (app_name, stored at registration time)
    // — that's the join needed to find its entity table names.
    let app_id: Option<i32> = sqlx::query_scalar("select id from pgapp_meta.apps where name = $1")
        .bind(&app.app_name)
        .fetch_optional(&pool)
        .await?;
    if let Some(app_id) = app_id {
        let tables: Vec<String> = sqlx::query_scalar(
            "select table_name from pgapp_meta.entities where app_id = $1 and source_query is null",
        )
        .bind(app_id)
        .fetch_all(&pool)
        .await?;
        for table in tables {
            try_drop(
                &pool,
                &format!("drop table if exists {}.{table} cascade", app.data_schema),
                &format!("table '{}.{table}'", app.data_schema),
            )
            .await;
        }
        sqlx::query("delete from pgapp_meta.apps where id = $1").bind(app_id).execute(&pool).await?;
    }
    control::delete_app_row(&pool, slug).await?;
    println!("App '{slug}' destroyed — its data tables are gone.");
    Ok(())
}

/// `pgapp run <file>.pgapp --instance <dbname> [--workspace <slug>]` —
/// registers/re-points the given app into the chosen workspace, then
/// serves every enabled app in the whole instance (classic-flow apps
/// and every workspace's apps alike), same "the registry decides"
/// behavior the classic flow already has.
async fn cmd_run(args: &[String]) -> anyhow::Result<()> {
    let markup_path = args
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: pgapp run <file>.pgapp --instance <dbname> [--workspace <slug>]"))?;
    let dbname = flag(args, "--instance").ok_or_else(|| anyhow::anyhow!("`pgapp run` needs --instance <dbname>"))?;
    let workspace_arg = flag(args, "--workspace");

    let inst = instance::load(&dbname)?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let ws = match workspace_arg {
        Some(slug) => control::find_workspace(&pool, &slug)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered (see `pgapp workspace list {dbname}`)"))?,
        None => pick_workspace(&pool, &dbname).await?,
    };

    let discovered =
        source::load_workspace(&markup_path).with_context(|| format!("failed to load '{markup_path}'"))?;
    let item_types = item_types::registry();
    let action_registry = actions::registry();
    for (slug, path, app_def) in &discovered {
        meta::sync_app(&pool, app_def, &item_types, &action_registry, &ws.schema_name).await?;
        control::register_in_workspace(&pool, slug, path, &app_def.name, ws.id, &ws.schema_name).await?;
    }

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    serve_registered_apps(pool, &bind_addr).await
}
