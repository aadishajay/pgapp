//! The App Builder's "New App" scaffold/sync/register logic — plain
//! functions, not a `ServerAction`: creating a brand-new app needs to
//! hot-register it into the *running* server's `AppState` (see
//! `server::AppState::register_app`) so it's servable immediately, no
//! `pgapp run` restart needed, and action modules have no `AppState`
//! access (`ActionContext` only ever carries `pool`/`app`/`page`/
//! `config`/`values`/`caller_key` — see `actions.rs`'s doc). So this
//! is called directly from a real route,
//! `admin_create_pending_app` in `server.rs`, which does have it.
//!
//! The Form on `examples/app_builder.pgapp`'s "NewApp" page writes a
//! pending row into `new_app_requests` — a durable request log, not
//! just scratch state — and `runtime.js`'s `bindNewAppProcessing`
//! triggers that route on every load of the NewApp page (a POST with
//! no body), same "fires right after submission" UX a `before_load`
//! action would give, without needing one. Errors land back in that
//! row's own `status`/`result` columns rather than a page-level
//! warning, so they stay visible on every later load too.

const THEMES: [&str; 4] = ["plain", "shadcn", "vivid", "google_m3"];

// `meta::sync_app` names an entity's physical table after its own
// slug (see meta/sync.rs's `slug`) — "new_app_requests" both in markup
// and on disk. The App Builder's own reserved schema (see
// `instance::APP_BUILDER_WORKSPACE_SLUG`'s doc / `main.rs`'s
// `provision_app_builder`) is fixed, so this never drifts; a generic
// lookup through pgapp_meta would be needless ceremony for a fact this
// static.
pub const REQUESTS_TABLE: &str = "pgapp_builder.new_app_requests";

/// What `create_one` needs to hand back so its caller can both update
/// the request row and hot-register the new app into `AppState`.
pub struct CreatedApp {
    pub key: String, // "<workspace_slug>/<slug>" — AppState::apps' own key shape
    pub markup_path: String,
    pub data_schema: String,
    pub control_app_id: i32,
    pub workspace_id: Option<i32>,
}

/// Reads the oldest pending request (if any) and processes it —
/// scaffold, sync, register. Returns `Ok(None)` when there's nothing
/// pending (the common case on most page loads): the caller should
/// treat that as a no-op, not an error.
pub async fn process_oldest_pending(pool: &sqlx::PgPool) -> anyhow::Result<Option<(i32, anyhow::Result<CreatedApp>)>> {
    let pending: Option<(i32, String, String, String)> = sqlx::query_as(&format!(
        "select id, app_name, workspace_slug, theme from {REQUESTS_TABLE} where status = 'pending' order by id limit 1"
    ))
    .fetch_optional(pool)
    .await?;
    let Some((id, app_name, workspace_slug, theme)) = pending else {
        return Ok(None);
    };
    let result = create_one(pool, &app_name, &workspace_slug, &theme).await;
    Ok(Some((id, result)))
}

async fn create_one(pool: &sqlx::PgPool, app_name: &str, workspace_slug: &str, theme: &str) -> anyhow::Result<CreatedApp> {
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
    // A retry of a request that scaffolded this file but failed at a
    // later step (sync, register, ...) shouldn't have to fight
    // `scaffold_file`'s own clobber guard — the file it already wrote
    // is exactly what a fresh scaffold would write again, so just load
    // it instead of bailing with "already exists" on every subsequent
    // attempt.
    if !std::path::Path::new(&target).exists() {
        crate::scaffold::scaffold_file(&target, app_name, theme)?;
    }

    let app_def = crate::source::load(&target)?;
    let item_types = crate::item_types::registry();
    let action_registry = crate::actions::registry();
    crate::meta::sync_app(pool, &app_def, &item_types, &action_registry, &ws.schema_name).await?;
    crate::control::register_in_workspace(pool, &slug, &target, &app_def.name, ws.id, &ws.schema_name).await?;
    let control_app = crate::control::find_app(pool, &slug, Some(workspace_slug))
        .await?
        .ok_or_else(|| anyhow::anyhow!("registered '{slug}' but couldn't read it back from pgapp_control"))?;

    Ok(CreatedApp {
        key: format!("{workspace_slug}/{slug}"),
        markup_path: target,
        data_schema: ws.schema_name,
        control_app_id: control_app.id,
        workspace_id: Some(ws.id),
    })
}
