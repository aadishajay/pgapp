//! Route handlers and the generic entity CRUD they're built on. Named
//! query execution and everything that depends on it (LOV choices,
//! regions, paginated query rows) lives in `server::query_engine`,
//! which this module just calls into.
//!
//! Every route is scoped under `/:workspace/:app` — the workspace an
//! app is registered into, plus its own URL slug — resolved against
//! [`AppState::apps`] (keyed by that combined `"workspace/app"` string)
//! once per request. A single shared `PgPool` backs every app in the
//! process (see `src/control.rs`); what's per-app is only the in-memory
//! [`AppEntry`] (the reloadable markup-derived snapshot) and the rows
//! that snapshot's queries touch.
//!
//! A page is an ordered list of components, rendered top to bottom by
//! `show` (`GET /:workspace/:app/:page`). `Form` and `EditableTable`
//! are the only *writable* component kinds; both are addressed by
//! their index on the page (`/:workspace/:app/:page/c/:idx/...`) since
//! a page may have more than one.
//! A `Report`'s row actions (Edit/Delete) only appear when the same page
//! also has a `Form` bound to the same entity — `sibling_form_idx`
//! finds it by scanning the page's own components, no extra metadata
//! needed.

pub mod auth;
mod query_engine;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use anyhow::Context as _;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde_json::json;
use sqlx::{PgPool, Row};
use tower::limit::ConcurrencyLimitLayer;

use auth::AuthCtx;

use crate::actions::{self, ActionContext};
use crate::chart_lib::ChartLib;
use crate::html::url_encode;
use crate::icons::Icons;
use crate::instance;
use crate::item_types;
use crate::meta::{self, Chrome, NavNode, RegionRows, RuntimeApp, RuntimeComponent, RuntimeEntity, RuntimePage};
use crate::model::{FieldItem, HtmlAttrs, PreAction};
use crate::markup;
use crate::page_reorder;
use crate::render;
use crate::theme::Theme;
use query_engine::{bind_context, resolve_field_choices, resolve_regions, run_named_query_page, run_named_query_rows};

/// Everything that comes from one app's markup file and `pgapp_meta` —
/// as opposed to `pool`/`item_types`/`actions`, which are shared across
/// every app in the process and can't be "reloaded" per-app without
/// rebuilding the binary. Bundled into one struct so a reload swaps
/// all of it atomically: a request never sees a new `RuntimeApp`
/// paired with a stale `Theme`.
pub struct AppData {
    pub app: RuntimeApp,
    pub theme: Theme,
    pub runtime_js: String,
    pub icons: Icons,
    pub chart_lib: ChartLib,
}

/// One app being served: where its markup lives on disk, and the
/// current reloadable snapshot of what's synced from it. Keyed by
/// `"<workspace_slug>/<slug>"` — its full URL path prefix — in
/// [`AppState::apps`], not just `slug`, so two apps of the same slug
/// in different workspaces route independently instead of colliding.
/// See `src/control.rs` for where the registry itself lives
/// (`pgapp_control.apps`/`.workspaces`).
pub struct AppEntry {
    pub markup_path: String,
    pub data: RwLock<Arc<AppData>>,
}

impl AppEntry {
    /// Loads a `.pgapp` file fresh — parses it, syncs it into
    /// `pgapp_meta`/its workspace schema, and builds the full in-memory
    /// snapshot a served app needs. Used both at process startup
    /// (`main.rs`'s `serve_registered_apps`, for every already-
    /// registered app) and to hot-register a brand-new one the App
    /// Builder just scaffolded (`admin_create_app` below) — the same
    /// loading logic either way, so the two paths can never drift.
    pub async fn load(
        pool: &PgPool,
        markup_path: &str,
        data_schema: &str,
        control_app_id: i32,
        workspace_id: Option<i32>,
        item_types: &item_types::Registry,
        actions: &actions::Registry,
    ) -> anyhow::Result<AppEntry> {
        let app_def = crate::source::load(markup_path)?;
        meta::sync_app(pool, &app_def, item_types, actions, data_schema).await?;
        let mut runtime_app = meta::load_app(pool, &app_def.name).await?;
        runtime_app.control_app_id = control_app_id;
        runtime_app.workspace_id = workspace_id;
        let runtime_js = meta::load_runtime_js(pool, &app_def.name).await?;
        let theme = crate::theme::load(runtime_app.theme.as_deref().unwrap_or("shadcn"))?;
        let icons = crate::icons::load(runtime_app.icons.as_deref().unwrap_or("builtin"))?;
        let chart_lib = crate::chart_lib::load(runtime_app.chart_lib.as_deref().unwrap_or("inline"))?;
        Ok(AppEntry {
            markup_path: markup_path.to_string(),
            data: RwLock::new(Arc::new(AppData { app: runtime_app, theme, runtime_js, icons, chart_lib })),
        })
    }

    /// A cheap snapshot of the current markup-derived state — an Arc
    /// clone, not a copy. Handlers take one of these at the top and use
    /// it for the rest of the request, so a concurrent reload can never
    /// leave a single request looking at a mix of old and new data.
    pub fn data(&self) -> Arc<AppData> {
        self.data.read().unwrap().clone()
    }

    /// Re-parses `markup_path`, re-syncs it into `pgapp_meta` and its
    /// workspace schema (an idempotent upsert — see `meta::sync_app`),
    /// and atomically swaps in the freshly loaded app/theme/runtime.js/
    /// icons/chart_lib. No process restart: in-flight requests keep
    /// whatever snapshot they already took via `data()`, and the very
    /// next request sees the update. If anything here fails (bad
    /// markup, a validation error), the swap never happens and this
    /// app keeps serving its last-good snapshot — other apps in the
    /// same process are entirely unaffected.
    pub async fn reload(&self, pool: &PgPool, item_types: &item_types::Registry, actions: &actions::Registry) -> anyhow::Result<()> {
        let app_def = crate::source::load(&self.markup_path)?;
        // A reload keeps this app in whatever schema it's already
        // synced into — it never migrates an app's data tables to a
        // different schema on its own. Same story for control_app_id/
        // workspace_id: pgapp_control isn't touched by a markup
        // resync, so carry the current snapshot's values forward
        // rather than re-deriving them (load_app doesn't know them at
        // all — see RuntimeApp's doc comment).
        let data_schema = self.data().app.data_schema.clone();
        let control_app_id = self.data().app.control_app_id;
        let workspace_id = self.data().app.workspace_id;
        meta::sync_app(pool, &app_def, item_types, actions, &data_schema).await?;
        let mut app = meta::load_app(pool, &app_def.name).await?;
        app.control_app_id = control_app_id;
        app.workspace_id = workspace_id;
        let runtime_js = meta::load_runtime_js(pool, &app_def.name).await?;
        let theme = crate::theme::load(app.theme.as_deref().unwrap_or("shadcn"))?;
        let icons = crate::icons::load(app.icons.as_deref().unwrap_or("builtin"))?;
        let chart_lib = crate::chart_lib::load(app.chart_lib.as_deref().unwrap_or("inline"))?;
        *self.data.write().unwrap() = Arc::new(AppData { app, theme, runtime_js, icons, chart_lib });
        Ok(())
    }
}

pub struct AppState {
    pub pool: PgPool,
    /// Behind a lock — unlike everything else here, this map is
    /// mutated after startup: `admin_create_app` inserts a brand-new
    /// app the moment the App Builder finishes scaffolding it, so it's
    /// servable immediately, no restart needed (an *existing* app's own
    /// hot-reload/reorder/add/edit/delete routes only ever swap that
    /// one app's own `AppData` — see `AppEntry::reload` — they never
    /// touch this map itself). Values are `Arc`-wrapped so a lookup can
    /// clone one out and drop the read lock immediately, rather than
    /// holding it for an entire request.
    pub apps: std::sync::RwLock<HashMap<String, Arc<AppEntry>>>,
    pub item_types: item_types::Registry,
    pub actions: actions::Registry,
}

impl AppState {
    /// `key` is `"<workspace_slug>/<app_slug>"` — every route handler
    /// builds this from its two leading path segments before looking
    /// an app up (see `build_router`'s `/:workspace/:app/...` routes).
    pub fn app_or_404(&self, key: &str) -> Result<Arc<AppEntry>, AppError> {
        self.apps
            .read()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such app '{key}'")))
    }

