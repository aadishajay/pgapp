use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

use pgapp::{actions, control, instance, item_types, meta, scaffold, secrets, server, source};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    match cli_args.get(1).map(|s| s.as_str()) {
        Some("new") | Some("create") => return scaffold::run(&cli_args[2..]).await,
        Some("instance") => return cmd_instance(&cli_args[2..]).await,
        Some("workspace") => return cmd_workspace(&cli_args[2..]).await,
        Some("app") => return cmd_app(&cli_args[2..]).await,
        Some("secret") => return cmd_secret(&cli_args[2..]).await,
        Some("run") => return cmd_run(&cli_args[2..]).await,
        _ => {
            print_usage();
            Ok(())
        }
    }
}

fn print_usage() {
    println!("pgapp — a Postgres-native, database-backed application server.");
    println!();
    println!("usage:");
    println!("  pgapp new|create [<AppName>] [path] [--dir] [--theme <name>]   scaffold a .pgapp file");
    println!("  pgapp instance init | destroy                                 one instance, globally, per machine");
    println!("  pgapp workspace create [--schema <name>] [--slug <slug>]      a schema an app's tables live in");
    println!("  pgapp workspace destroy <slug> | list");
    println!("  pgapp app create [--workspace <slug>] [--slug <app-slug>]     scaffold/register an app in a workspace");
    println!("  pgapp app destroy <slug> | list");
    println!("  pgapp secret set|list|rm <name> (--workspace <slug> | --app <slug>)");
    println!("  pgapp run <file>.pgapp [--workspace <slug>]                   serve every registered app");
    println!();
    println!("Every app lives in exactly one workspace's schema — see README's \"Instance mode\" section.");
    println!("Start with `pgapp instance init`.");
}

/// The shared tail of `pgapp run`: load every enabled row in
/// `pgapp_control.apps` across every workspace (each already knows its
/// own `data_schema`), print the banner, and serve. One bad app is
/// skipped with a warning rather than taking the whole process down.
///
/// [`server::AppState::apps`] is keyed by `"<workspace_slug>/<slug>"`,
/// not just `slug` — the app's full URL path prefix — so two apps of
/// the same slug in different workspaces route independently instead
/// of colliding (see `build_router`'s `/:workspace/:app` routes).
async fn serve_registered_apps(pool: PgPool, bind_addr: &str) -> anyhow::Result<()> {
    let item_types = item_types::registry();
    let action_registry = actions::registry();

    let registered = control::list_enabled(&pool).await?;
    let mut apps: HashMap<String, Arc<server::AppEntry>> = HashMap::new();
    for (control_app_id, slug, path, data_schema, workspace_id, workspace_slug) in registered {
        let Some(workspace_slug) = workspace_slug else {
            println!("pgapp: warning: skipping app '{slug}' at '{path}' — its workspace no longer exists");
            continue;
        };
        let key = format!("{workspace_slug}/{slug}");
        match server::AppEntry::load(&pool, &path, &data_schema, control_app_id, workspace_id, &item_types, &action_registry).await {
            Ok(entry) => {
                apps.insert(key, Arc::new(entry));
            }
            Err(e) => {
                println!("pgapp: warning: skipping app '{key}' at '{path}' — {e:#}");
            }
        }
    }
    if apps.is_empty() {
        anyhow::bail!("no registered app could be loaded — see the warnings above");
    }

    print_banner(bind_addr, &apps).await;

    let state = Arc::new(server::AppState { pool, apps: std::sync::RwLock::new(apps), item_types, actions: action_registry });
    let router = server::build_router(state);
    // Wraps the whole `Router` from the outside (rather than via its own
    // `.layer()`) so a trailing slash is stripped before route matching
    // runs, not after — matching `/:workspace/:app/` to the same route as
    // `/:workspace/:app` instead of 404ing.
    use axum::ServiceExt;
    use tower::Layer;
    let app = tower_http::normalize_path::NormalizePathLayer::trim_trailing_slash().layer(router);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;
    let make_service =
        <_ as ServiceExt<axum::extract::Request>>::into_make_service(app);
    axum::serve(listener, make_service).await?;
    Ok(())
}

