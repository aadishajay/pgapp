mod markup;
mod meta;
mod model;
mod render;
mod server;
mod theme;

use std::sync::Arc;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let markup_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/todo.app".to_string());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/pgapp".to_string());
    let theme_name = std::env::var("PGAPP_THEME").unwrap_or_else(|_| "shadcn".to_string());
    let theme = theme::load(&theme_name)?;

    let src = std::fs::read_to_string(&markup_path)
        .with_context(|| format!("failed to read markup file '{markup_path}'"))?;
    let app_def = markup::parse_app(&src)
        .with_context(|| format!("failed to parse markup file '{markup_path}'"))?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .with_context(|| format!("failed to connect to database '{database_url}'"))?;

    meta::ensure_schema(&pool).await?;
    meta::sync_app(&pool, &app_def).await?;
    let runtime_app = meta::load_app(&pool, &app_def.name).await?;
    let runtime_js = meta::load_runtime_js(&pool, &app_def.name).await?;

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
    for page in &runtime_app.pages {
        match &page.entity {
            Some(entity) => println!(
                "  http://{bind_addr}/{}  ({}, entity: {}, table: pgapp_data.{})",
                page.name,
                page.kind.as_str(),
                entity.name,
                entity.table_name
            ),
            None => println!(
                "  http://{bind_addr}/{}  ({})",
                page.name,
                page.kind.as_str()
            ),
        }
    }

    let state = Arc::new(server::AppState {
        pool,
        app: runtime_app,
        theme,
        runtime_js,
    });
    let router = server::build_router(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;
    axum::serve(listener, router).await?;

    Ok(())
}