    /// Adds (or replaces) one app in the live registry — the hot-
    /// registration half of `admin_create_app`. A plain `insert` either
    /// way: there's no existing entry to preserve anything from when
    /// this is a brand-new app, and overwriting is the right move in
    /// the rare case a slug was already served (stale) and is being
    /// re-registered.
    pub fn register_app(&self, key: String, entry: AppEntry) {
        self.apps.write().unwrap().insert(key, Arc::new(entry));
    }
}

/// Caps how many requests this process processes at once, across every
/// app it serves. Sized to the shared connection pool rather than
/// picked separately: since almost every route needs a pool connection
/// to do anything, admitting more requests than the pool can serve in
/// parallel doesn't add throughput — it just piles up in-flight work
/// (each holding its own row buffers, rendered-HTML strings, etc.) that
/// then queues for the same fixed number of connections anyway. Excess
/// requests wait here (tower's semaphore-backed backpressure) instead
/// of being admitted and immediately blocking on the pool — the same
/// total wait, but without the extra memory pressure from having all
/// of it in flight simultaneously. Confirmed by load-testing
/// examples/nexus-erp: a 1,000-concurrent-request burst against a
/// 20-connection pool spiked RSS from ~11MB to 400-600MB, a floor that
/// then persisted (allocator retention, not a leak, but still real
/// memory the host doesn't get back without a restart).
fn concurrency_limit() -> usize {
    instance::max_connections() as usize
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(landing))
        .route("/:workspace/:app", get(index))
        .route("/:workspace/:app/theme.css", get(theme_css))
        .route("/:workspace/:app/runtime.js", get(runtime_js))
        .route("/:workspace/:app/chart-lib.js", get(chart_lib_js))
        .route("/:workspace/:app/assets/*path", get(asset))
        .route("/:workspace/:app/api/:entity", get(api_list))
        .route("/:workspace/:app/login", get(auth::login_form).post(auth::login))
        .route("/:workspace/:app/setup", post(auth::setup))
        .route("/:workspace/:app/logout", post(auth::logout))
        .route("/:workspace/:app/users", get(auth::users_page).post(auth::users_create))
        .route("/:workspace/:app/users/:id/delete", post(auth::users_delete))
        .route("/:workspace/:app/admin/reload", get(admin_reload_page).post(admin_reload))
        .route("/:workspace/:app/admin/pages-list", get(admin_pages_list))
        .route("/:workspace/:app/admin/pages/:page/reorder", post(admin_reorder_page))
        .route("/:workspace/:app/admin/pages/add", post(admin_add_page))
        .route("/:workspace/:app/admin/pages/:page/delete", post(admin_delete_page))
        .route("/:workspace/:app/admin/pages/:page/rename", post(admin_rename_page))
        .route(
            "/:workspace/:app/admin/pages/:page/components/add",
            post(admin_add_component_source),
        )
        .route(
            "/:workspace/:app/admin/pages/:page/components/:idx/source",
            get(admin_component_source),
        )
        .route("/:workspace/:app/admin/pages/:page/components/:idx/edit", post(admin_edit_component_source))
        .route("/:workspace/:app/admin/pages/:page/components/:idx/delete", post(admin_delete_component))
        .route("/pgapp/builder/admin/apps/create-pending", post(admin_create_pending_app))
        .route("/:workspace/:app/:page", get(show))
        .route("/:workspace/:app/:page/region/:query", get(region_fragment))
        .route("/:workspace/:app/:page/c/:idx/create", post(create))
        .route("/:workspace/:app/:page/c/:idx/update/:id", post(update))
        .route("/:workspace/:app/:page/c/:idx/delete/:id", post(delete))
        .route("/:workspace/:app/:page/c/:idx/run", post(run_action))
        .route("/:workspace/:app/:page/c/:idx/views", post(save_view))
        .route("/:workspace/:app/:page/c/:idx/views/:vid/delete", post(delete_view))
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_login))
        .with_state(state)
        .layer(ConcurrencyLimitLayer::new(concurrency_limit()))
}

/// Gate for `/:workspace/:app/admin/reload`: same "everyone's an admin" fallback as
/// the rest of the app when there's no `auth { }` block at all (see
/// `report_extras`'s `can_delete`), since reload isn't part of the
/// user model — it should work in the common no-auth demo apps too.
fn require_reload_access(data: &AppData, auth: &AuthCtx) -> Result<(), AppError> {
    if !data.app.auth_enabled {
        return Ok(());
    }
    match &auth.0 {
        Some(user) if user.is_admin() => Ok(()),
        Some(_) => Err((StatusCode::FORBIDDEN, "reloading metadata requires the 'admin' role".to_string())),
        None => Err((StatusCode::UNAUTHORIZED, "sign in required".to_string())),
    }
}

type AppError = (StatusCode, String);

fn err_response(e: anyhow::Error) -> AppError {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn page_or_404<'a>(app: &'a RuntimeApp, name: &str) -> Result<&'a RuntimePage, AppError> {
    app.page(name).ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such page '{name}'")))
}

/// Filters a nav tree down to what the signed-in user (or public
/// visitor) is actually allowed to open — a leaf whose target page
/// has a `requires: <role>` the user doesn't hold is dropped the same
/// way `show`/etc. would reject a direct visit to it (same
/// `auth::authorize` check), and a group with no surviving children
/// disappears too rather than rendering an empty dropdown.
fn visible_nav(app: &RuntimeApp, nodes: &[NavNode], data: &AppData, auth: &AuthCtx) -> Vec<NavNode> {
    nodes
        .iter()
        .filter_map(|node| match &node.target_page {
            Some(target) => {
                let required_role = app.page(target).and_then(|p| p.required_role.as_deref());
                auth::authorize(data, required_role, auth).is_ok().then(|| node.clone())
            }
            None => {
                let children = visible_nav(app, &node.children, data, auth);
                (!children.is_empty()).then(|| NavNode {
                    label: node.label.clone(),
                    target_page: None,
                    children,
                })
            }
        })
        .collect()
}

fn component_at<'a>(page: &'a RuntimePage, idx: usize) -> Result<&'a RuntimeComponent, AppError> {
    page.components
        .get(idx)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("page '{}' has no component #{idx}", page.name)))
}

/// The (entity, field names, item types) a `Form` or `EditableTable`
/// writes through — the two writable component kinds share create/
/// update/delete handling, since both ultimately mean "a named subset
/// of one entity's fields, rendered via item types."
fn writable_fields<'a>(
    component: &'a RuntimeComponent,
    page_name: &str,
    idx: usize,
) -> Result<(&'a RuntimeEntity, &'a [String], &'a HashMap<String, FieldItem>), AppError> {
    match component {
        RuntimeComponent::Form { entity, fields, item_types, .. } => Ok((entity, fields, item_types)),
        RuntimeComponent::EditableTable { entity, columns, item_types, .. } => Ok((entity, columns, item_types)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            format!("page '{page_name}' component #{idx} does not accept writes"),
        )),
    }
}

/// Finds the first `Form` on the page bound to `entity_name`, so a
/// `Report` on the same entity can link its rows to it for edit/delete.
fn sibling_form_idx(page: &RuntimePage, entity_name: &str) -> Option<usize> {
    page.components
        .iter()
        .position(|c| matches!(c, RuntimeComponent::Form { entity, .. } if entity.name == entity_name))
}

/// The reverse of `sibling_form_idx`: the `Report` (if any) that a `Form`
/// is the edit/create popup for.
fn companion_report_idx(page: &RuntimePage, entity_name: &str) -> Option<usize> {
    page.components
        .iter()
        .position(|c| matches!(c, RuntimeComponent::Report { entity, .. } if entity.name == entity_name))
}

/// Where a redirect after a component action should scroll back to: a
/// popup Form's own container disappears once its `edit_{idx}`/`new_{idx}`
/// query param is gone, so its companion Report (if any) is what the user
/// actually wants to land back on; anything else anchors to itself.
fn redirect_anchor(page: &RuntimePage, idx: usize) -> usize {
    match page.components.get(idx) {
        Some(RuntimeComponent::Form { entity, .. }) => companion_report_idx(page, &entity.name).unwrap_or(idx),
        _ => idx,
    }
}

/// All entity columns, cast to text, so the generic layer only ever deals
/// with strings regardless of the underlying Postgres type.
fn select_columns(entity: &RuntimeEntity) -> String {
    let mut cols = vec!["id::text as id".to_string()];
    for f in &entity.fields {
        if f.name == "id" {
            continue;
        }
        cols.push(format!("{name}::text as {name}", name = f.name));
    }
    cols.join(", ")
}