async fn print_banner(bind_addr: &str, apps: &HashMap<String, Arc<server::AppEntry>>) {
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
                    meta::RuntimeComponent::DynamicContent { .. } => "dynamic_content",
                    meta::RuntimeComponent::Action { .. } => "action",
                    meta::RuntimeComponent::Button { .. } => "button",
                    meta::RuntimeComponent::DynamicAction { .. } => "dynamic_action",
                    meta::RuntimeComponent::Calendar { .. } => "calendar",
                    meta::RuntimeComponent::Map { .. } => "map",
                    meta::RuntimeComponent::FacetedSearch { .. } => "faceted_search",
                })
                .collect();
            println!("  http://{bind_addr}/{slug}/{}  [{}]", page.name, kinds.join(", "));
        }
    }
}

// ============================================================
// Instance mode: `pgapp instance/workspace/app/run` — a durable,
// database-backed deployment with a dedicated `pgapp_admin` Postgres
// role. Every app is registered into exactly one workspace's schema —
// there is no workspace-less/global-schema fallback. See README's
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

// `ensure_role`/`grant_admin_on_schema` live in `control.rs` now,
// shared with the App Builder's "New Workspace" web form
// (`actions::create_workspace`) — see their doc comments there.

/// Connects `opts` and, if the target database doesn't exist yet
/// (Postgres error 3D000), creates it via the same host/credentials
/// against the `postgres` maintenance database and retries once,
/// rather than requiring a manual `createdb` step first.
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
        Some("destroy") => instance_destroy().await,
        _ => {
            println!("usage: pgapp instance init | pgapp instance destroy");
            Ok(())
        }
    }
}

/// The App Builder (see README's "App Builder" section) is available
/// by default on every instance — not an opt-in example the operator
/// has to `workspace create`/`app create` for. Its workspace/app slugs
/// (`instance::APP_BUILDER_WORKSPACE_SLUG`/`APP_BUILDER_APP_SLUG`) are
/// fixed and reserved (a user `workspace create --schema pgapp` just
/// hits the ordinary "workspace already exists" error, since this row
/// is already there), and its schema is `pgapp_builder` — one of
/// pgapp's own bookkeeping schemas, like `pgapp_meta`/`pgapp_control`,
/// not a user data workspace; it owns no tables of its own (every
/// entity in its markup is query-backed).
const APP_BUILDER_SCHEMA: &str = "pgapp_builder";
const APP_BUILDER_MARKUP: &str = include_str!("../examples/app_builder.pgapp");

/// Idempotent: creates `pgapp_builder` if missing, registers its fixed
/// workspace/app rows (an upsert either way), and (re)writes its markup
/// to `<PGAPP_HOME>/app_builder.pgapp` — an absolute path, not CWD-
/// relative like a user's own `app create` output, so the App Builder
/// stays reachable regardless of which directory a later `pgapp run`
/// happens to be invoked from. Called both from `instance_init` (so
/// it's there immediately) and from `cmd_run` (so it self-heals onto
/// any instance that predates this feature, without needing `instance
/// init` to be re-run). A failure here is reported but never fatal —
/// the rest of the instance is fully usable without it.
async fn provision_app_builder(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!("create schema if not exists {APP_BUILDER_SCHEMA}"))
        .execute(pool)
        .await
        .with_context(|| format!("failed to create '{APP_BUILDER_SCHEMA}' schema"))?;
    control::grant_admin_on_schema(pool, APP_BUILDER_SCHEMA).await?;

    let ws_id = control::register_workspace(pool, instance::APP_BUILDER_WORKSPACE_SLUG, APP_BUILDER_SCHEMA, None)
        .await
        .context("failed to register the App Builder's workspace")?;

    let markup_path = instance::home_dir()?.join("app_builder.pgapp");
    std::fs::write(&markup_path, APP_BUILDER_MARKUP)
        .with_context(|| format!("failed to write '{}'", markup_path.display()))?;
    let markup_path = markup_path.to_string_lossy().to_string();

    let app_def = source::load(&markup_path).context("the built-in App Builder markup failed to parse")?;
    let item_types = item_types::registry();
    let action_registry = actions::registry();
    meta::sync_app(pool, &app_def, &item_types, &action_registry, APP_BUILDER_SCHEMA)
        .await
        .context("failed to sync the App Builder into pgapp_meta")?;
    control::register_in_workspace(pool, instance::APP_BUILDER_APP_SLUG, &markup_path, &app_def.name, ws_id, APP_BUILDER_SCHEMA)
        .await
        .context("failed to register the App Builder app")?;
    Ok(())
}

