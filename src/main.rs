mod actions;
mod chart_lib;
mod html;
mod icons;
mod item_types;
mod markup;
mod meta;
mod model;
mod render;
mod server;
mod source;
mod theme;

use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let markup_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/todo.pgapp".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/pgapp".to_string());
    // A single .pgapp file, or a directory of them merged into one app
    // (see src/source.rs).
    let app_def = source::load(&markup_path)?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .with_context(|| format!("failed to connect to database '{database_url}'"))?;

    let item_types = item_types::registry();
    let action_registry = actions::registry();

    meta::ensure_schema(&pool).await?;
    meta::sync_app(&pool, &app_def, &item_types, &action_registry).await?;
    let runtime_app = meta::load_app(&pool, &app_def.name).await?;
    let runtime_js = meta::load_runtime_js(&pool, &app_def.name).await?;

    // Theme / icons / chart library are app settings declared in the
    // markup (`theme: vivid`, ...) and reloaded from pgapp_meta like
    // everything else — not environment variables.
    let theme = theme::load(runtime_app.theme.as_deref().unwrap_or("shadcn"))?;
    let icons = icons::load(runtime_app.icons.as_deref().unwrap_or("builtin"))?;
    let chart_lib = chart_lib::load(runtime_app.chart_lib.as_deref().unwrap_or("inline"))?;

    println!("pgapp: serving '{}' from {}", runtime_app.name, markup_path);
    println!(
        "  theme: {} ({}) - {}",
        theme.name,
        if theme.meta.label.is_empty() {
            "no label"
        } else {
            &theme.meta.label
        },
        theme.meta.description
    );
    println!("  icons: {}", icons.name);
    println!("  chart library: {}", chart_lib.name);
    println!(
        "  auth: {}",
        if runtime_app.auth_enabled {
            "enabled (first visit to /login creates the admin account)"
        } else {
            "disabled (no `auth { }` block in the markup)"
        }
    );
    println!("  hot reload: http://{bind_addr}/admin/reload (re-syncs the markup file without restarting)");
    for page in &runtime_app.pages {
        let kinds: Vec<&str> = page
            .components
            .iter()
            .map(|c| match c {
                crate::meta::RuntimeComponent::Report { .. } => "report",
                crate::meta::RuntimeComponent::Form { .. } => "form",
                crate::meta::RuntimeComponent::EditableTable { .. } => "editable_table",
                crate::meta::RuntimeComponent::Chart { .. } => "chart",
                crate::meta::RuntimeComponent::Text { .. } => "text",
                crate::meta::RuntimeComponent::Link { .. } => "link",
                crate::meta::RuntimeComponent::Region { .. } => "region",
                crate::meta::RuntimeComponent::Action { .. } => "action",
                crate::meta::RuntimeComponent::DynamicAction { .. } => "dynamic_action",
            })
            .collect();
        println!("  http://{bind_addr}/{}  [{}]", page.name, kinds.join(", "));
    }

    let state = Arc::new(server::AppState {
        pool,
        markup_path,
        data: std::sync::RwLock::new(Arc::new(server::AppData {
            app: runtime_app,
            theme,
            runtime_js,
            icons,
            chart_lib,
        })),
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