fn row_from_sqlx(row: &sqlx::postgres::PgRow, entity: &RuntimeEntity) -> anyhow::Result<BTreeMap<String, Option<String>>> {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), row.try_get::<Option<String>, _>("id")?);
    for f in &entity.fields {
        if f.name == "id" {
            continue;
        }
        map.insert(f.name.clone(), row.try_get::<Option<String>, _>(f.name.as_str())?);
    }
    Ok(map)
}

async fn fetch_rows(pool: &PgPool, data_schema: &str, entity: &RuntimeEntity) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    // `order by t.id`, qualified: the select list aliases `id::text as
    // id`, and an unqualified ORDER BY id would bind to that *text*
    // output column, sorting "10" before "2".
    let sql = format!("select {} from {data_schema}.{} t order by t.id", select_columns(entity), entity.table_name);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(|r| row_from_sqlx(r, entity)).collect()
}

async fn fetch_row(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    id: &str,
) -> anyhow::Result<Option<BTreeMap<String, Option<String>>>> {
    let sql = format!(
        "select {} from {data_schema}.{} where id = $1::integer",
        select_columns(entity),
        entity.table_name
    );
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(|r| row_from_sqlx(r, entity)).transpose()
}

/// One page of a `Report`'s rows plus whether there's a previous/next
/// page — see `fetch_report_rows` for how that's known without a
/// `COUNT(*)`.
struct ReportPage {
    rows: Vec<BTreeMap<String, Option<String>>>,
    has_prev: bool,
    has_next: bool,
}

/// A report's live filter state, from its `r<idx>_q` (search across
/// visible columns) and `r<idx>_col`/`r<idx>_val` (single-column
/// filter) URL parameters.
#[derive(Debug, Clone, Default)]
struct ReportFilters {
    q: Option<String>,
    col: Option<(String, String)>,
}

impl ReportFilters {
    /// Extracts and validates filters for report `idx`. A `col` that
    /// isn't one of the report's own columns is rejected (the column
    /// name gets spliced into SQL, so it must come from the markup's
    /// validated set, never from the request).
    fn from_query(query: &HashMap<String, String>, idx: usize, columns: &[String]) -> Result<Self, AppError> {
        let get = |suffix: &str| {
            query
                .get(&format!("r{idx}_{suffix}"))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let col = match (get("col"), get("val")) {
            (Some(col), Some(val)) => {
                if !columns.contains(&col) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("cannot filter on '{col}': not a column of this report"),
                    ));
                }
                Some((col, val))
            }
            _ => None,
        };
        Ok(ReportFilters { q: get("q"), col })
    }

    /// Builds the SQL conditions for these filters. `prefix` qualifies
    /// column references (e.g. `"t."`); `first_param` is the number the
    /// first added `$N` placeholder should use. Column names come from
    /// the report's markup-validated column list only.
    fn to_sql(&self, prefix: &str, columns: &[String], first_param: usize) -> (Vec<String>, Vec<String>) {
        let mut conditions = Vec::new();
        let mut binds = Vec::new();
        if let Some(q) = &self.q {
            if !columns.is_empty() {
                let n = first_param + binds.len();
                let ors: Vec<String> = columns
                    .iter()
                    .map(|c| format!("({prefix}{c})::text ilike ${n}"))
                    .collect();
                conditions.push(format!("({})", ors.join(" or ")));
                binds.push(format!("%{q}%"));
            }
        }
        if let Some((col, val)) = &self.col {
            let n = first_param + binds.len();
            conditions.push(format!("({prefix}{col})::text ilike ${n}"));
            binds.push(format!("%{val}%"));
        }
        (conditions, binds)
    }
}

/// Reads one caller's page of a named collection — OFFSET-paginated
/// like a query-backed report (a collection has no assumed sort key
/// beyond its own insertion `seq`, and collections are small enough in
/// practice that `COUNT(*)`-free keyset pagination isn't worth the
/// complexity). The `app_id`/`caller_key`/`name` filter is baked in
/// here, not written by the app author — see `EntityDef::source_collection`.
async fn fetch_collection_page(
    pool: &PgPool,
    app_id: i32,
    caller_key: &str,
    collection_name: &str,
    entity: &RuntimeEntity,
    page_size: i64,
    page_num: i64,
) -> anyhow::Result<(Vec<BTreeMap<String, Option<String>>>, bool)> {
    let offset = (page_num - 1).max(0) * page_size;
    let json_rows: Vec<(i32, serde_json::Value)> = sqlx::query_as(
        "select seq, data from pgapp_meta.collections
          where app_id = $1 and caller_key = $2 and name = $3
          order by seq
          offset $4 limit $5",
    )
    .bind(app_id)
    .bind(caller_key)
    .bind(collection_name)
    .bind(offset)
    .bind(page_size + 1)
    .fetch_all(pool)
    .await?;

    let has_next = json_rows.len() as i64 > page_size;
    let rows = json_rows
        .into_iter()
        .take(page_size as usize)
        .map(|(seq, data)| {
            let mut map = BTreeMap::new();
            map.insert("id".to_string(), Some(seq.to_string()));
            for f in &entity.fields {
                if f.name == "id" {
                    continue;
                }
                let value = match data.get(&f.name) {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(other) => Some(other.to_string()),
                };
                map.insert(f.name.clone(), value);
            }
            map
        })
        .collect();
    Ok((rows, has_next))
}

/// Keyset ("seek") pagination for an entity-backed `Report`: `after`/
/// `before` cursor on `id`, fetching `page_size + 1` rows in the
/// query's own direction. Zero extra queries: the direction we fetched
/// tells us whether *it* has more (the extra row); the direction we
/// arrived *from* always has more, because reaching this page via a
/// cursor implies a page on the other side of it. `COUNT(*)`/`OFFSET`
/// never enter into it, so this stays cheap no matter how large the
/// table gets.
#[allow(clippy::too_many_arguments)]
async fn fetch_report_rows(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    filters: &ReportFilters,
    filter_columns: &[String],
    page_size: i64,
    after: Option<&str>,
    before: Option<&str>,
) -> anyhow::Result<ReportPage> {
    let cols = select_columns(entity);
    let lim = page_size + 1;

    // Filter conditions first ($1..), then the keyset cursor.
    let (mut conditions, binds) = filters.to_sql("t.", filter_columns, 1);
    let cursor_param = binds.len() + 1;

    // ORDER BY is qualified (`t.id`) for the same reason as in
    // `fetch_rows`: the select list re-exports `id` as text, and the
    // cursor comparison below is numeric — mixing the two orderings
    // would make pages skip/repeat rows.
    let (cursor_bind, order, reverse) = if let Some(after) = after {
        conditions.push(format!("t.id > ${cursor_param}::integer"));
        (Some(after), "asc", false)
    } else if let Some(before) = before {
        conditions.push(format!("t.id < ${cursor_param}::integer"));
        (Some(before), "desc", true)
    } else {
        (None, "asc", false)
    };

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("where {}", conditions.join(" and "))
    };
    let sql = format!(
        "select {cols} from {data_schema}.{} t {where_clause} order by t.id {order} limit {lim}",
        entity.table_name
    );

    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b.as_str());
    }
    if let Some(b) = cursor_bind {
        query = query.bind(b);
    }
    let db_rows = query.fetch_all(pool).await?;
    let mut rows: Vec<BTreeMap<String, Option<String>>> = db_rows.iter().map(|r| row_from_sqlx(r, entity)).collect::<anyhow::Result<_>>()?;

    let has_extra = rows.len() as i64 > page_size;
    if has_extra {
        rows.truncate(page_size as usize);
    }
    if reverse {
        rows.reverse();
    }

    let (has_prev, has_next) = if before.is_some() {
        (has_extra, true)
    } else if after.is_some() {
        (true, has_extra)
    } else {
        (false, has_extra)
    };

    Ok(ReportPage { rows, has_prev, has_next })
}