/// `pgapp instance init` — sets up *the* pgapp instance (there's
/// exactly one, globally, per machine — see `src/instance.rs`):
/// creates a dedicated `pgapp_admin` Postgres role the server operates
/// as from then on, and writes the local instance file
/// (`~/.pgapp/instance.json`) that gates future instance/workspace/app
/// commands. Refuses outright if one is already set up — `pgapp
/// instance destroy` first if you want to point at a different
/// database.
async fn instance_init() -> anyhow::Result<()> {
    if instance::exists()? {
        anyhow::bail!("a pgapp instance is already set up on this machine — run `pgapp instance destroy` first if you want to replace it");
    }
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
    control::ensure_role(&pool, instance::ADMIN_ROLE, &admin_password).await?;
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
    // Not `pgapp_data`: every app's entity tables live in a workspace's
    // own schema (`pgapp workspace create`) — there's no workspace-less
    // global schema to grant access to.
    for schema in ["pgapp_meta", "pgapp_control"] {
        control::grant_admin_on_schema(&pool, schema).await?;
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

    // Provisioned through pgapp_admin, not the superuser `pool` used
    // above — every later self-heal call (see `cmd_run`) provisions
    // through pgapp_admin too, and any physical table the App Builder
    // owns (e.g. `new_app_requests`) needs one consistent owner across
    // every provisioning call, or a later `alter table add column`
    // fails with "must be owner of table" the moment table ownership
    // and connecting role disagree.
    let admin_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(
            PgConnectOptions::new()
                .host(opts.get_host())
                .port(opts.get_port())
                .database(&dbname)
                .username(instance::ADMIN_ROLE)
                .password(&admin_password),
        )
        .await
        .context("failed to connect as the newly-created pgapp_admin role")?;
    match provision_app_builder(&admin_pool).await {
        Ok(()) => {}
        Err(e) => println!("warning: could not set up the built-in App Builder: {e:#}"),
    }

    println!();
    println!("pgapp instance '{dbname}' is ready.");
    println!(
        "The App Builder (drag-and-drop page editing — see README) is available at /{}/{}",
        instance::APP_BUILDER_WORKSPACE_SLUG, instance::APP_BUILDER_APP_SLUG
    );
    println!("Every future instance/workspace/app/run command against it needs:");
    println!("  export PGAPP_ADMIN_DB_PASSWORD=<the password you just set for '{}'>", instance::ADMIN_ROLE);
    println!("Next: `pgapp workspace create` to create a schema for your first app's tables.");
    Ok(())
}

/// `pgapp instance destroy` — always a hard delete: drops every
/// workspace schema/role pgapp itself created, pgapp_meta/
/// pgapp_control, the `pgapp_admin` role, and the local instance file.
/// Needs a superuser-capable connection supplied fresh (never the
/// stored `pgapp_admin` credential, which can't drop its own role or
/// schemas it doesn't own).
async fn instance_destroy() -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let dbname = &inst.dbname;

    println!("This permanently destroys pgapp instance '{dbname}':");
    println!("every workspace schema/role pgapp created, pgapp_meta/pgapp_control, and the '{}' role.", inst.admin_role);
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
    if &confirm != dbname {
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
    // `pgapp_data` is never created by current instance init, but a
    // pre-instance-mode database may still have one — drop it too if so.
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

    instance::delete_file()?;
    println!("Instance '{dbname}' destroyed.");
    Ok(())
}

async fn cmd_workspace(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("create") => workspace_create(flag(args, "--schema"), flag(args, "--slug"), flag(args, "--password")).await,
        Some("destroy") => {
            let slug = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp workspace destroy <slug> [--hard|--soft]"))?;
            let hard = args.iter().any(|a| a == "--hard");
            let soft = args.iter().any(|a| a == "--soft");
            workspace_destroy(&slug, hard, soft).await
        }
        Some("list") => workspace_list().await,
        _ => {
            println!("usage: pgapp workspace create|destroy|list ...");
            Ok(())
        }
    }
}

