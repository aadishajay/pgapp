//! Turns a pending row in `new_app_requests` (written by the App
//! Builder's own "New App" form — see `examples/app_builder.pgapp`'s
//! "NewApp" page) into a real registered app: scaffolds a `.pgapp`
//! file, syncs it into `pgapp_meta`, and registers it in the target
//! workspace. Wired as a companion Report's `before_load` so it fires
//! on the very next page load after the Form's own create-and-redirect
//! (both land on the same page — see `server.rs`'s `create` handler),
//! giving a one-step "submit and see the result" UX with no new
//! component kind needed.
//!
//! Errors are written back into the request row's own `status`/
//! `result` columns rather than propagated as the usual before_load
//! warning banner — the report showing that row *is* the error
//! display, and it stays visible on every later page load too, not
//! just the one right after submission.

use crate::actions::{ActionContext, BoxFuture, ServerAction};

const THEMES: [&str; 4] = ["plain", "shadcn", "vivid", "google_m3"];

// `meta::sync_app` always names an entity's physical table
// `<app-slug>_<entity-slug>` (see meta/sync.rs's own `slug` + its
// doc), never the bare declared entity name — "new_app_requests" in
// markup, but this on disk. The App Builder's own app name/entity
// name are both fixed (see examples/app_builder.pgapp), so this never
// drifts; a generic lookup through pgapp_meta would be needless
// ceremony for a fact this static.
const REQUESTS_TABLE: &str = "app_builder_new_app_requests";

pub struct CreateApp;

impl ServerAction for CreateApp {
    fn name(&self) -> &'static str {
        "create_app"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let schema = &ctx.app.data_schema;
            let pending: Option<(i32, String, String, String)> = sqlx::query_as(&format!(
                "select id, app_name, workspace_slug, theme from {schema}.{REQUESTS_TABLE} \
                 where status = 'pending' order by id limit 1"
            ))
            .fetch_optional(ctx.pool)
            .await?;

            let Some((id, app_name, workspace_slug, theme)) = pending else {
                return Ok(String::new());
            };

            let result = create_one(ctx.pool, &app_name, &workspace_slug, &theme).await;
            let (status, message) = match result {
                // Registered in pgapp_control, but `serve_registered_apps`
                // only reads that registry once, at process startup — a
                // running server has no live "watch for new apps" path
                // (unlike an *existing* served app's own hot-reload/
                // reorder routes, which mutate an already-running
                // AppEntry in place). So this is honest about the one
                // remaining manual step rather than implying it's live.
                Ok(path) => ("done", format!("Created and registered — restart `pgapp run` to start serving it at {path}")),
                Err(e) => ("error", format!("{e:#}")),
            };
            sqlx::query(&format!("update {schema}.{REQUESTS_TABLE} set status = $1, result = $2 where id = $3"))
                .bind(status)
                .bind(&message)
                .bind(id)
                .execute(ctx.pool)
                .await?;
            Ok(String::new())
        })
    }
}

async fn create_one(pool: &sqlx::PgPool, app_name: &str, workspace_slug: &str, theme: &str) -> anyhow::Result<String> {
    let app_name = app_name.trim();
    if app_name.is_empty() {
        anyhow::bail!("app name can't be empty");
    }
    if !THEMES.contains(&theme) {
        anyhow::bail!("'{theme}' isn't a known theme ({})", THEMES.join(", "));
    }
    // The workspace picker already excludes it, but a request row can
    // be hand-crafted past whatever a picker itself declines to offer
    // — belt and suspenders, same reasoning as the reorder route's own
    // self-edit guard in server.rs.
    if workspace_slug == crate::instance::APP_BUILDER_WORKSPACE_SLUG {
        anyhow::bail!("the '{workspace_slug}' workspace is reserved for the App Builder itself");
    }
    let ws = crate::control::find_workspace(pool, workspace_slug)
        .await?
        .filter(|w| w.enabled)
        .ok_or_else(|| anyhow::anyhow!("no enabled workspace '{workspace_slug}'"))?;

    let slug = crate::scaffold::slugify(app_name);
    if crate::control::find_app(pool, &slug, Some(workspace_slug)).await?.is_some() {
        anyhow::bail!("an app named '{slug}' already exists in workspace '{workspace_slug}'");
    }

    // An absolute, instance-managed path — not CWD-relative like a
    // human's own `pgapp new` output — so the new app stays reachable
    // regardless of which directory a later `pgapp run` happens to be
    // launched from, same reasoning as the App Builder's own markup
    // file (see main.rs's `provision_app_builder`).
    let target = crate::instance::home_dir()?.join(format!("{slug}.pgapp"));
    let target = target.to_string_lossy().to_string();
    crate::scaffold::scaffold_file(&target, app_name, theme)?;

    let app_def = crate::source::load(&target)?;
    let item_types = crate::item_types::registry();
    let action_registry = crate::actions::registry();
    crate::meta::sync_app(pool, &app_def, &item_types, &action_registry, &ws.schema_name).await?;
    crate::control::register_in_workspace(pool, &slug, &target, &app_def.name, ws.id, &ws.schema_name).await?;

    Ok(format!("/{workspace_slug}/{slug}"))
}