/// Builds (column names, value expressions, bind values) for a Form's
/// or EditableTable's writable fields. Empty, non-required values
/// become SQL `NULL` literals directly (an empty string can't be cast
/// to e.g. integer); everything else is bound as text and cast in SQL,
/// since the actual Postgres column type isn't known at compile time.
///
/// Each field's submitted value is read through its registered item
/// type's `read_value` (e.g. Checkbox reads presence-in-the-form rather
/// than a submitted value) — dispatched by whatever `item_types` says,
/// not a hardcoded kind list.
fn build_value_exprs(
    entity: &RuntimeEntity,
    field_names: &[String],
    item_types: &HashMap<String, FieldItem>,
    values: &HashMap<String, String>,
    registry: &item_types::Registry,
) -> anyhow::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut columns = Vec::new();
    let mut exprs = Vec::new();
    let mut binds = Vec::new();

    for name in field_names {
        let field = entity.field(name).ok_or_else(|| anyhow::anyhow!("unknown field '{name}'"))?;

        let raw = match item_types.get(name).and_then(|fi| registry.get(fi.kind.as_str())) {
            Some(component) => component.read_value(name, values),
            None => values.get(name).cloned().unwrap_or_default(),
        };
        let raw = raw.trim().to_string();

        if field.required && raw.is_empty() {
            anyhow::bail!("'{name}' is required");
        }

        columns.push(name.clone());
        if raw.is_empty() {
            exprs.push("NULL".to_string());
        } else {
            binds.push(raw);
            exprs.push(format!("${}::{}", binds.len(), field.data_type.sql_cast()));
        }
    }

    Ok((columns, exprs, binds))
}

/// `GET /` — a single-app server redirects straight into that one
/// app; a multi-app server shows a plain list of what's registered.
async fn landing(State(state): State<Arc<AppState>>) -> Response {
    let mut keys: Vec<String> = state.apps.read().unwrap().keys().cloned().collect();
    if keys.len() == 1 {
        let key = &keys[0];
        // `key` is "<workspace>/<app>" — encode each segment on its
        // own, never the whole key as one unit (that would turn its
        // internal "/" into "%2F" and break the redirect).
        let (workspace, app) = key.split_once('/').unwrap_or((key.as_str(), ""));
        return Redirect::to(&format!("/{}/{}", url_encode(workspace), url_encode(app))).into_response();
    }
    keys.sort();
    Html(render::workspace_landing(&keys)).into_response()
}

/// `/:workspace/:app` is just a redirect to that app's first page —
/// there's no separate "homepage" content to render, so nothing here
/// needs `auth_ctx`; `show` (the `/:workspace/:app/:page` handler)
/// re-checks login/role requirements on the page it lands on.
async fn index(State(state): State<Arc<AppState>>, Path((workspace, app)): Path<(String, String)>) -> Result<Redirect, AppError> {
    let key = format!("{workspace}/{app}");
    let entry = state.app_or_404(&key)?;
    let data = entry.data();
    let first = data
        .app
        .pages
        .first()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "this app has no pages".to_string()))?;
    Ok(Redirect::to(&format!(
        "/{}/{}/{}",
        url_encode(&workspace),
        url_encode(&app),
        url_encode(&first.name)
    )))
}

async fn theme_css(State(state): State<Arc<AppState>>, Path((workspace, app)): Path<(String, String)>) -> Response {
    let Ok(entry) = state.app_or_404(&format!("{workspace}/{app}")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&entry.data().theme.css_path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/css")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The pgapp runtime JS library — stored in `pgapp_meta`, not a static
/// file (see `AppData::runtime_js` / `main.rs`), so it's part of the
/// same in-database metadata as everything else.
async fn runtime_js(State(state): State<Arc<AppState>>, Path((workspace, app)): Path<(String, String)>) -> Response {
    let Ok(entry) = state.app_or_404(&format!("{workspace}/{app}")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        entry.data().runtime_js.clone(),
    )
        .into_response()
}

/// The active pluggable chart library's JS, if one is configured
/// (`PGAPP_CHART_LIB` other than the built-in "inline" backend, which
/// needs no JS at all — see `src/chart_lib.rs`).
async fn chart_lib_js(State(state): State<Arc<AppState>>, Path((workspace, app)): Path<(String, String)>) -> Response {
    let Ok(entry) = state.app_or_404(&format!("{workspace}/{app}")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match &entry.data().chart_lib.js_path {
        Some(path) => match tokio::fs::read(path).await {
            Ok(bytes) => ([(header::CONTENT_TYPE, "application/javascript")], bytes).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Static app-level asset override (`assets/app.css`/`assets/app.js`),
/// served from one shared directory regardless of which app asked —
/// there's no per-app asset directory (yet); only the URL is app-scoped,
/// to keep every path consistently rooted at `/:workspace/:app`.
async fn asset(Path((_workspace, _app, path)): Path<(String, String, String)>) -> Response {
    let safe = path.rsplit('/').next().unwrap_or("");
    if safe != "app.css" && safe != "app.js" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let full = format!("assets/{safe}");
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            let content_type = if safe.ends_with(".css") {
                "text/css"
            } else {
                "application/javascript"
            };
            ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Runs a component's `before_load` action, if it has one, immediately
/// before that component fetches its data — same registry and
/// `ActionContext` a button-triggered `action` component uses, just
/// invoked on every page load instead of a click. Failures are
/// non-fatal (an unreachable third-party API shouldn't take the whole
/// page down): the error is returned as text for the caller to surface
/// as an inline warning, and the component still renders with whatever
/// data already exists.
///
/// GET-only by construction: this function is only ever called from
/// `render_component`, which is only ever called from `show` (routed
/// as `get(show)` — see `build_router`). Keep it that way — do not
/// call `render_component`/`run_before_load` from `create`/`update`/
/// `delete`/`run_action`/`save_view` or any other POST handler; a
/// mutating request already has its own explicit, user-initiated
/// action and has no business also implicitly firing someone's
/// `before_load`.
async fn run_before_load(
    state: &AppState,
    data: &AppData,
    page: &RuntimePage,
    query: &HashMap<String, String>,
    caller_key: &str,
    pre: &PreAction,
) -> Option<String> {
    let module = match state.actions.get(pre.name.as_str()) {
        Some(m) => m,
        None => return Some(format!("before_load: action module '{}' is not registered (rebuild?)", pre.name)),
    };
    module
        .run(ActionContext {
            pool: &state.pool,
            app: &data.app,
            page,
            config: &pre.config,
            values: query,
            caller_key,
        })
        .await
        .err()
        .map(|e| format!("before_load ({}): {e}", pre.name))
}

/// Renders one component into its HTML body, fetching whatever data it
/// needs along the way.
#[allow(clippy::too_many_arguments)]
async fn render_component(
    app: &str,
    state: &AppState,
    data: &AppData,
    page_name: &str,
    page: &RuntimePage,
    idx: usize,
    component: &RuntimeComponent,
    query: &HashMap<String, String>,
    regions: &RegionRows,
    auth_ctx: &AuthCtx,
    caller_key: &str,
) -> anyhow::Result<String> {
    match component {
        RuntimeComponent::Text { text, html } => Ok(render::text_html(text, html)),
        RuntimeComponent::Link { label, target_page, html } => Ok(render::link_html(app, label, target_page, html)),
        RuntimeComponent::Region { label, query: qname, columns, html } => {
            Ok(render::region_html(label, qname, regions, columns, html))
        }

        // A dynamic action renders nothing in the body — show() gathers
        // them all into one JSON script for the runtime.js dispatcher.
        RuntimeComponent::DynamicAction { .. } => Ok(String::new()),

        RuntimeComponent::Action { label, name, html, .. } => {
            Ok(render::action_html(app, page_name, idx, label, name, html))
        }

        RuntimeComponent::Chart { title, query: qname, chart_type, x, y, html } => {
            let rq = page
                .resolve_query(&data.app, qname)
                .ok_or_else(|| anyhow::anyhow!("chart '{title}' references unknown query '{qname}'"))?;
            let ctx = bind_context(query, None);
            let rows = run_named_query_rows(&state.pool, &data.app.data_schema, rq, &ctx).await?;
            Ok(render::chart_html(title, chart_type, x, y, &rows, &data.chart_lib, html))
        }

        RuntimeComponent::Report { title, entity, columns, source_query, link_column, page_size, before_load, html } => {
            let before_load_warning = match before_load {
                Some(pre) => run_before_load(state, data, page, query, caller_key, pre).await,
                None => None,
            };

            let form_idx = sibling_form_idx(page, &entity.name);
            let p_after = format!("r{idx}_after");
            let p_before = format!("r{idx}_before");
            let p_page = format!("r{idx}_page");

            let filters = ReportFilters::from_query(query, idx, columns)
                .map_err(|(_, msg)| anyhow::anyhow!(msg))?;
            // Filter params re-serialized for pagination links, so
            // Prev/Next stay inside the filtered result set.
            let mut filter_qs = String::new();
            if let Some(q) = &filters.q {
                filter_qs.push_str(&format!("&r{idx}_q={}", url_encode(q)));
            }
            if let Some((col, val)) = &filters.col {
                filter_qs.push_str(&format!("&r{idx}_col={}&r{idx}_val={}", url_encode(col), url_encode(val)));
            }

            // A report is query-paginated when it declares `source:` or
            // its entity is query-backed (`entity ... from query`);
            // offset-paginated the same way when the entity is
            // collection-backed instead; keyset-paginated otherwise.
            let effective_query = source_query.as_deref().or(entity.source_query.as_deref());

            let (rows, prev_href, next_href) = if let Some(qname) = effective_query {
                let rq = page
                    .resolve_query(&data.app, qname)
                    .ok_or_else(|| anyhow::anyhow!("report '{title}' sources from unknown query '{qname}'"))?;
                let ctx = bind_context(query, None);
                let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                let (conditions, binds) = filters.to_sql("t.", columns, rq.bind_names.len() + 1);
                let where_clause = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("where {}", conditions.join(" and "))
                };
                let (json_rows, has_next) =
                    run_named_query_page(&state.pool, &data.app.data_schema, rq, &ctx, &where_clause, &binds, *page_size, page_num).await?;
                let rows: Vec<_> = json_rows.into_iter().map(query_engine::json_row_to_map).collect();
                let prev_href = (page_num > 1).then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num - 1));
                let next_href = has_next.then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num + 1));
                (rows, prev_href, next_href)
            } else if let Some(coll_name) = entity.source_collection.as_deref() {
                let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                let (rows, has_next) =
                    fetch_collection_page(&state.pool, data.app.id, caller_key, coll_name, entity, *page_size, page_num).await?;
                let prev_href = (page_num > 1).then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num - 1));
                let next_href = has_next.then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num + 1));
                (rows, prev_href, next_href)
            } else {
                let after = query.get(&p_after).map(|s| s.as_str());
                let before = query.get(&p_before).map(|s| s.as_str());
                let rp = fetch_report_rows(&state.pool, &data.app.data_schema, entity, &filters, columns, *page_size, after, before).await?;
                let prev_href = rp.has_prev.then(|| {
                    let id = rp.rows.first().and_then(|r| r.get("id")).and_then(|v| v.clone()).unwrap_or_default();
                    format!("/{app}/{page_name}?{p_before}={}{filter_qs}", url_encode(&id))
                });
                let next_href = rp.has_next.then(|| {
                    let id = rp.rows.last().and_then(|r| r.get("id")).and_then(|v| v.clone()).unwrap_or_default();
                    format!("/{app}/{page_name}?{p_after}={}{filter_qs}", url_encode(&id))
                });
                (rp.rows, prev_href, next_href)
            };

            let mut extras = report_extras(app, state, data, page_name, idx, &filters, auth_ctx).await?;
            extras.warning = before_load_warning;

            Ok(render::report_html(
                app,
                page_name,
                idx,
                title,
                columns,
                &rows,
                link_column.as_ref(),
                prev_href.as_deref(),
                next_href.as_deref(),
                form_idx,
                &data.icons,
                &extras,
                html,
            ))
        }

        RuntimeComponent::Form { title, entity, fields, item_types, field_html, html } => {
            // A Form that's a Report's edit/create companion renders as a
            // floating popup instead of a block sitting inline below the
            // table: closed (nothing rendered) unless its edit_{idx}/
            // new_{idx} query flag is present. A standalone Form (no
            // sibling Report) keeps the old always-visible behavior.
            let report_idx = companion_report_idx(page, &entity.name);
            let floating = report_idx.is_some();
            let close_href = match report_idx {
                Some(ridx) => format!("/{app}/{page_name}#pgapp-c{ridx}"),
                None => format!("/{app}/{page_name}"),
            };
            let edit_param = format!("edit_{idx}");
            let new_param = format!("new_{idx}");
            if floating && !query.contains_key(&edit_param) && !query.contains_key(&new_param) {
                return Ok(String::new());
            }
            match query.get(&edit_param) {
                Some(id) => {
                    let row = fetch_row(&state.pool, &data.app.data_schema, entity, id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("row '{id}' not found"))?;
                    let ctx = bind_context(query, Some(&row));
                    let choices = resolve_field_choices(&state.pool, &data.app, page, item_types, &ctx).await?;
                    Ok(render::form_html(
                        app, page_name, idx, title, fields, entity, &row, Some(id), &choices, item_types, &state.item_types,
                        floating, &close_href, field_html, html,
                    ))
                }
                None => {
                    let ctx = bind_context(query, None);
                    let choices = resolve_field_choices(&state.pool, &data.app, page, item_types, &ctx).await?;
                    let empty = BTreeMap::new();
                    Ok(render::form_html(
                        app, page_name, idx, title, fields, entity, &empty, None, &choices, item_types, &state.item_types,
                        floating, &close_href, field_html, html,
                    ))
                }
            }
        }

        RuntimeComponent::EditableTable { title, entity, columns, item_types, field_html, html } => {
            let ctx = bind_context(query, None);
            let choices = resolve_field_choices(&state.pool, &data.app, page, item_types, &ctx).await?;
            let rows = fetch_rows(&state.pool, &data.app.data_schema, entity).await?;
            Ok(render::editable_table_html(
                app,
                page_name,
                idx,
                title,
                columns,
                entity,
                &rows,
                &choices,
                item_types,
                &state.item_types,
                &data.icons,
                field_html,
                html,
            ))
        }
    }
}