/// `pgapp workspace create [--schema <name>] [--slug <slug>]` —
/// `schema` is the actual Postgres schema (existing or new; if
/// omitted, prompted for); `slug` is just the short name later
/// commands use to refer to this workspace (`--workspace <slug>`) —
/// optional, defaults to the schema name. Whether the schema is
/// treated as "new" or "existing" is auto-detected (`pg_namespace`),
/// not asked: if it doesn't exist yet, pgapp creates it (and an owning
/// role for it); if it does, pgapp only asks to be granted access.
async fn workspace_create(schema_arg: Option<String>, slug_arg: Option<String>, password_arg: Option<String>) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let schema_name = match schema_arg {
        Some(s) => s,
        None => scaffold::prompt_required("Schema name (an existing schema to use, or a new one to create)")?,
    };
    if !instance::valid_identifier(&schema_name) {
        anyhow::bail!("'{schema_name}' must start with a letter/underscore and contain only letters, digits, underscores");
    }
    let slug = slug_arg.unwrap_or_else(|| schema_name.clone());
    if !instance::valid_identifier(&slug) {
        anyhow::bail!("'{slug}' must start with a letter/underscore and contain only letters, digits, underscores");
    }

    // pg_namespace, not information_schema.schemata: the latter only
    // lists schemas the connecting role already has some privilege
    // on, which pgapp_admin by definition doesn't yet for a schema
    // it's about to ask permission to use.
    let exists: bool = sqlx::query_scalar("select exists(select 1 from pg_catalog.pg_namespace where nspname = $1)")
        .bind(&schema_name)
        .fetch_one(&pool)
        .await?;

    if exists {
        let grant = scaffold::prompt_yes_no(
            &format!("Grant {} USAGE+CREATE access to schema '{schema_name}'?", instance::ADMIN_ROLE),
            true,
        )?;
        if grant {
            // pgapp_admin has no privileges of its own on a schema it
            // didn't create — granting them requires whoever *does*
            // own/administer that schema, supplied fresh here (never
            // stored), the same as every other elevated operation. See
            // `control::create_workspace_existing_schema`'s doc for the
            // shared implementation (also used by the App Builder's
            // "New Workspace" web form).
            let conn = scaffold::prompt(
                &format!("A connection that can GRANT on schema '{schema_name}' (never stored)"),
                &format!("postgres://postgres:postgres@{}:{}/{}", inst.host, inst.port, inst.dbname),
            )?;
            control::create_workspace_existing_schema(&pool, &slug, &schema_name, &conn).await?;
        } else {
            if control::find_workspace(&pool, &slug).await?.is_some() {
                anyhow::bail!("workspace '{slug}' already exists");
            }
            control::register_workspace(&pool, &slug, &schema_name, None).await?;
        }
        println!("Workspace '{slug}' registered against existing schema '{schema_name}'.");
    } else {
        let password = match password_arg {
            Some(p) => p,
            None => scaffold::prompt_required(&format!("Password for the new schema-owning role '{schema_name}'"))?,
        };
        control::create_workspace_new_schema(&pool, &slug, &schema_name, &password).await?;
        println!("Workspace '{slug}' created with new schema '{schema_name}' and its own owning role.");
    }
    Ok(())
}

