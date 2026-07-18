use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;

use pgapp::{actions, chart_lib, control, icons, item_types, meta, scaffold, server, source, theme};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli_args: Vec<String> = std::env::args().collect();
    if matches!(cli_args.get(1).map(|s| s.as_str()), Some("new") | Some("create")) {
        return scaffold::run(&cli_args[2..]).await;
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

    let markup_path = cli_args.get(1).cloned().unwrap_or_else(|| "examples/todo.pgapp".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
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
    for (slug, path, _app_def) in &discovered {
        control::register(&pool, slug, path).await?;
    }

    let item_types = item_types::registry();
    let action_registry = actions::registry();

    let registered = control::list_enabled(&pool).await?;
    let mut apps: HashMap<String, server::AppEntry> = HashMap::new();
    for (slug, path) in registered {
        match load_one_app(&pool, &path, &item_types, &action_registry).await {
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

    print_banner(&bind_addr, &apps).await;

    let state = Arc::new(server::AppState {
        pool,
        apps,
        item_types,
        actions: action_registry,
    });
    let router = server::build_router(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;
    axum::serve(listener, router).await?;

    Ok(())
}

/// Parses, syncs, and loads one app into a fresh [`server::AppEntry`] —
/// exactly what every app registered in `pgapp_control.apps` goes
/// through on every server start (and again, for just this one app, on
/// its own `/{app}/admin/reload`).
async fn load_one_app(
    pool: &sqlx::PgPool,
    markup_path: &str,
    item_types: &item_types::Registry,
    action_registry: &actions::Registry,
) -> anyhow::Result<server::AppEntry> {
    let app_def = source::load(markup_path)?;
    meta::sync_app(pool, &app_def, item_types, action_registry).await?;
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

async fn control_pool(database_url: &str) -> anyhow::Result<sqlx::PgPool> {
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
    for (slug, path, enabled) in apps {
        println!("{slug}\t{}\t{path}", if enabled { "enabled" } else { "disabled" });
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