/// Fetches the saved views visible to the current user for one report
/// (their own plus public ones) and packages the toolbar state for
/// rendering.
async fn report_extras(
    app: &str,
    state: &AppState,
    data: &AppData,
    page_name: &str,
    idx: usize,
    filters: &ReportFilters,
    auth_ctx: &AuthCtx,
) -> anyhow::Result<render::ReportExtras> {
    let user_id = auth_ctx.0.as_ref().map(|u| u.id);
    let is_admin = auth_ctx.0.as_ref().is_some_and(|u| u.is_admin());

    let rows: Vec<(i32, String, serde_json::Value, Option<i32>)> = sqlx::query_as(
        "select id, name, params, owner_user_id from pgapp_meta.report_views
          where app_id = $1 and page_name = $2 and component_idx = $3
            and (is_public or owner_user_id is not distinct from $4)
          order by name",
    )
    .bind(data.app.id)
    .bind(page_name)
    .bind(idx as i32)
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;

    let views = rows
        .into_iter()
        .map(|(id, name, params, owner)| {
            let mut href = format!("/{app}/{page_name}?");
            for (key, param) in [("q", "q"), ("col", "col"), ("val", "val")] {
                if let Some(v) = params.get(key).and_then(|v| v.as_str()) {
                    href.push_str(&format!("r{idx}_{param}={}&", url_encode(v)));
                }
            }
            render::ReportViewLink {
                id,
                name,
                href: href.trim_end_matches(['&', '?']).to_string(),
                can_delete: !data.app.auth_enabled || is_admin || owner == user_id,
            }
        })
        .collect();

    Ok(render::ReportExtras {
        q: filters.q.clone().unwrap_or_default(),
        fcol: filters.col.as_ref().map(|(c, _)| c.clone()).unwrap_or_default(),
        fval: filters.col.as_ref().map(|(_, v)| v.clone()).unwrap_or_default(),
        views,
        warning: None,
    })
}