async fn workspace_destroy(slug: &str, hard: bool, soft: bool) -> anyhow::Result<()> {
    if slug == instance::APP_BUILDER_WORKSPACE_SLUG {
        anyhow::bail!(
            "'{slug}' is the App Builder's own reserved workspace — it's created automatically \
             and can't be destroyed on its own; `pgapp instance destroy` removes it along with \
             everything else."
        );
    }
    let inst = instance::load()?;
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
    control::hard_delete_workspace(&pool, &ws, &conn).await?;
    println!("Workspace '{slug}' destroyed.");
    Ok(())
}

async fn workspace_list() -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let workspaces = control::list_workspaces(&pool).await?;
    if workspaces.is_empty() {
        println!("no workspaces registered yet — `pgapp workspace create`");
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

/// The App Builder's own reserved workspace (see
/// `instance::APP_BUILDER_WORKSPACE_SLUG`'s doc) never holds anyone
/// else's app — the web "New App"/`run` paths already enforce this
/// (`actions::create_app::create_one`), so the CLI equivalents
/// (`app_create`/`cmd_run`) need the same belt-and-suspenders check:
/// a hand-typed `--workspace pgapp` shouldn't succeed just because it
/// bypasses `pick_workspace`'s own listing.
fn reject_app_builder_workspace(slug: &str) -> anyhow::Result<()> {
    if slug == instance::APP_BUILDER_WORKSPACE_SLUG {
        anyhow::bail!("the '{slug}' workspace is reserved for the App Builder itself");
    }
    Ok(())
}

/// The "if no clue, list available workspaces and ask to choose"
/// picker — used by both `pgapp app create` and `pgapp run` when
/// `--workspace` wasn't given. Never offers the App Builder's own
/// reserved workspace as a choice (see `reject_app_builder_workspace`,
/// the explicit safety net for the case a caller passed `--workspace
/// pgapp` directly instead of going through this picker).
async fn pick_workspace(pool: &PgPool) -> anyhow::Result<control::WorkspaceRow> {
    let workspaces = control::list_workspaces(pool).await?;
    let enabled: Vec<_> = workspaces
        .into_iter()
        .filter(|w| w.enabled && w.slug != instance::APP_BUILDER_WORKSPACE_SLUG)
        .collect();
    if enabled.is_empty() {
        anyhow::bail!("no workspaces registered yet — run `pgapp workspace create` first");
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
        Some("create") => app_create(flag(args, "--workspace"), flag(args, "--slug")).await,
        Some("destroy") => {
            let slug = args
                .get(1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("usage: pgapp app destroy <slug> [--workspace <slug>] [--hard|--soft]"))?;
            let hard = args.iter().any(|a| a == "--hard");
            let soft = args.iter().any(|a| a == "--soft");
            app_destroy(&slug, flag(args, "--workspace"), hard, soft).await
        }
        Some("list") => app_list().await,
        _ => {
            println!("usage: pgapp app create|destroy|list ...");
            Ok(())
        }
    }
}

/// `pgapp app list` — every app registered across every workspace in
/// this instance (including disabled ones).
async fn app_list() -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let apps = control::list_all(&pool).await?;
    if apps.is_empty() {
        println!("no apps registered yet — `pgapp app create`");
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

/// `pgapp secret set|list|rm ...` — a workspace- or app-scoped named
/// secret, referenced from markup as `{{secret.<name>}}` (see
/// `src/secrets.rs`). Exactly one of `--workspace <slug>` / `--app
/// <slug>` picks the scope; an app-scoped secret shadows a workspace-
/// scoped one of the same name at resolve time.
async fn cmd_secret(args: &[String]) -> anyhow::Result<()> {
    const USAGE: &str = "usage: pgapp secret set|list|rm <name> (--workspace <slug> | --app <slug>) [--value <value>]\n\
                          (\"set\"'s <name> is required; \"list\" takes no name — omit it to list every secret in scope)";
    match args.first().map(|s| s.as_str()) {
        Some("set") => {
            let name = args.get(1).cloned().ok_or_else(|| anyhow::anyhow!(USAGE))?;
            secret_set(&name, flag(args, "--workspace"), flag(args, "--app"), flag(args, "--value")).await
        }
        Some("list") => secret_list(flag(args, "--workspace"), flag(args, "--app")).await,
        Some("rm") => {
            let name = args.get(1).cloned().ok_or_else(|| anyhow::anyhow!(USAGE))?;
            secret_rm(&name, flag(args, "--workspace"), flag(args, "--app")).await
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

/// Exactly one of `--workspace`/`--app` names the scope; resolved
/// against the control-plane registry (`pgapp_control.workspaces`/
/// `.apps`), not `pgapp_meta`, since that's what `secrets::Scope`
/// keys off of.
async fn secret_scope(pool: &PgPool, workspace_arg: Option<String>, app_arg: Option<String>) -> anyhow::Result<secrets::Scope> {
    match (workspace_arg, app_arg) {
        (Some(_), Some(_)) => anyhow::bail!("pass exactly one of --workspace or --app, not both"),
        (Some(slug), None) => {
            let ws = control::find_workspace(pool, &slug)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered (see `pgapp workspace list`)"))?;
            Ok(secrets::Scope::Workspace(ws.id))
        }
        (None, Some(slug)) => {
            // No `--workspace` alongside `--app` here (mutually
            // exclusive above) — if `slug` happens to be registered in
            // more than one workspace, `find_app` reports exactly that
            // rather than guessing which app the secret belongs to.
            let app = control::find_app(pool, &slug, None)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no app '{slug}' registered"))?;
            Ok(secrets::Scope::App(app.id))
        }
        (None, None) => anyhow::bail!("pass one of --workspace <slug> or --app <slug>"),
    }
}

async fn secret_set(
    name: &str,
    workspace_arg: Option<String>,
    app_arg: Option<String>,
    value_arg: Option<String>,
) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let scope = secret_scope(&pool, workspace_arg, app_arg).await?;
    let key = secrets::load_key()?;
    // `--value` exists for scripts, but it lands in shell history and
    // `ps` like any other argument — the interactive prompt (typed,
    // not masked, same as the CLI operator password prompt) is the
    // one that doesn't.
    let value = match value_arg {
        Some(v) => v,
        None => scaffold::prompt_required(&format!("Value for secret '{name}'"))?,
    };
    secrets::set(&pool, &key, scope, name, &value).await?;
    println!("Secret '{name}' saved.");
    Ok(())
}

async fn secret_list(workspace_arg: Option<String>, app_arg: Option<String>) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let scope = secret_scope(&pool, workspace_arg, app_arg).await?;
    let names = secrets::list(&pool, scope).await?;
    if names.is_empty() {
        println!("(no secrets in this scope)");
    } else {
        for name in names {
            println!("{name}");
        }
    }
    Ok(())
}

async fn secret_rm(name: &str, workspace_arg: Option<String>, app_arg: Option<String>) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;
    let scope = secret_scope(&pool, workspace_arg, app_arg).await?;
    if secrets::remove(&pool, scope, name).await? {
        println!("Secret '{name}' removed.");
    } else {
        println!("No secret '{name}' found in that scope.");
    }
    Ok(())
}

/// `pgapp app create [--workspace <slug>] [--slug <app-slug>]` —
/// `workspace` says which workspace's schema the app's tables live in
/// (prompted with a picker if omitted); `slug` is the app's own URL
/// identifier (`/<workspace>/<slug>/...`, unique within that
/// workspace) — optional, defaults to a slugified version of the app
/// name you enter below.
async fn app_create(workspace_arg: Option<String>, slug_arg: Option<String>) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let ws = match workspace_arg {
        Some(slug) => control::find_workspace(&pool, &slug)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered (see `pgapp workspace list`)"))?,
        None => pick_workspace(&pool).await?,
    };
    reject_app_builder_workspace(&ws.slug)?;

    println!("Let's scaffold a new app in workspace '{}'.", ws.slug);
    let name = scaffold::prompt_required("App name")?;
    let theme = scaffold::prompt("Theme (plain/shadcn/vivid/google_m3/apex_universal)", "shadcn")?;
    let as_dir = scaffold::prompt_yes_no("Scaffold as a directory of files instead of one?", false)?;
    let slug = match slug_arg {
        Some(s) => {
            if !instance::valid_identifier(&s) {
                anyhow::bail!("'{s}' must start with a letter/underscore and contain only letters, digits, underscores");
            }
            s
        }
        None => scaffold::slugify(&name),
    };
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
    println!("  pgapp run {target} --workspace {}", ws.slug);
    Ok(())
}

async fn app_destroy(slug: &str, workspace_arg: Option<String>, hard: bool, soft: bool) -> anyhow::Result<()> {
    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    let app = control::find_app(&pool, slug, workspace_arg.as_deref())
        .await?
        .ok_or_else(|| anyhow::anyhow!("no app '{slug}' registered"))?;

    if app.slug == instance::APP_BUILDER_APP_SLUG && app.workspace_slug.as_deref() == Some(instance::APP_BUILDER_WORKSPACE_SLUG) {
        anyhow::bail!(
            "'{slug}' is the built-in App Builder — it's created automatically and can't be \
             destroyed on its own; `pgapp instance destroy` removes it along with everything else."
        );
    }

    let do_hard = if hard {
        true
    } else if soft {
        false
    } else {
        scaffold::prompt("Hard delete (drop its data tables) or soft disable? [hard/soft]", "soft")?.eq_ignore_ascii_case("hard")
    };

    if !do_hard {
        control::disable(&pool, app.id).await?;
        println!("App '{slug}' disabled (soft) — its tables and rows are untouched.");
        return Ok(());
    }

    let confirm = scaffold::prompt_required(&format!("Type '{slug}' to confirm permanently dropping its data tables"))?;
    if confirm != slug {
        anyhow::bail!("confirmation did not match — aborted");
    }

    control::hard_delete_app(&pool, &app).await?;
    println!("App '{slug}' destroyed — its data tables are gone.");
    Ok(())
}

/// `pgapp run <file>.pgapp [--workspace <slug>]` — registers/re-points
/// the given app into the chosen workspace, then serves every enabled
/// app across every workspace in the instance — the registry, not just
/// this one invocation's markup path, decides what's actually served.
async fn cmd_run(args: &[String]) -> anyhow::Result<()> {
    let markup_path = args
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: pgapp run <file>.pgapp [--workspace <slug>]"))?;
    let workspace_arg = flag(args, "--workspace");

    let inst = instance::load()?;
    instance::verify_operator(&inst)?;
    let pool = instance::connect_as_admin(&inst).await?;

    // Self-heals the App Builder onto any instance that predates this
    // feature (see provision_app_builder's doc) — cheap and idempotent,
    // so it's fine to just always run this rather than tracking
    // whether it's "already been done" separately.
    if let Err(e) = provision_app_builder(&pool).await {
        println!("warning: could not set up the built-in App Builder: {e:#}");
    }

    let ws = match workspace_arg {
        Some(slug) => control::find_workspace(&pool, &slug)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no workspace '{slug}' registered (see `pgapp workspace list`)"))?,
        None => pick_workspace(&pool).await?,
    };
    reject_app_builder_workspace(&ws.slug)?;

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