async fn show(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app, page_name)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let ctx = bind_context(&query, None);
    let regions = resolve_regions(&state.pool, &data.app, Some(page), &ctx)
        .await
        .map_err(err_response)?;

    let mut body = String::new();
    for (idx, component) in page.components.iter().enumerate() {
        let html = render_component(&app, &state, &data, &page_name, page, idx, component, &query, &regions, &auth_ctx, &caller.0)
            .await
            .map_err(err_response)?;
        // A stable per-component anchor: mutating actions (save/delete,
        // running an action, applying a filter) redirect back to
        // `#pgapp-c{idx}` instead of the bare page, so the browser lands
        // near the component the user was just looking at rather than
        // resetting scroll to the top.
        body.push_str(&format!(r#"<div id="pgapp-c{idx}">{html}</div>"#));
    }

    // All the page's dynamic actions, as one JSON blob the runtime.js
    // dispatcher binds on DOMContentLoaded.
    let dyn_actions: Vec<&serde_json::Value> = page
        .components
        .iter()
        .filter_map(|c| match c {
            RuntimeComponent::DynamicAction { config } => Some(config),
            _ => None,
        })
        .collect();
    if !dyn_actions.is_empty() {
        body.push_str(&render::dynamic_actions_script(&dyn_actions));
    }

    let nav = visible_nav(&data.app, &data.app.nav, &data, &auth_ctx);
    Ok(Html(render::page_layout(
        &app,
        &data.app.name,
        &page.name,
        &body,
        query.get("error").map(|s| s.as_str()),
        query.get("notice").map(|s| s.as_str()),
        Chrome { nav: &nav, ..data.app.chrome(&regions) },
        &data.icons,
        &data.chart_lib,
        auth_ctx.display(),
    )))
}

/// One region's rows re-rendered as an HTML fragment — what a dynamic
/// action's `refresh` op fetches. The page's current item values arrive
/// as query parameters and become the query's bind context, so a
/// region can follow a form field the user just changed.
async fn region_fragment(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, query_name)): Path<(String, String, String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let entry = state.app_or_404(&format!("{workspace}/{app}"))?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let rq = page.resolve_query(&data.app, &query_name).ok_or_else(|| {
        (StatusCode::NOT_FOUND, format!("no query '{query_name}' visible from page '{page_name}'"))
    })?;

    let ctx = bind_context(&params, None);
    let rows = run_named_query_rows(&state.pool, &data.app.data_schema, rq, &ctx).await.map_err(err_response)?;

    let region = page.components.iter().find_map(|c| match c {
        RuntimeComponent::Region { label, query, columns, html } if *query == query_name => {
            Some((label.clone(), columns.clone(), html.clone()))
        }
        _ => None,
    });
    let (label, columns, html) = region.unwrap_or_else(|| (query_name.clone(), Vec::new(), HtmlAttrs::default()));

    let mut regions = RegionRows::new();
    regions.insert(query_name.clone(), rows);
    Ok(Html(render::region_html(&label, &query_name, &regions, &columns, &html)))
}

/// Runs a page's server-side action module (`action ... calls <name>`)
/// and redirects back with its outcome as a notice or error banner.
async fn run_action(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app, page_name, idx)): Path<(String, String, String, usize)>,
    Query(query): Query<HashMap<String, String>>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let RuntimeComponent::Action { name, config, .. } = component else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("page '{page_name}' component #{idx} is not an action"),
        ));
    };
    let module = state.actions.get(name.as_str()).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("action module '{name}' is in metadata but not registered (rebuild?)"),
        )
    })?;

    // URL parameters plus the POSTed form fields (form wins) — the same
    // merged shape named-query bind contexts use.
    let mut merged = query.clone();
    merged.extend(values);

    let outcome = module
        .run(ActionContext {
            pool: &state.pool,
            app: &data.app,
            page,
            config,
            values: &merged,
            caller_key: &caller.0,
        })
        .await;

    let anchor = redirect_anchor(page, idx);
    match outcome {
        Ok(msg) => Ok(Redirect::to(&format!("/{app}/{page_name}?notice={}#pgapp-c{anchor}", url_encode(&msg))).into_response()),
        Err(e) => Ok(Redirect::to(&format!("/{app}/{page_name}?error={}#pgapp-c{anchor}", url_encode(&e.to_string()))).into_response()),
    }
}

/// Saves the current filter state of one report as a named view —
/// private by default, public when the checkbox says so.
async fn save_view(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, idx)): Path<(String, String, String, usize)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    if !matches!(component, RuntimeComponent::Report { .. }) {
        return Err((StatusCode::BAD_REQUEST, format!("component #{idx} is not a report")));
    }

    let name = values.get("name").map(|s| s.trim()).unwrap_or_default();
    if name.is_empty() {
        return Ok(Redirect::to(&format!(
            "/{app}/{page_name}?error={}#pgapp-c{idx}",
            url_encode("A saved view needs a name.")
        ))
        .into_response());
    }

    let get = |k: &str| values.get(&format!("r{idx}_{k}")).map(|s| s.trim()).filter(|s| !s.is_empty());
    let mut params = serde_json::Map::new();
    if let Some(q) = get("q") {
        params.insert("q".into(), q.into());
    }
    if let (Some(col), Some(val)) = (get("col"), get("val")) {
        params.insert("col".into(), col.into());
        params.insert("val".into(), val.into());
    }

    sqlx::query(
        "insert into pgapp_meta.report_views (app_id, page_name, component_idx, name, owner_user_id, is_public, params)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(data.app.id)
    .bind(&page_name)
    .bind(idx as i32)
    .bind(name)
    .bind(auth_ctx.0.as_ref().map(|u| u.id))
    .bind(values.contains_key("is_public"))
    .bind(serde_json::Value::Object(params))
    .execute(&state.pool)
    .await
    .map_err(|e| err_response(e.into()))?;

    Ok(Redirect::to(&format!("/{app}/{page_name}?notice={}#pgapp-c{idx}", url_encode(&format!("View '{name}' saved.")))).into_response())
}

async fn delete_view(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, idx, view_id)): Path<(String, String, String, usize, i32)>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;

    let owner: Option<Option<i32>> =
        sqlx::query_scalar("select owner_user_id from pgapp_meta.report_views where id = $1 and app_id = $2")
            .bind(view_id)
            .bind(data.app.id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| err_response(e.into()))?;
    let Some(owner) = owner else {
        return Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{idx}")).into_response());
    };

    let allowed = !data.app.auth_enabled
        || auth_ctx.0.as_ref().is_some_and(|u| u.is_admin() || Some(u.id) == owner);
    if !allowed {
        return Err((StatusCode::FORBIDDEN, "only the view's owner or an admin can delete it".to_string()));
    }

    sqlx::query("delete from pgapp_meta.report_views where id = $1 and app_id = $2")
        .bind(view_id)
        .bind(data.app.id)
        .execute(&state.pool)
        .await
        .map_err(|e| err_response(e.into()))?;
    Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{idx}")).into_response())
}

async fn create(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, idx)): Path<(String, String, String, usize)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let (entity, fields, item_types) = writable_fields(component, &page_name, idx)?;

    match build_value_exprs(entity, fields, item_types, &values, &state.item_types) {
        Ok((columns, exprs, binds)) => {
            let sql = format!(
                "insert into {}.{} ({}) values ({})",
                data.app.data_schema,
                entity.table_name,
                columns.join(", "),
                exprs.join(", ")
            );
            let mut sql_query = sqlx::query(&sql);
            for b in &binds {
                sql_query = sql_query.bind(b);
            }
            sql_query
                .execute(&state.pool)
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to create row: {e}")))?;
            Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response())
        }
        // Reopen the popup in create mode (via new_{idx}) so the error is
        // visible instead of silently closing.
        Err(e) => Ok(Redirect::to(&format!(
            "/{app}/{page_name}?error={}&new_{idx}=1#pgapp-c{idx}",
            url_encode(&e.to_string())
        ))
        .into_response()),
    }
}

async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, idx, id)): Path<(String, String, String, usize, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let (entity, fields, item_types) = writable_fields(component, &page_name, idx)?;

    match build_value_exprs(entity, fields, item_types, &values, &state.item_types) {
        Ok((columns, exprs, mut binds)) => {
            let set_clause = columns
                .iter()
                .zip(exprs.iter())
                .map(|(c, e)| format!("{c} = {e}"))
                .collect::<Vec<_>>()
                .join(", ");
            binds.push(id.clone());
            let where_placeholder = binds.len();
            let sql = format!(
                "update {}.{} set {set_clause} where id = ${where_placeholder}::integer",
                data.app.data_schema,
                entity.table_name
            );
            let mut sql_query = sqlx::query(&sql);
            for b in &binds {
                sql_query = sql_query.bind(b);
            }
            sql_query
                .execute(&state.pool)
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to update row: {e}")))?;
            Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response())
        }
        Err(e) => {
            // A Form component re-enters edit mode on error (so the
            // user doesn't lose their place); an EditableTable has no
            // separate edit mode to return to. The anchor stays on the
            // form's own idx (not its companion report) since that's
            // where it's reopening.
            let extra = match component {
                RuntimeComponent::Form { .. } => format!("&edit_{idx}={}", url_encode(&id)),
                _ => String::new(),
            };
            Ok(Redirect::to(&format!("/{app}/{page_name}?error={}{extra}#pgapp-c{idx}", url_encode(&e.to_string()))).into_response())
        }
    }
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page_name, idx, id)): Path<(String, String, String, usize, String)>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let (entity, _, _) = writable_fields(component, &page_name, idx)?;

    let sql = format!("delete from {}.{} where id = $1::integer", data.app.data_schema, entity.table_name);
    sqlx::query(&sql)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to delete row: {e}")))?;
    Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response())
}

/// Minimal JSON API, keyed by entity rather than page — a stand-in for
/// the REST routing PostgREST would otherwise provide. Looks for the
/// entity on any Report/Form/EditableTable component across every page.
/// Query-backed entities serve their query's rows.
async fn api_list(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app, entity_name)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = state.app_or_404(&format!("{workspace}/{app}"))?;
    let data = entry.data();
    let entity = data
        .app
        .pages
        .iter()
        .flat_map(|p| p.components.iter())
        .find_map(|c| match c {
            RuntimeComponent::Report { entity, .. } if entity.name == entity_name => Some(entity),
            RuntimeComponent::Form { entity, .. } if entity.name == entity_name => Some(entity),
            RuntimeComponent::EditableTable { entity, .. } if entity.name == entity_name => Some(entity),
            _ => None,
        })
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such entity '{entity_name}'")))?;

    let rows = if let Some(qname) = &entity.source_query {
        let rq = data.app.queries.get(qname).ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("entity '{entity_name}' is backed by unknown query '{qname}'"),
            )
        })?;
        let ctx = HashMap::new();
        run_named_query_rows(&state.pool, &data.app.data_schema, rq, &ctx).await.map_err(err_response)?
    } else if let Some(coll_name) = &entity.source_collection {
        // No pagination here (unlike the Report component) — the /api
        // route always returns every row, same as a table-backed
        // entity's fetch_rows. i64::MAX/2 avoids overflowing the
        // `page_size + 1` inside fetch_collection_page while still
        // being an effectively unbounded limit.
        let (rows, _) = fetch_collection_page(&state.pool, data.app.id, &caller.0, coll_name, entity, i64::MAX / 2, 1)
            .await
            .map_err(err_response)?;
        rows
    } else {
        fetch_rows(&state.pool, &data.app.data_schema, entity).await.map_err(err_response)?
    };
    Ok(axum::Json(json!(rows)).into_response())
}

/// GET /:workspace/:app/admin/reload — shows the current markup (editable inline
/// when it's a single file) plus a button to re-sync it into
/// `pgapp_meta` without restarting the process. See `AppEntry::reload`.
async fn admin_reload_page(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    require_reload_access(&data, &auth_ctx)?;

    let markup_text = if std::path::Path::new(&entry.markup_path).is_dir() {
        None
    } else {
        tokio::fs::read_to_string(&entry.markup_path).await.ok()
    };

    let ctx = HashMap::new();
    let regions = resolve_regions(&state.pool, &data.app, None, &ctx).await.map_err(err_response)?;

    let nav = visible_nav(&data.app, &data.app.nav, &data, &auth_ctx);
    Ok(Html(render::reload_page(
        &app,
        &data.app.name,
        &entry.markup_path,
        markup_text.as_deref(),
        query.get("error").map(|s| s.as_str()),
        query.get("notice").map(|s| s.as_str()),
        Chrome { nav: &nav, ..data.app.chrome(&regions) },
        &data.icons,
        &data.chart_lib,
        auth_ctx.display(),
    )))
}

/// POST /:workspace/:app/admin/reload — optionally writes edited markup back to
/// disk (`do=save`, single-file apps only), then always re-runs the
/// full parse/sync/load pipeline and atomically swaps in the result —
/// only for this one app; every other app in the process is untouched.
async fn admin_reload(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    {
        let data = entry.data();
        require_reload_access(&data, &auth_ctx)?;
    }

    if values.get("do").map(|s| s.as_str()) == Some("save") {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            return Ok(Redirect::to(&format!(
                "/{app}/admin/reload?error={}",
                url_encode("This app's markup is a directory of files — edit them on disk, then use \"Reload from disk\".")
            ))
            .into_response());
        }
        let markup = values.get("markup").cloned().unwrap_or_default();
        if let Err(e) = tokio::fs::write(&entry.markup_path, markup).await {
            return Ok(Redirect::to(&format!(
                "/{app}/admin/reload?error={}",
                url_encode(&format!("failed to write markup file: {e}"))
            ))
            .into_response());
        }
    }

    match entry.reload(&state.pool, &state.item_types, &state.actions).await {
        Ok(()) => Ok(Redirect::to(&format!(
            "/{app}/admin/reload?notice={}",
            url_encode("Metadata reloaded — no restart needed.")
        ))
        .into_response()),
        Err(e) => Ok(Redirect::to(&format!("/{app}/admin/reload?error={}", url_encode(&e.to_string()))).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/reorder — the App Builder's
/// drag-and-drop save: `order` (form-encoded) is the page's
/// `pgapp_meta.components.id`s in their new order. Updates the
/// database *and* the app's own `.pgapp` file (see `page_reorder.rs`)
/// so the two never drift apart, then hot-reloads this one app in
/// place — same "no restart" story as `admin/reload`. JSON in, JSON
/// out (`{"ok":true}` / `{"ok":false,"error":"..."}"`), since this is
/// called from `fetch()`, not submitted as a browser form. Single-file
/// apps only for now — a directory app's page lives across more than
/// one file, and splicing across files isn't implemented yet.
async fn admin_reorder_page(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page)): Path<(String, String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    if workspace == instance::APP_BUILDER_WORKSPACE_SLUG && app == instance::APP_BUILDER_APP_SLUG {
        return Err((StatusCode::FORBIDDEN, "the App Builder can't edit itself".to_string()));
    }

    let app_key = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app_key)?;
    let data = entry.data();
    require_reload_access(&data, &auth_ctx)?;

    let reorder_result = reorder_page_impl(&state.pool, &data.app, &entry.markup_path, &page, &values).await;
    match reorder_result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("reordered, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

async fn reorder_page_impl(
    pool: &PgPool,
    app: &RuntimeApp,
    markup_path: &str,
    page_name: &str,
    values: &HashMap<String, String>,
) -> anyhow::Result<()> {
    if std::path::Path::new(markup_path).is_dir() {
        anyhow::bail!("this app's markup is a directory of files — drag-and-drop reordering only supports single-file apps right now");
    }

    let page_id: i32 = sqlx::query_scalar("select id from pgapp_meta.pages where app_id = $1 and name = $2")
        .bind(app.id)
        .bind(page_name)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no page named '{page_name}' in this app's metadata"))?;

    let old_order: Vec<i32> =
        sqlx::query_scalar("select id from pgapp_meta.components where page_id = $1 order by ordinal")
            .bind(page_id)
            .fetch_all(pool)
            .await?;

    let posted: Vec<i32> = values
        .get("order")
        .map(|s| s.as_str())
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>().context("'order' must be a comma-separated list of component ids"))
        .collect::<anyhow::Result<Vec<i32>>>()?;

    if posted.len() != old_order.len() {
        anyhow::bail!("'order' names {} components but this page has {}", posted.len(), old_order.len());
    }
    let new_order_indices: Vec<usize> = posted
        .iter()
        .map(|id| {
            old_order
                .iter()
                .position(|old_id| old_id == id)
                .ok_or_else(|| anyhow::anyhow!("component id {id} isn't one of this page's current components"))
        })
        .collect::<anyhow::Result<Vec<usize>>>()?;

    let markup_text = tokio::fs::read_to_string(markup_path)
        .await
        .with_context(|| format!("failed to read '{markup_path}'"))?;
    let reordered = crate::page_reorder::reorder_page(&markup_text, page_name, &new_order_indices)?;
    tokio::fs::write(markup_path, reordered)
        .await
        .with_context(|| format!("failed to write '{markup_path}'"))?;

    for (ordinal, id) in posted.iter().enumerate() {
        sqlx::query("update pgapp_meta.components set ordinal = $1 where id = $2")
            .bind(ordinal as i32)
            .bind(id)
            .execute(pool)
            .await
            .context("failed to update component ordinal")?;
    }
    Ok(())
}

/// Shared by the three routes below: refuses to touch the App Builder
/// itself (same belt-and-suspenders reasoning as `admin_reorder_page`),
/// looks up the target app, and checks reload access. Returns the
/// entry so the caller can read its markup path / call `.reload()`.
fn admin_edit_guard(
    state: &AppState,
    auth_ctx: &AuthCtx,
    workspace: &str,
    app: &str,
) -> Result<Arc<AppEntry>, AppError> {
    if workspace == instance::APP_BUILDER_WORKSPACE_SLUG && app == instance::APP_BUILDER_APP_SLUG {
        return Err((StatusCode::FORBIDDEN, "the App Builder can't edit itself".to_string()));
    }
    let entry = state.app_or_404(&format!("{workspace}/{app}"))?;
    require_reload_access(&entry.data(), auth_ctx)?;
    Ok(entry)
}

/// Parses `text` as a whole app and discards the result — used to
/// reject a hand-edited component/page block *before* it's written to
/// disk, so a typo can never leave the file in a broken state (unlike
/// `entry.reload()`, which only notices after the write already
/// happened). Doesn't catch every possible mistake (an unknown
/// entity/query reference still only surfaces at `meta::sync_app`,
/// same as any other markup edit) — just malformed syntax.
fn validate_markup(text: &str) -> anyhow::Result<()> {
    markup::parse_app(text).map(|_| ()).context("that markup doesn't parse")
}

/// GET /:workspace/:app/admin/pages-list — every page name currently in
/// the target app's markup, for the App Builder's "Target page"
/// dropdown on anything with a `-> page <Name>` target (report `link:`,
/// `link` components) — lets that be picked from a real list instead of
/// hand-typed, without introducing a whole structured property sheet.
async fn admin_pages_list(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<Vec<String>> = async {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let (starts, _) = markup::app_page_start_lines(&markup_text)?;
        Ok(starts.into_iter().map(|(name, _)| name).collect())
    }
    .await;

    match result {
        Ok(pages) => Ok(axum::Json(json!({"ok": true, "pages": pages})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/pages/:page/components/:idx/source —
/// returns component `idx`'s exact current markup text, for the App
/// Builder's "Edit" panel to prefill its textarea with (see
/// `page_reorder::component_source`).
async fn admin_component_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page, idx)): Path<(String, String, String, usize)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<String> = async {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        page_reorder::component_source(&markup_text, &page, idx)
    }
    .await;

    match result {
        Ok(source) => Ok(axum::Json(json!({"ok": true, "source": source})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/components/add — the App
/// Builder's "Add Component" panel: `source` is the caller's own raw
/// markup text for exactly one new component, of *any* kind
/// (text/report/form/editable_table/chart/region/action/link) with
/// *any* attribute the grammar supports — the panel just seeds the
/// textarea with a per-kind starter template client-side (see
/// runtime.js's `bindAddComponentForm`), it doesn't constrain what gets
/// submitted. Validated with `validate_markup` before writing, so a
/// malformed block is rejected instead of corrupting the file. Always
/// appends at the end of the page — drag it into place afterward with
/// the existing reorder feature. JSON in/out, same as
/// `admin_reorder_page`.
async fn admin_add_component_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page)): Path<(String, String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    // Trimmed only for the emptiness check below — the value actually
    // spliced in keeps whatever leading indentation the user's textarea
    // had, since a single-line component (e.g. `link`/`text`) has that
    // indentation as literally the first characters of the whole
    // string, and a blanket `.trim()` here would strip it every time
    // (a real bug this once was: the App Builder's own raw indent
    // vanishing on every one-line component add/edit).
    let new_component = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_component.trim().is_empty() {
            anyhow::bail!("a component needs some markup text");
        }
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::append_component(&markup_text, &page, &new_component)?;
        validate_markup(&updated)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("added, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/components/:idx/edit — the
/// App Builder's full-property "Edit": `source` replaces component
/// `idx` outright (see `page_reorder::replace_component`), so *any*
/// attribute of *any* kind can change, not just label/columns —
/// APEX-Page-Designer-style full edit, minus the property-sheet UI
/// (this is a raw block editor instead, prefilled via
/// `admin_component_source`). Validated with `validate_markup` before
/// writing.
async fn admin_edit_component_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page, idx)): Path<(String, String, String, usize)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    // See admin_add_component_source's comment on why this isn't trimmed.
    let new_component = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_component.trim().is_empty() {
            anyhow::bail!("a component needs some markup text");
        }
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::replace_component(&markup_text, &page, idx, &new_component)?;
        validate_markup(&updated)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("edited, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/rename — renames a whole
/// page (see `page_reorder::rename_page`, which also fixes up any
/// `-> page <old_name>` reference elsewhere in the file). Validated
/// with `validate_markup` before writing.
async fn admin_rename_page(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page)): Path<(String, String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_name = values.get("new_name").map(|s| s.trim().to_string()).unwrap_or_default();

    let result: anyhow::Result<()> = async {
        if new_name.is_empty() {
            anyhow::bail!("a page needs a name");
        }
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::rename_page(&markup_text, &page, &new_name)?;
        validate_markup(&updated)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true, "new_name": new_name})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("renamed, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/components/:idx/delete —
/// removes component `idx` outright.
async fn admin_delete_component(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page, idx)): Path<(String, String, String, usize)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;

    let result: anyhow::Result<()> = async {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::delete_component(&markup_text, &page, idx)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("deleted, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/add — the App Builder's "Add
/// Page": a new, empty page, appended to the target app's file (see
/// `page_reorder::add_page`), then hot-reloaded in place. Add
/// components to it afterward the normal way.
async fn admin_add_page(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let name = values.get("name").map(|s| s.trim()).unwrap_or("");

    let result: anyhow::Result<()> = async {
        if name.is_empty() {
            anyhow::bail!("a page needs a name");
        }
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::add_page(&markup_text, name)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("added, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/pages/:page/delete — removes an entire
/// page (and every component on it) from the target app's file.
async fn admin_delete_page(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;

    let result: anyhow::Result<()> = async {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let markup_text = tokio::fs::read_to_string(&entry.markup_path)
            .await
            .with_context(|| format!("failed to read '{}'", entry.markup_path))?;
        let updated = page_reorder::delete_page(&markup_text, &page)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("deleted, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /pgapp/builder/admin/apps/create-pending — the App Builder's
/// "New App" processing step. Fixed path, not `/:workspace/:app/...`:
/// unlike every other admin route here, this never acts on some other
/// app — it's intrinsically singular to the one App Builder an
/// instance has (see `instance::APP_BUILDER_WORKSPACE_SLUG`'s doc), so
/// a generic `:workspace/:app` shape would just invite a URL that
/// claims to act on a *different* app but doesn't. Triggered by
/// `runtime.js`'s `bindNewAppProcessing` on every load of the NewApp
/// page — a harmless no-op when nothing is pending. Unlike every
/// other admin route above, a real success here needs `AppState`
/// access to hot-register the brand-new app (see
/// `actions::create_app`'s doc for why that logic isn't a
/// `ServerAction`), so this is the one route that also calls
/// `AppState::register_app`.
async fn admin_create_pending_app(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    match actions::create_app::process_oldest_pending(&state.pool).await {
        Ok(None) => Ok(axum::Json(json!({"ok": true, "processed": false})).into_response()),
        Ok(Some((id, Ok(created)))) => {
            let register_result = AppEntry::load(
                &state.pool,
                &created.markup_path,
                &created.data_schema,
                created.control_app_id,
                created.workspace_id,
                &state.item_types,
                &state.actions,
            )
            .await;
            let (status, message) = match register_result {
                Ok(entry) => {
                    state.register_app(created.key.clone(), entry);
                    ("done", format!("Created and now live at /{}", created.key))
                }
                // The app IS registered in pgapp_control at this point
                // (create_one's job, already done) — just not yet
                // loaded into this process. The next `pgapp run` picks
                // it up the normal way; this is the one path where that
                // fallback still applies.
                Err(e) => ("done", format!("Created and registered, but couldn't load it live ({e:#}) — restart `pgapp run` to serve it at /{}", created.key)),
            };
            sqlx::query(&format!("update {} set status = $1, result = $2 where id = $3", actions::create_app::REQUESTS_TABLE))
                .bind(status)
                .bind(&message)
                .bind(id)
                .execute(&state.pool)
                .await
                .ok();
            Ok(axum::Json(json!({"ok": true, "processed": true})).into_response())
        }
        Ok(Some((id, Err(e)))) => {
            sqlx::query(&format!("update {} set status = 'error', result = $1 where id = $2", actions::create_app::REQUESTS_TABLE))
                .bind(format!("{e:#}"))
                .bind(id)
                .execute(&state.pool)
                .await
                .ok();
            Ok(axum::Json(json!({"ok": true, "processed": true})).into_response())
        }
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}
