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
use axum::extract::{DefaultBodyLimit, Form, Multipart, Path, Query, State};
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
use crate::app_editor;
use crate::chart_lib::ChartLib;
use crate::control;
use crate::html::url_encode;
use crate::icons::Icons;
use crate::instance;
use crate::item_types;
use crate::meta::{self, Chrome, NavNode, RegionRows, RuntimeApp, RuntimeComponent, RuntimeEntity, RuntimePage, RuntimeQuery};
use crate::model::{AggregateFn, ComputedColumn, Facet, FieldItem, HtmlAttrs, PreAction, CHART_TYPES};
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

    /// Removes one app from the live registry outright — the hard-
    /// delete counterpart to `register_app`. A soft delete (disable)
    /// deliberately does *not* call this: it only flips
    /// `pgapp_control.apps.enabled`, the same "takes effect on the next
    /// `pgapp run`, not this already-running process" behavior the CLI
    /// has always had. A hard delete is different — its tables are
    /// really gone, so leaving it servable would mean every request
    /// against it starts failing on missing tables instead of cleanly
    /// 404ing.
    pub fn unregister_app(&self, key: &str) {
        self.apps.write().unwrap().remove(key);
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

/// Per-file cap for `file_browse` uploads (see `upload_file`) —
/// generous for the typical case (a document/image attachment) while
/// still bounding how much of one request axum buffers in memory.
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(landing))
        .route("/:workspace/:app", get(index))
        .route("/:workspace/:app/theme.css", get(theme_css))
        .route("/:workspace/:app/runtime.js", get(runtime_js))
        .route("/:workspace/:app/chart-lib.js", get(chart_lib_js))
        .route("/:workspace/:app/assets/*path", get(asset))
        .route("/:workspace/:app/api/:entity", get(api_list))
        .route("/:workspace/:app/uploads", post(upload_file))
        .route("/:workspace/:app/uploads/:id", get(download_file))
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
        .route(
            "/:workspace/:app/admin/pages/:page/components/:idx/structured",
            get(admin_component_structured),
        )
        .route("/:workspace/:app/admin/pages/:page/app-meta", get(admin_app_meta))
        .route("/:workspace/:app/admin/pages/:page/components/:idx/edit", post(admin_edit_component_source))
        .route("/:workspace/:app/admin/pages/:page/components/:idx/delete", post(admin_delete_component))
        .route("/:workspace/:app/admin/entities-list", get(admin_entities_list))
        .route("/:workspace/:app/admin/entities/add", post(admin_add_entity))
        .route("/:workspace/:app/admin/entities/:name/source", get(admin_entity_source))
        .route("/:workspace/:app/admin/entities/:name/edit", post(admin_edit_entity))
        .route("/:workspace/:app/admin/entities/:name/delete", post(admin_delete_entity))
        .route("/:workspace/:app/admin/queries-list", get(admin_queries_list))
        .route("/:workspace/:app/admin/queries/add", post(admin_add_query))
        .route("/:workspace/:app/admin/queries/:name/source", get(admin_query_source))
        .route("/:workspace/:app/admin/queries/:name/edit", post(admin_edit_query))
        .route("/:workspace/:app/admin/queries/:name/delete", post(admin_delete_query))
        .route("/:workspace/:app/admin/nav-list", get(admin_nav_list))
        .route("/:workspace/:app/admin/nav/add", post(admin_add_nav_item))
        .route("/:workspace/:app/admin/nav/reorder", post(admin_reorder_nav))
        .route("/:workspace/:app/admin/nav/:idx/source", get(admin_nav_item_source))
        .route("/:workspace/:app/admin/nav/:idx/edit", post(admin_edit_nav_item))
        .route("/:workspace/:app/admin/nav/:idx/delete", post(admin_delete_nav_item))
        .route("/:workspace/:app/admin/settings", get(admin_settings_get).post(admin_settings_set))
        .route("/:workspace/:app/admin/destroy", post(admin_destroy_app))
        .route("/:workspace/:app/admin/destroy-workspace", post(admin_destroy_workspace))
        .route("/pgapp/builder/admin/apps/create-pending", post(admin_create_pending_app))
        .route("/:workspace/:app/:page", get(show))
        .route("/:workspace/:app/:page/region/:query", get(region_fragment))
        .route("/:workspace/:app/:page/c/:idx/create", post(create))
        .route("/:workspace/:app/:page/c/:idx/update/:id", post(update))
        .route("/:workspace/:app/:page/c/:idx/delete/:id", post(delete))
        .route("/:workspace/:app/:page/c/:idx/run", post(run_action))
        .route("/:workspace/:app/:page/c/:idx/call/:op_idx", post(call_dynamic_action))
        .route("/:workspace/:app/:page/c/:idx/views", post(save_view))
        .route("/:workspace/:app/:page/c/:idx/views/:vid/delete", post(delete_view))
        .route("/:workspace/:app/:page/c/:idx/csv", get(report_csv))
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_login))
        .with_state(state)
        .layer(ConcurrencyLimitLayer::new(concurrency_limit()))
        // Overrides axum's 2MB default body limit for the whole router —
        // generous enough for a `file_browse` upload, still bounded so a
        // client can't force an unbounded in-memory multipart buffer.
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
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

/// A component's own `requires:` — the per-component counterpart to a
/// page's `required_role`, read generically across every kind (mirrors
/// `meta::sync::component_requires` on the markup side).
fn component_requires(component: &RuntimeComponent) -> Option<&str> {
    match component {
        RuntimeComponent::Report { requires, .. }
        | RuntimeComponent::Form { requires, .. }
        | RuntimeComponent::EditableTable { requires, .. }
        | RuntimeComponent::Chart { requires, .. }
        | RuntimeComponent::Text { requires, .. }
        | RuntimeComponent::Link { requires, .. }
        | RuntimeComponent::Region { requires, .. }
        | RuntimeComponent::DynamicContent { requires, .. }
        | RuntimeComponent::Action { requires, .. }
        | RuntimeComponent::Button { requires, .. }
        | RuntimeComponent::Calendar { requires, .. }
        | RuntimeComponent::Map { requires, .. }
        | RuntimeComponent::FacetedSearch { requires, .. } => requires.as_deref(),
        RuntimeComponent::DynamicAction { .. } => None,
    }
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

/// The `FacetedSearch` (if any) bound to the same entity as a `Report`
/// on this page — same "sibling by shared entity" lookup as
/// `sibling_form_idx`.
fn sibling_faceted_search<'a>(page: &'a RuntimePage, entity_name: &str) -> Option<(usize, &'a Vec<Facet>)> {
    page.components.iter().enumerate().find_map(|(i, c)| match c {
        RuntimeComponent::FacetedSearch { entity, facets, .. } if entity.name == entity_name => Some((i, facets)),
        _ => None,
    })
}

/// Re-serializes a `FacetedSearch`'s currently-active facets back into
/// `&f<fs_idx>_...=...` query-string fragments — the facet-search
/// counterpart of the report's own `filter_qs`/`base_qs`, so sort
/// links, pagination, the CSV download, and saved views all preserve
/// the active facet selection instead of resetting it.
fn facet_query_string(fs_idx: usize, facets: &[FacetFilter]) -> String {
    let mut qs = String::new();
    for f in facets {
        match f {
            FacetFilter::In { column, values } => {
                qs.push_str(&format!("&f{fs_idx}_{column}={}", url_encode(&values.join(","))));
            }
            FacetFilter::Between { column, low_suffix, high_suffix, low, high, .. } => {
                if let Some(low) = low {
                    qs.push_str(&format!("&f{fs_idx}_{column}_{low_suffix}={}", url_encode(low)));
                }
                if let Some(high) = high {
                    qs.push_str(&format!("&f{fs_idx}_{column}_{high_suffix}={}", url_encode(high)));
                }
            }
        }
    }
    qs
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
/// with strings regardless of the underlying Postgres type. `computed`
/// appends a Report's own extra columns (see `model::ComputedColumn`) —
/// empty for every caller except the entity-backed `fetch_report_rows`.
fn select_columns(entity: &RuntimeEntity, computed: &[ComputedColumn]) -> String {
    let mut cols = vec!["id::text as id".to_string()];
    for f in &entity.fields {
        if f.name == "id" {
            continue;
        }
        cols.push(format!("{name}::text as {name}", name = f.name));
    }
    for c in computed {
        cols.push(format!("({sql})::text as {name}", sql = c.sql, name = c.name));
    }
    cols.join(", ")
}

fn row_from_sqlx(
    row: &sqlx::postgres::PgRow,
    entity: &RuntimeEntity,
    computed: &[ComputedColumn],
) -> anyhow::Result<BTreeMap<String, Option<String>>> {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), row.try_get::<Option<String>, _>("id")?);
    for f in &entity.fields {
        if f.name == "id" {
            continue;
        }
        map.insert(f.name.clone(), row.try_get::<Option<String>, _>(f.name.as_str())?);
    }
    for c in computed {
        map.insert(c.name.clone(), row.try_get::<Option<String>, _>(c.name.as_str())?);
    }
    Ok(map)
}

async fn fetch_rows(pool: &PgPool, data_schema: &str, entity: &RuntimeEntity) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    // `order by t.id`, qualified: the select list aliases `id::text as
    // id`, and an unqualified ORDER BY id would bind to that *text*
    // output column, sorting "10" before "2".
    let sql = format!("select {} from {data_schema}.{} t order by t.id", select_columns(entity, &[]), entity.table_name);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(|r| row_from_sqlx(r, entity, &[])).collect()
}

async fn fetch_row(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    id: &str,
) -> anyhow::Result<Option<BTreeMap<String, Option<String>>>> {
    let sql = format!(
        "select {} from {data_schema}.{} where id = $1::integer",
        select_columns(entity, &[]),
        entity.table_name
    );
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(|r| row_from_sqlx(r, entity, &[])).transpose()
}

/// One page of a `Report`'s rows plus whether there's a previous/next
/// page — see `fetch_report_rows` for how that's known without a
/// `COUNT(*)`.
struct ReportPage {
    rows: Vec<BTreeMap<String, Option<String>>>,
    has_prev: bool,
    has_next: bool,
}

/// One `FacetedSearch` facet's currently-selected filter state, already
/// validated against that facet's declared column/kind — see
/// `FacetFilter::from_query`. Carried inside `ReportFilters` so every
/// report row-fetcher picks these up for free through the existing
/// `ReportFilters::to_sql` call, no extra plumbing needed.
#[derive(Debug, Clone)]
enum FacetFilter {
    /// `checkbox_list`: any number of selected values, ORed together.
    In { column: String, values: Vec<String> },
    /// `range`/`date_range`: an inclusive `[low, high]` bound, either
    /// end optional. `cast` is the field's own SQL type (`integer` or
    /// `timestamp`), so the bind compares numerically/chronologically
    /// rather than as text. `low_suffix`/`high_suffix` are the wire
    /// param suffixes this facet's kind uses (`min`/`max` for `range`,
    /// `from`/`to` for `date_range`), kept alongside so re-serializing
    /// back to a query string (`facet_query_string`) uses the same ones
    /// it was parsed from.
    Between {
        column: String,
        cast: &'static str,
        low_suffix: &'static str,
        high_suffix: &'static str,
        low: Option<String>,
        high: Option<String>,
    },
}

impl FacetFilter {
    /// The column this filter applies to — used to exclude one facet's
    /// own selection from its *own* checkbox_list count query (see the
    /// `RuntimeComponent::FacetedSearch` render arm) by column identity,
    /// not by position: `FacetFilter::from_query` only returns *active*
    /// facets, packed contiguously, so its own index never lines up
    /// with the declared facet list's index.
    fn column(&self) -> &str {
        match self {
            FacetFilter::In { column, .. } | FacetFilter::Between { column, .. } => column,
        }
    }

    /// Parses every facet's selection out of the query string for the
    /// `FacetedSearch` component at `fs_idx`, validated against its
    /// declared facets — a column/kind not declared there is silently
    /// ignored (same "just doesn't filter" tolerance as an unknown
    /// column would get, since these are markup-validated at sync time,
    /// not user input). Wire format: `f{fs_idx}_{col}` (checkbox_list,
    /// comma-joined values), `f{fs_idx}_{col}_min`/`_max` (range),
    /// `f{fs_idx}_{col}_from`/`_to` (date_range).
    fn from_query(query: &HashMap<String, String>, fs_idx: usize, facets: &[Facet], entity: &RuntimeEntity) -> Vec<FacetFilter> {
        let mut out = Vec::new();
        for f in facets {
            match f.kind {
                crate::model::FacetKind::CheckboxList => {
                    let key = format!("f{fs_idx}_{}", f.column);
                    if let Some(raw) = query.get(&key) {
                        let values: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        if !values.is_empty() {
                            out.push(FacetFilter::In { column: f.column.clone(), values });
                        }
                    }
                }
                crate::model::FacetKind::Range | crate::model::FacetKind::DateRange => {
                    let (lo_suffix, hi_suffix) = if f.kind == crate::model::FacetKind::Range { ("min", "max") } else { ("from", "to") };
                    let low = query.get(&format!("f{fs_idx}_{}_{lo_suffix}", f.column)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                    let high = query.get(&format!("f{fs_idx}_{}_{hi_suffix}", f.column)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                    if low.is_some() || high.is_some() {
                        let cast = entity.field(&f.column).map(|fd| fd.data_type.sql_cast()).unwrap_or("text");
                        out.push(FacetFilter::Between {
                            column: f.column.clone(),
                            cast,
                            low_suffix: lo_suffix,
                            high_suffix: hi_suffix,
                            low,
                            high,
                        });
                    }
                }
            }
        }
        out
    }
}

/// A report's live filter state, from its `r<idx>_q` (search across
/// visible columns) and `r<idx>_col`/`r<idx>_val` (single-column
/// filter) URL parameters, plus any sibling `FacetedSearch`'s active
/// facets (see `FacetFilter`) — ANDed together with the rest.
#[derive(Debug, Clone, Default)]
struct ReportFilters {
    q: Option<String>,
    col: Option<(String, String)>,
    facets: Vec<FacetFilter>,
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
        Ok(ReportFilters { q: get("q"), col, facets: Vec::new() })
    }

    /// Builds the SQL conditions for these filters. `prefix` qualifies a
    /// plain field reference (e.g. `"t."`); a name in `computed` filters
    /// on its own SQL expression instead, since it isn't a real column
    /// on the underlying table — only its alias in the `SELECT` list is.
    /// `first_param` is the number the first added `$N` placeholder
    /// should use. Column names come from the report's markup-validated
    /// column list only.
    fn to_sql(&self, prefix: &str, columns: &[String], computed: &[ComputedColumn], first_param: usize) -> (Vec<String>, Vec<String>) {
        let expr_for = |col: &str| match computed.iter().find(|c| c.name == col) {
            Some(c) => format!("({})", c.sql),
            None => format!("{prefix}{col}"),
        };
        let mut conditions = Vec::new();
        let mut binds = Vec::new();
        if let Some(q) = &self.q {
            if !columns.is_empty() {
                let n = first_param + binds.len();
                let ors: Vec<String> = columns.iter().map(|c| format!("({})::text ilike ${n}", expr_for(c))).collect();
                conditions.push(format!("({})", ors.join(" or ")));
                binds.push(format!("%{q}%"));
            }
        }
        if let Some((col, val)) = &self.col {
            let n = first_param + binds.len();
            conditions.push(format!("({})::text ilike ${n}", expr_for(col)));
            binds.push(format!("%{val}%"));
        }
        for facet in &self.facets {
            match facet {
                FacetFilter::In { column, values } => {
                    let mut ors = Vec::new();
                    for v in values {
                        let n = first_param + binds.len();
                        ors.push(format!("{prefix}{column} = ${n}"));
                        binds.push(v.clone());
                    }
                    conditions.push(format!("({})", ors.join(" or ")));
                }
                FacetFilter::Between { column, cast, low, high, .. } => {
                    if let Some(low) = low {
                        let n = first_param + binds.len();
                        conditions.push(format!("{prefix}{column} >= ${n}::{cast}"));
                        binds.push(low.clone());
                    }
                    if let Some(high) = high {
                        let n = first_param + binds.len();
                        conditions.push(format!("{prefix}{column} <= ${n}::{cast}"));
                        binds.push(high.clone());
                    }
                }
            }
        }
        (conditions, binds)
    }
}

/// A report's live column sort, from its `r<idx>_sort=<col>:<asc|desc>`
/// URL parameter — Interactive Report's clickable column-header sort.
/// Choosing any sort switches an entity-backed report from keyset to
/// offset pagination (keyset assumes `id` order, which a custom sort
/// breaks), the same pagination style query/collection-backed reports
/// already use.
#[derive(Debug, Clone)]
struct SortSpec {
    column: String,
    desc: bool,
}

impl SortSpec {
    /// `col` must be one of the report's own columns or computed
    /// columns — same trust boundary as `ReportFilters`' `col`, since
    /// it's spliced directly into SQL as an identifier.
    fn from_query(
        query: &HashMap<String, String>,
        idx: usize,
        columns: &[String],
        computed: &[ComputedColumn],
    ) -> Result<Option<Self>, AppError> {
        let Some(raw) = query.get(&format!("r{idx}_sort")).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let (col, dir) = raw.split_once(':').unwrap_or((raw, "asc"));
        if !columns.contains(&col.to_string()) && !computed.iter().any(|c| c.name == col) {
            return Err((StatusCode::BAD_REQUEST, format!("cannot sort on '{col}': not a column of this report")));
        }
        let desc = match dir {
            "asc" => false,
            "desc" => true,
            other => return Err((StatusCode::BAD_REQUEST, format!("invalid sort direction '{other}'"))),
        };
        Ok(Some(SortSpec { column: col.to_string(), desc }))
    }

    /// The SQL `ORDER BY` expression for this sort — a computed
    /// column's own SQL expression, or a plain qualified field
    /// reference otherwise (mirrors `ReportFilters::to_sql`'s `expr_for`).
    fn order_expr(&self, prefix: &str, computed: &[ComputedColumn]) -> String {
        match computed.iter().find(|c| c.name == self.column) {
            Some(c) => format!("({})", c.sql),
            None => format!("{prefix}{}", self.column),
        }
    }

    fn dir_str(&self) -> &'static str {
        if self.desc {
            "desc"
        } else {
            "asc"
        }
    }
}

/// Reads one caller's page of a named collection — OFFSET-paginated
/// like a query-backed report (a collection has no assumed sort key
/// beyond its own insertion `seq`, and collections are small enough in
/// practice that `COUNT(*)`-free keyset pagination isn't worth the
/// complexity). The `app_id`/`caller_key`/`name` filter is baked in
/// here, not written by the app author — see `EntityDef::source_collection`.
#[allow(clippy::too_many_arguments)]
async fn fetch_collection_page(
    pool: &PgPool,
    app_id: i32,
    caller_key: &str,
    collection_name: &str,
    entity: &RuntimeEntity,
    page_size: i64,
    page_num: i64,
    sort: Option<&SortSpec>,
) -> anyhow::Result<(Vec<BTreeMap<String, Option<String>>>, bool)> {
    let offset = (page_num - 1).max(0) * page_size;
    // A collection has no schema, so a custom sort can only compare the
    // JSONB payload as text (`data->>'col'`) — good enough for the
    // common case (names, statuses) but not numeric-aware; `id` (really
    // `seq`, the collection's own insertion order) is the one column
    // that's a real integer column, so it sorts numerically as normal.
    let order_by = match sort {
        Some(s) if s.column == "id" => format!("seq {}", s.dir_str()),
        Some(s) => format!("(data->>'{}') {}", s.column, s.dir_str()),
        None => "seq asc".to_string(),
    };
    let sql = format!(
        "select seq, data from pgapp_meta.collections
          where app_id = $1 and caller_key = $2 and name = $3
          order by {order_by}
          offset $4 limit $5"
    );
    let json_rows: Vec<(i32, serde_json::Value)> = sqlx::query_as(&sql)
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
    computed: &[ComputedColumn],
    filters: &ReportFilters,
    filter_columns: &[String],
    page_size: i64,
    after: Option<&str>,
    before: Option<&str>,
) -> anyhow::Result<ReportPage> {
    let cols = select_columns(entity, computed);
    let lim = page_size + 1;

    // Filter conditions first ($1..), then the keyset cursor.
    let (mut conditions, binds) = filters.to_sql("t.", filter_columns, computed, 1);
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
    let mut rows: Vec<BTreeMap<String, Option<String>>> =
        db_rows.iter().map(|r| row_from_sqlx(r, entity, computed)).collect::<anyhow::Result<_>>()?;

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

/// Plain OFFSET pagination for an entity-backed `Report` when a
/// column sort is active — `fetch_report_rows`'s keyset cursor assumes
/// ascending `id` order, which a custom `ORDER BY` breaks, so a sort
/// switches to the same offset-pagination style query/collection-backed
/// reports already use (see `SortSpec`).
#[allow(clippy::too_many_arguments)]
async fn fetch_report_rows_sorted(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    computed: &[ComputedColumn],
    filters: &ReportFilters,
    filter_columns: &[String],
    sort: &SortSpec,
    page_size: i64,
    page_num: i64,
) -> anyhow::Result<(Vec<BTreeMap<String, Option<String>>>, bool)> {
    let cols = select_columns(entity, computed);
    let offset = (page_num - 1).max(0) * page_size;
    let (conditions, binds) = filters.to_sql("t.", filter_columns, computed, 1);
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("where {}", conditions.join(" and "))
    };
    let order_expr = sort.order_expr("t.", computed);
    let sql = format!(
        "select {cols} from {data_schema}.{} t {where_clause} order by {order_expr} {} limit {} offset {offset}",
        entity.table_name,
        sort.dir_str(),
        page_size + 1,
    );
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b.as_str());
    }
    let db_rows = query.fetch_all(pool).await?;
    let mut rows: Vec<BTreeMap<String, Option<String>>> =
        db_rows.iter().map(|r| row_from_sqlx(r, entity, computed)).collect::<anyhow::Result<_>>()?;
    let has_next = rows.len() as i64 > page_size;
    rows.truncate(page_size as usize);
    Ok((rows, has_next))
}

/// One `<col> -> agg(col)::text` SQL expression per configured
/// aggregate — shared by the entity- and query-backed aggregate
/// fetchers below. `count` accepts any column type; `sum`/`avg`/`min`/
/// `max` cast through `numeric` first, since the underlying column's
/// real Postgres type isn't known at this layer (same "cast to text,
/// let Postgres do the real work" approach as `select_columns`).
fn aggregate_select_exprs(aggregates: &HashMap<String, AggregateFn>, computed: &[ComputedColumn]) -> Vec<String> {
    aggregates
        .iter()
        .map(|(col, agg)| {
            let inner = match computed.iter().find(|c| c.name == *col) {
                Some(c) => format!("({})", c.sql),
                None => format!("t.{col}"),
            };
            match agg {
                AggregateFn::Count => format!(r#"(count({inner}))::text as "{col}""#),
                other => format!(r#"({}(({inner})::numeric))::text as "{col}""#, other.as_str()),
            }
        })
        .collect()
}

/// Interactive Report's footer aggregates, computed over the report's
/// whole *filtered* result set (every page, not just the one on
/// screen) — an entity-backed report's counterpart to
/// `fetch_report_rows`/`fetch_report_rows_sorted`.
async fn fetch_report_aggregates_entity(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    computed: &[ComputedColumn],
    filters: &ReportFilters,
    filter_columns: &[String],
    aggregates: &HashMap<String, AggregateFn>,
) -> anyhow::Result<HashMap<String, Option<String>>> {
    if aggregates.is_empty() {
        return Ok(HashMap::new());
    }
    let exprs = aggregate_select_exprs(aggregates, computed);
    let (conditions, binds) = filters.to_sql("t.", filter_columns, computed, 1);
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("where {}", conditions.join(" and "))
    };
    let sql = format!("select {} from {data_schema}.{} t {where_clause}", exprs.join(", "), entity.table_name);
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b.as_str());
    }
    let row = query.fetch_one(pool).await?;
    let mut out = HashMap::new();
    for col in aggregates.keys() {
        out.insert(col.clone(), row.try_get::<Option<String>, _>(col.as_str())?);
    }
    Ok(out)
}

/// The query-backed counterpart of `fetch_report_aggregates_entity`:
/// wraps the named query's own SQL the same way `run_named_query_page`
/// does, aggregating over its filtered rows instead of the entity's
/// table directly.
async fn fetch_report_aggregates_query(
    pool: &PgPool,
    data_schema: &str,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
    where_clause: &str,
    extra_binds: &[String],
    aggregates: &HashMap<String, AggregateFn>,
) -> anyhow::Result<HashMap<String, Option<String>>> {
    if aggregates.is_empty() {
        return Ok(HashMap::new());
    }
    let exprs = aggregate_select_exprs(aggregates, &[]);
    let sql = format!("select {} from ({}) as t {}", exprs.join(", "), rq.sql, where_clause);
    let mut query = sqlx::query(&sql);
    for name in &rq.bind_names {
        query = query.bind(ctx.get(name).map(|s| s.as_str()));
    }
    for bind in extra_binds {
        query = query.bind(bind.as_str());
    }
    let mut conn = crate::meta::scoped_conn(pool, data_schema).await?;
    let row = query.fetch_one(&mut *conn).await?;
    let mut out = HashMap::new();
    for col in aggregates.keys() {
        out.insert(col.clone(), row.try_get::<Option<String>, _>(col.as_str())?);
    }
    Ok(out)
}

/// Parses a `?cal<idx>=YYYY-MM` query param into `(year, month)`;
/// `None` on anything malformed, so the caller falls back to today's
/// month rather than erroring the whole page over a hand-edited URL.
fn parse_year_month(s: &str) -> Option<(i32, u32)> {
    let (y, m) = s.split_once('-')?;
    let year: i32 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    if (1..=12).contains(&month) {
        Some((year, month))
    } else {
        None
    }
}

/// Every row of `entity` whose `date_field` (cast to `date`) falls in
/// `year`/`month`, as `(day-of-month, id, title_field's value)` —
/// `Calendar`'s whole-month fetch, deliberately unpaginated and with no
/// query/collection-backed sourcing (see the entity checks in
/// `meta::sync::build_component_config`'s `Calendar` arm): a month grid
/// only ever has 28-31 cells, so there's no page to turn within it.
async fn fetch_calendar_entries(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    date_field: &str,
    title_field: &str,
    year: i32,
    month: u32,
) -> anyhow::Result<Vec<(u32, String, String)>> {
    let (next_y, next_m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let start = format!("{year:04}-{month:02}-01");
    let end = format!("{next_y:04}-{next_m:02}-01");
    let sql = format!(
        "select id::text as id, extract(day from ({date_field})::date)::int as day, ({title_field})::text as title \
         from {data_schema}.{table} \
         where ({date_field})::date >= $1::date and ({date_field})::date < $2::date \
         order by ({date_field})::date asc, id asc",
        table = entity.table_name
    );
    let db_rows = sqlx::query(&sql).bind(&start).bind(&end).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(db_rows.len());
    for row in &db_rows {
        let id: Option<String> = row.try_get("id")?;
        let day: Option<i32> = row.try_get("day")?;
        let entry_title: Option<String> = row.try_get("title")?;
        out.push((day.unwrap_or(0).max(0) as u32, id.unwrap_or_default(), entry_title.unwrap_or_default()));
    }
    Ok(out)
}

/// Every row of `entity` with non-null `lat_field`/`lng_field`, as
/// `(lat, lng, id, title_field's value)` — `Map`'s whole-entity fetch,
/// deliberately unpaginated (a marker scatter has no natural "page"
/// concept) and, like `Calendar`, restricted to a real data table (see
/// the entity checks in `meta::sync::build_component_config`'s `Map`
/// arm).
async fn fetch_map_points(
    pool: &PgPool,
    data_schema: &str,
    entity: &RuntimeEntity,
    lat_field: &str,
    lng_field: &str,
    title_field: &str,
) -> anyhow::Result<Vec<(f64, f64, String, String)>> {
    let sql = format!(
        "select id::text as id, ({lat_field})::double precision as lat, ({lng_field})::double precision as lng, \
         ({title_field})::text as title \
         from {data_schema}.{table} \
         where {lat_field} is not null and {lng_field} is not null \
         order by id asc",
        table = entity.table_name
    );
    let db_rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(db_rows.len());
    for row in &db_rows {
        let id: Option<String> = row.try_get("id")?;
        let lat: Option<f64> = row.try_get("lat")?;
        let lng: Option<f64> = row.try_get("lng")?;
        let title: Option<String> = row.try_get("title")?;
        if let (Some(lat), Some(lng)) = (lat, lng) {
            out.push((lat, lng, id.unwrap_or_default(), title.unwrap_or_default()));
        }
    }
    Ok(out)
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
    // A component's own `requires:` (on top of whatever the page itself
    // requires, already checked before this is called) silently hides
    // it from view — same "just don't show it" precedent as a role-
    // gated nav item (see `visible_nav`), not a hard error, since a page
    // can otherwise be entirely open to this user.
    if auth::authorize(data, component_requires(component), auth_ctx).is_err() {
        return Ok(String::new());
    }
    match component {
        RuntimeComponent::Text { text, html, .. } => Ok(render::text_html(text, html)),
        RuntimeComponent::Link { label, target_page, html, .. } => Ok(render::link_html(app, label, target_page, html)),
        RuntimeComponent::Region { label, query: qname, columns, html, .. } => {
            Ok(render::region_html(label, qname, regions, columns, html))
        }

        // PL/SQL Dynamic Content: runs once per page load, server-side —
        // unlike the ajax callback (DaOp::Call), which only runs on a
        // client-side event. A failed module shows its error inline
        // instead of failing the whole page, same soft-fail precedent as
        // Report::before_load (run_before_load above).
        RuntimeComponent::DynamicContent { label, name, config, html, .. } => {
            let module = state.actions.get(name.as_str()).ok_or_else(|| {
                anyhow::anyhow!("dynamic_content '{label}' calls unknown module '{name}' (not registered — rebuild?)")
            })?;
            let outcome = module
                .run(ActionContext {
                    pool: &state.pool,
                    app: &data.app,
                    page,
                    config,
                    values: query,
                    caller_key,
                })
                .await;
            let content = match outcome {
                Ok(result) => result,
                Err(e) => format!(
                    r#"<div class="pgapp-alert pgapp-alert-error">{}</div>"#,
                    crate::html::escape(&e.to_string())
                ),
            };
            Ok(render::dynamic_content_html(label, &content, html))
        }

        // A dynamic action renders nothing in the body — show() gathers
        // them all into one JSON script for the runtime.js dispatcher.
        RuntimeComponent::DynamicAction { .. } => Ok(String::new()),

        RuntimeComponent::Action { label, name, html, .. } => {
            Ok(render::action_html(app, page_name, idx, label, name, html))
        }

        RuntimeComponent::Button { label, behavior, html, .. } => match behavior {
            meta::ButtonBehavior::Redirect { target_page, extra_params } => {
                Ok(render::button_redirect_html(app, label, target_page, extra_params, query, html))
            }
            meta::ButtonBehavior::RunAction { name, .. } => Ok(render::action_html(app, page_name, idx, label, name, html)),
        },

        RuntimeComponent::Chart { title, query: qname, chart_type, x, y, html, .. } => {
            let rq = page
                .resolve_query(&data.app, qname)
                .ok_or_else(|| anyhow::anyhow!("chart '{title}' references unknown query '{qname}'"))?;
            let ctx = bind_context(query, None);
            let rows = run_named_query_rows(&state.pool, &data.app.data_schema, rq, &ctx).await?;
            Ok(render::chart_html(title, chart_type, x, y, &rows, &data.chart_lib, html))
        }

        RuntimeComponent::Calendar {
            title,
            entity,
            date_field,
            title_field,
            link_page,
            html,
            ..
        } => {
            let param = format!("cal{idx}");
            let (year, month) = query
                .get(&param)
                .and_then(|s| parse_year_month(s))
                .unwrap_or_else(|| {
                    let (y, m, _d) = crate::dateutil::today_ymd();
                    (y, m)
                });
            let entries =
                fetch_calendar_entries(&state.pool, &data.app.data_schema, entity, date_field, title_field, year, month)
                    .await?;
            Ok(render::calendar_html(app, page_name, idx, title, year, month, &entries, link_page.as_deref(), html))
        }

        RuntimeComponent::Map {
            title,
            entity,
            lat_field,
            lng_field,
            title_field,
            link_page,
            html,
            ..
        } => {
            let points = fetch_map_points(&state.pool, &data.app.data_schema, entity, lat_field, lng_field, title_field).await?;
            Ok(render::map_html(app, title, &points, link_page.as_deref(), html))
        }

        RuntimeComponent::FacetedSearch { title, entity, facets, html, .. } => {
            let report_idx = companion_report_idx(page, &entity.name);
            // This facet search's own currently-active selections (wire
            // format: `f{idx}_...`, same `idx` this component renders
            // at) — needed both to pre-check/pre-fill each facet's
            // control and, per checkbox_list facet below, to exclude
            // that one facet's own selection from its count query (so
            // checking a box narrows the *other* facets' counts without
            // making its own other options disappear).
            let all_facets = FacetFilter::from_query(query, idx, facets, entity);

            let (report_filters, report_columns) = match report_idx.and_then(|ridx| page.components.get(ridx)) {
                Some(RuntimeComponent::Report { columns, .. }) => {
                    let f = ReportFilters::from_query(query, report_idx.unwrap(), columns).unwrap_or_default();
                    (f, columns.clone())
                }
                _ => (ReportFilters::default(), Vec::new()),
            };

            let mut ui = Vec::new();
            for f in facets {
                match f.kind {
                    crate::model::FacetKind::CheckboxList => {
                        let mut filters = report_filters.clone();
                        filters.facets = all_facets.iter().filter(|ff| ff.column() != f.column).cloned().collect();
                        let (conditions, binds) = filters.to_sql("t.", &report_columns, &[], 1);
                        let where_clause = if conditions.is_empty() {
                            String::new()
                        } else {
                            format!("where {}", conditions.join(" and "))
                        };
                        let sql = format!(
                            "select (t.{col})::text as v, count(*) as cnt from {schema}.{table} t {where_clause} group by t.{col} order by t.{col}",
                            col = f.column,
                            schema = data.app.data_schema,
                            table = entity.table_name,
                        );
                        let mut sql_query = sqlx::query(&sql);
                        for b in &binds {
                            sql_query = sql_query.bind(b.as_str());
                        }
                        let db_rows = sql_query.fetch_all(&state.pool).await?;
                        let selected: Vec<String> = all_facets
                            .iter()
                            .find_map(|ff| match ff {
                                FacetFilter::In { column, values } if *column == f.column => Some(values.clone()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let options: Vec<(String, i64, bool)> = db_rows
                            .iter()
                            .map(|r| {
                                let v: Option<String> = r.try_get("v").unwrap_or(None);
                                let v = v.unwrap_or_default();
                                let cnt: i64 = r.try_get("cnt").unwrap_or(0);
                                let checked = selected.contains(&v);
                                (v, cnt, checked)
                            })
                            .collect();
                        ui.push(render::FacetUi::CheckboxList { column: f.column.clone(), options });
                    }
                    crate::model::FacetKind::Range | crate::model::FacetKind::DateRange => {
                        let (low, high) = all_facets
                            .iter()
                            .find_map(|ff| match ff {
                                FacetFilter::Between { column, low, high, .. } if *column == f.column => Some((low.clone(), high.clone())),
                                _ => None,
                            })
                            .unwrap_or((None, None));
                        if f.kind == crate::model::FacetKind::Range {
                            ui.push(render::FacetUi::Range { column: f.column.clone(), min: low, max: high });
                        } else {
                            ui.push(render::FacetUi::DateRange { column: f.column.clone(), from: low, to: high });
                        }
                    }
                }
            }

            Ok(render::faceted_search_html(app, page_name, idx, title, report_idx, &ui, html))
        }

        RuntimeComponent::Report {
            title,
            entity,
            columns,
            source_query,
            link_column,
            page_size,
            before_load,
            computed,
            formats,
            aggregates,
            break_on,
            highlights,
            headings,
            aligns,
            display,
            html,
            ..
        } => {
            let before_load_warning = match before_load {
                Some(pre) => run_before_load(state, data, page, query, caller_key, pre).await,
                None => None,
            };

            let form_idx = sibling_form_idx(page, &entity.name);
            let p_after = format!("r{idx}_after");
            let p_before = format!("r{idx}_before");
            let p_page = format!("r{idx}_page");

            let mut filters = ReportFilters::from_query(query, idx, columns)
                .map_err(|(_, msg)| anyhow::anyhow!(msg))?;
            // A sibling FacetedSearch on the same entity ANDs its active
            // facets into this report's own filters — same "sibling by
            // shared entity" convention as the companion Form.
            let facet_qs = match sibling_faceted_search(page, &entity.name) {
                Some((fs_idx, facets_decl)) => {
                    filters.facets = FacetFilter::from_query(query, fs_idx, facets_decl, entity);
                    facet_query_string(fs_idx, &filters.facets)
                }
                None => String::new(),
            };
            // Control Break needs its rows actually grouped together to
            // read as intended — an explicit column-header sort always
            // wins, but absent one, `break_on` supplies its own default
            // sort (ascending) rather than leaving row order undefined.
            let sort = SortSpec::from_query(query, idx, columns, computed)
                .map_err(|(_, msg)| anyhow::anyhow!(msg))?
                .or_else(|| break_on.as_ref().map(|col| SortSpec { column: col.clone(), desc: false }));
            // Filter/sort params re-serialized for pagination and
            // column-header links, so Prev/Next/re-sorting stay inside
            // the same filtered (and, for sort, ordered) result set.
            let mut filter_qs = String::new();
            if let Some(q) = &filters.q {
                filter_qs.push_str(&format!("&r{idx}_q={}", url_encode(q)));
            }
            if let Some((col, val)) = &filters.col {
                filter_qs.push_str(&format!("&r{idx}_col={}&r{idx}_val={}", url_encode(col), url_encode(val)));
            }
            if let Some(s) = &sort {
                filter_qs.push_str(&format!("&r{idx}_sort={}:{}", url_encode(&s.column), s.dir_str()));
            }
            filter_qs.push_str(&facet_qs);

            // A report is query-paginated when it declares `source:` or
            // its entity is query-backed (`entity ... from query`);
            // offset-paginated the same way when the entity is
            // collection-backed instead, or when a column sort is active
            // (keyset assumes `id` order, which a custom sort breaks —
            // see `SortSpec`); keyset-paginated only in the plain,
            // unsorted entity-backed case.
            let effective_query = source_query.as_deref().or(entity.source_query.as_deref());

            // Row highlight rules ride along as extra hidden computed
            // columns (see `model::highlight_hidden_name`) — the entity-
            // backed row-fetchers below already splice `computed` into
            // their `SELECT` and the resulting row map, so this reuses
            // that machinery instead of needing its own. Only entity-
            // backed reports get highlights (validated at sync time),
            // so this merged list is only ever used on that path.
            let computed_with_highlights: Vec<ComputedColumn> = computed
                .iter()
                .cloned()
                .chain(highlights.iter().enumerate().map(|(i, h)| ComputedColumn {
                    name: crate::model::highlight_hidden_name(i),
                    sql: h.when.clone(),
                }))
                .collect();

            let (rows, prev_href, next_href) = if let Some(qname) = effective_query {
                let rq = page
                    .resolve_query(&data.app, qname)
                    .ok_or_else(|| anyhow::anyhow!("report '{title}' sources from unknown query '{qname}'"))?;
                let ctx = bind_context(query, None);
                let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                let (conditions, binds) = filters.to_sql("t.", columns, &[], rq.bind_names.len() + 1);
                let where_clause = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("where {}", conditions.join(" and "))
                };
                let sort_arg = sort.as_ref().map(|s| (s.column.as_str(), s.desc));
                let (json_rows, has_next) = run_named_query_page(
                    &state.pool,
                    &data.app.data_schema,
                    rq,
                    &ctx,
                    &where_clause,
                    &binds,
                    *page_size,
                    page_num,
                    sort_arg,
                )
                .await?;
                let rows: Vec<_> = json_rows.into_iter().map(query_engine::json_row_to_map).collect();
                let prev_href = (page_num > 1).then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num - 1));
                let next_href = has_next.then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num + 1));
                (rows, prev_href, next_href)
            } else if let Some(coll_name) = entity.source_collection.as_deref() {
                let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                let (rows, has_next) = fetch_collection_page(
                    &state.pool,
                    data.app.id,
                    caller_key,
                    coll_name,
                    entity,
                    *page_size,
                    page_num,
                    sort.as_ref(),
                )
                .await?;
                let prev_href = (page_num > 1).then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num - 1));
                let next_href = has_next.then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num + 1));
                (rows, prev_href, next_href)
            } else if let Some(s) = &sort {
                let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                let (rows, has_next) = fetch_report_rows_sorted(
                    &state.pool,
                    &data.app.data_schema,
                    entity,
                    &computed_with_highlights,
                    &filters,
                    columns,
                    s,
                    *page_size,
                    page_num,
                )
                .await?;
                let prev_href = (page_num > 1).then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num - 1));
                let next_href = has_next.then(|| format!("/{app}/{page_name}?{p_page}={}{filter_qs}", page_num + 1));
                (rows, prev_href, next_href)
            } else {
                let after = query.get(&p_after).map(|s| s.as_str());
                let before = query.get(&p_before).map(|s| s.as_str());
                let rp = fetch_report_rows(
                    &state.pool,
                    &data.app.data_schema,
                    entity,
                    &computed_with_highlights,
                    &filters,
                    columns,
                    *page_size,
                    after,
                    before,
                )
                .await?;
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

            // Aggregates run over the report's whole *filtered* result
            // set, independent of pagination/sort — a separate query
            // from the row fetch above, not a byproduct of it. Not yet
            // supported for collection-backed reports (an internal
            // pgapp extension, not something a real APEX migration
            // needs) — those just render without a footer row.
            let agg_values = if aggregates.is_empty() {
                HashMap::new()
            } else if let Some(qname) = effective_query {
                let rq = page
                    .resolve_query(&data.app, qname)
                    .ok_or_else(|| anyhow::anyhow!("report '{title}' sources from unknown query '{qname}'"))?;
                let ctx = bind_context(query, None);
                let (conditions, binds) = filters.to_sql("t.", columns, &[], rq.bind_names.len() + 1);
                let where_clause = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("where {}", conditions.join(" and "))
                };
                fetch_report_aggregates_query(&state.pool, &data.app.data_schema, rq, &ctx, &where_clause, &binds, aggregates).await?
            } else if entity.source_collection.is_some() {
                HashMap::new()
            } else {
                fetch_report_aggregates_entity(&state.pool, &data.app.data_schema, entity, computed, &filters, columns, aggregates).await?
            };

            let mut extras = report_extras(app, state, data, page_name, idx, &filters, auth_ctx).await?;
            extras.warning = before_load_warning;
            extras.facet_qs = facet_qs;

            let sort_arg = sort.as_ref().map(|s| (s.column.as_str(), s.desc));
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
                formats,
                aggregates,
                &agg_values,
                break_on.as_deref(),
                highlights,
                headings,
                aligns,
                display,
                sort_arg,
                html,
            ))
        }

        RuntimeComponent::Form { title, entity, fields, item_types, field_html, html, .. } => {
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

        RuntimeComponent::EditableTable { title, entity, columns, item_types, field_html, html, .. } => {
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
            for (key, param) in [("q", "q"), ("col", "col"), ("val", "val"), ("sort", "sort")] {
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
        facet_qs: String::new(),
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
    // dispatcher binds on DOMContentLoaded. Each gets its own
    // component index merged in as "idx" — needed to address a `call`
    // op's own `/c/:idx/call/:op_idx` route (see call_dynamic_action);
    // every other op ignores it.
    let dyn_actions: Vec<serde_json::Value> = page
        .components
        .iter()
        .enumerate()
        .filter_map(|(idx, c)| match c {
            RuntimeComponent::DynamicAction { config } => {
                let mut with_idx = config.clone();
                if let Some(obj) = with_idx.as_object_mut() {
                    obj.insert("idx".to_string(), serde_json::json!(idx));
                }
                Some(with_idx)
            }
            _ => None,
        })
        .collect();
    if !dyn_actions.is_empty() {
        let dyn_actions_refs: Vec<&serde_json::Value> = dyn_actions.iter().collect();
        body.push_str(&render::dynamic_actions_script(&dyn_actions_refs));
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
        RuntimeComponent::Region { label, query, columns, html, .. } if *query == query_name => {
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
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
    let (name, config) = match component {
        RuntimeComponent::Action { name, config, .. } => (name, config),
        RuntimeComponent::Button { behavior: meta::ButtonBehavior::RunAction { name, config }, .. } => (name, config),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("page '{page_name}' component #{idx} is not an action"),
            ));
        }
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

/// POST /:workspace/:app/:page/c/:idx/call/:op_idx — the "ajax
/// callback": a `DynamicAction`'s `call <action> (...) into <target>`
/// op (`model::DaOp::Call`), invoked from `pgapp.runDynamicActionCall`
/// in `/runtime.js` via `fetch()` instead of a full-page form POST like
/// `run_action` above. `idx` addresses the `DynamicAction` component
/// itself; `op_idx` addresses which of its `ops` array entries to run,
/// since one dynamic action can hold more than one `call`. Runs the
/// exact same `ActionContext`/module dispatch as `run_action` — the
/// only difference is the response shape (JSON, not a redirect), since
/// the caller is client-side JS that applies the result to `target`
/// itself rather than a full page navigation showing a notice banner.
async fn call_dynamic_action(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app, page_name, idx, op_idx)): Path<(String, String, String, usize, usize)>,
    Query(query): Query<HashMap<String, String>>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    auth::authorize(&data, component_requires(component), &auth_ctx)?;

    let RuntimeComponent::DynamicAction { config } = component else {
        return Err((StatusCode::BAD_REQUEST, format!("page '{page_name}' component #{idx} is not a dynamic action")));
    };
    let op = config
        .get("ops")
        .and_then(|v| v.as_array())
        .and_then(|ops| ops.get(op_idx))
        .ok_or_else(|| (StatusCode::NOT_FOUND, "no such dynamic-action op".to_string()))?;
    if op.get("op").and_then(|v| v.as_str()) != Some("call") {
        return Err((StatusCode::BAD_REQUEST, "that dynamic-action op isn't a 'call'".to_string()));
    }
    let name = op.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
        (StatusCode::INTERNAL_SERVER_ERROR, "malformed 'call' op: missing 'action'".to_string())
    })?;
    let op_config = op.get("config").cloned().unwrap_or_else(|| serde_json::json!({}));

    let module = state.actions.get(name).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("action module '{name}' is in metadata but not registered (rebuild?)"),
        )
    })?;

    // Same merge order as run_action: URL query params, form fields on top.
    let mut merged = query.clone();
    merged.extend(values);

    let outcome = module
        .run(ActionContext {
            pool: &state.pool,
            app: &data.app,
            page,
            config: &op_config,
            values: &merged,
            caller_key: &caller.0,
        })
        .await;

    match outcome {
        Ok(result) => Ok(axum::Json(json!({ "ok": true, "result": result })).into_response()),
        Err(e) => Ok((StatusCode::BAD_REQUEST, axum::Json(json!({ "ok": false, "error": e.to_string() }))).into_response()),
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
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
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
    if let Some(sort) = get("sort") {
        params.insert("sort".into(), sort.into());
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

/// Quotes a CSV field only when it needs it (contains a comma, quote,
/// or newline) — RFC 4180's minimal-quoting rule, doubling any embedded
/// quotes.
fn csv_field(s: &str) -> String {
    if s.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Interactive Report's "Download CSV": streams every row matching the
/// report's *current* filters and sort — not just the page on screen —
/// as a CSV of its declared `columns` (formatted the same way the table
/// displays them). Reuses each display mode's own paginated row-fetcher
/// with a page size large enough to be effectively unlimited, the same
/// "piggyback on existing plumbing" approach as aggregates and
/// highlights, rather than a new unpaginated query path.
async fn report_csv(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app, page_name, idx)): Path<(String, String, String, usize)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    let page = page_or_404(&data.app, &page_name)?;
    auth::authorize(&data, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
    let RuntimeComponent::Report { title, entity, columns, source_query, computed, formats, headings, .. } = component else {
        return Err((StatusCode::BAD_REQUEST, format!("component #{idx} is not a report")));
    };

    let mut filters = ReportFilters::from_query(&query, idx, columns)?;
    if let Some((fs_idx, facets_decl)) = sibling_faceted_search(page, &entity.name) {
        filters.facets = FacetFilter::from_query(&query, fs_idx, facets_decl, entity);
    }
    let sort = SortSpec::from_query(&query, idx, columns, computed)?;
    let effective_query = source_query.as_deref().or(entity.source_query.as_deref());

    const CSV_ROW_LIMIT: i64 = 1_000_000;

    let rows: Vec<BTreeMap<String, Option<String>>> = if let Some(qname) = effective_query {
        let rq = page
            .resolve_query(&data.app, qname)
            .ok_or_else(|| anyhow::anyhow!("report '{title}' sources from unknown query '{qname}'"))
            .map_err(err_response)?;
        let ctx = bind_context(&query, None);
        let (conditions, binds) = filters.to_sql("t.", columns, &[], rq.bind_names.len() + 1);
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("where {}", conditions.join(" and "))
        };
        let sort_arg = sort.as_ref().map(|s| (s.column.as_str(), s.desc));
        let (json_rows, _) = run_named_query_page(
            &state.pool,
            &data.app.data_schema,
            rq,
            &ctx,
            &where_clause,
            &binds,
            CSV_ROW_LIMIT,
            1,
            sort_arg,
        )
        .await
        .map_err(err_response)?;
        json_rows.into_iter().map(query_engine::json_row_to_map).collect()
    } else if let Some(coll_name) = entity.source_collection.as_deref() {
        let (rows, _) = fetch_collection_page(&state.pool, data.app.id, &caller.0, coll_name, entity, CSV_ROW_LIMIT, 1, sort.as_ref())
            .await
            .map_err(err_response)?;
        rows
    } else if let Some(s) = &sort {
        let (rows, _) =
            fetch_report_rows_sorted(&state.pool, &data.app.data_schema, entity, computed, &filters, columns, s, CSV_ROW_LIMIT, 1)
                .await
                .map_err(err_response)?;
        rows
    } else {
        let rp = fetch_report_rows(&state.pool, &data.app.data_schema, entity, computed, &filters, columns, CSV_ROW_LIMIT, None, None)
            .await
            .map_err(err_response)?;
        rp.rows
    };

    let mut csv = columns
        .iter()
        .map(|c| csv_field(headings.get(c).map(|h| h.as_str()).unwrap_or(c)))
        .collect::<Vec<_>>()
        .join(",");
    csv.push_str("\r\n");
    for row in &rows {
        let fields: Vec<String> = columns
            .iter()
            .map(|c| {
                let raw = row.get(c).and_then(|v| v.as_deref()).unwrap_or("");
                let display = match formats.get(c) {
                    Some(mask) => mask.apply(raw),
                    None => raw.to_string(),
                };
                csv_field(&display)
            })
            .collect();
        csv.push_str(&fields.join(","));
        csv.push_str("\r\n");
    }

    let filename = format!("{}.csv", title.to_lowercase().replace(' ', "_").replace('"', ""));
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        csv,
    )
        .into_response())
}

/// Oracle APEX's Branch after a DML process: where a `Form` with
/// `after_save` set redirects instead of the default same-page/anchor
/// — `None` for any other component kind, or a `Form` with no
/// `after_save`. `id` is the just-saved row's id; `values` are the
/// submitted form fields — together the only data forwarded fields can
/// draw from (see `model::AfterSave`'s doc comment on that restriction).
fn after_save_target(app: &str, component: &RuntimeComponent, id: &str, values: &HashMap<String, String>) -> Option<String> {
    let RuntimeComponent::Form { after_save: Some(a), .. } = component else {
        return None;
    };
    let mut url = format!("/{app}/{}", a.target_page);
    let mut sep = '?';
    for (field, param) in &a.extra_params {
        let value = if field == "id" { id } else { values.get(field).map(|s| s.as_str()).unwrap_or("") };
        url.push_str(&format!("{sep}{param}={}", url_encode(value)));
        sep = '&';
    }
    Some(url)
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
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
    let (entity, fields, item_types) = writable_fields(component, &page_name, idx)?;
    let branches_after_save = matches!(component, RuntimeComponent::Form { after_save: Some(_), .. });

    match build_value_exprs(entity, fields, item_types, &values, &state.item_types) {
        Ok((columns, exprs, binds)) => {
            let sql = format!(
                "insert into {}.{} ({}) values ({})",
                data.app.data_schema,
                entity.table_name,
                columns.join(", "),
                exprs.join(", ")
            );
            // Branch after save needs the new row's id (to forward as
            // `id`, or just to build the redirect at all when no other
            // field is enough) — otherwise stick to the plain `execute`
            // every other create already used.
            let new_id = if branches_after_save {
                let sql_with_returning = format!("{sql} returning id::text");
                let mut sql_query = sqlx::query_scalar(&sql_with_returning);
                for b in &binds {
                    sql_query = sql_query.bind(b);
                }
                let id: String = sql_query
                    .fetch_one(&state.pool)
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to create row: {e}")))?;
                Some(id)
            } else {
                let mut sql_query = sqlx::query(&sql);
                for b in &binds {
                    sql_query = sql_query.bind(b);
                }
                sql_query
                    .execute(&state.pool)
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to create row: {e}")))?;
                None
            };
            match new_id.and_then(|id| after_save_target(&app, component, &id, &values)) {
                Some(url) => Ok(Redirect::to(&url).into_response()),
                None => Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response()),
            }
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
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
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
            match after_save_target(&app, component, &id, &values) {
                Some(url) => Ok(Redirect::to(&url).into_response()),
                None => Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response()),
            }
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
    auth::authorize(&data, component_requires(component), &auth_ctx)?;
    let (entity, _, _) = writable_fields(component, &page_name, idx)?;

    let sql = format!("delete from {}.{} where id = $1::integer", data.app.data_schema, entity.table_name);
    sqlx::query(&sql)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to delete row: {e}")))?;
    Ok(Redirect::to(&format!("/{app}/{page_name}#pgapp-c{}", redirect_anchor(page, idx))).into_response())
}

/// POST /:workspace/:app/uploads — the `file_browse` item type's
/// upload endpoint (see `item_types::file_browse`'s doc comment): the
/// one route that takes multipart instead of the universal urlencoded
/// `Form` every create/update route uses. Returns `{"id", "filename"}`
/// JSON; the client (`pgapp.uploadFile` in `/runtime.js`) writes
/// `"<id>:<filename>"` into the field's real hidden input itself — this
/// route only knows which *app* an upload belongs to, not which
/// entity/field, so it stays useful for however many `file_browse`
/// fields an app declares.
async fn upload_file(
    State(state): State<Arc<AppState>>,
    Extension(caller): Extension<auth::CallerKey>,
    Path((workspace, app)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let app_key = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app_key)?;
    let data = entry.data();

    let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid upload: {e}")))? else {
        return Err((StatusCode::BAD_REQUEST, "no file field in upload".to_string()));
    };
    let filename = field.file_name().unwrap_or("upload").to_string();
    let content_type = field.content_type().unwrap_or("application/octet-stream").to_string();
    let bytes = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to read upload: {e}")))?;

    let id: i64 = sqlx::query_scalar(
        "insert into pgapp_meta.file_uploads (app_id, caller_key, filename, content_type, data)
         values ($1, $2, $3, $4, $5) returning id",
    )
    .bind(data.app.id)
    .bind(&caller.0)
    .bind(&filename)
    .bind(&content_type)
    .bind(bytes.as_ref())
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to store upload: {e}")))?;

    Ok(axum::Json(json!({ "id": id, "filename": filename })).into_response())
}

/// GET /:workspace/:app/uploads/:id — streams a previously uploaded
/// blob back with its original content-type. Not gated by `caller_key`
/// (see `db/schema.sql`'s `file_uploads` comment): a file referenced
/// from an entity row is visible to whoever can already see that row,
/// same as any other column's value. Scoped to the current app's id so
/// one app can't fetch another's upload by guessing its id.
async fn download_file(
    State(state): State<Arc<AppState>>,
    Path((workspace, app, id)): Path<(String, String, i64)>,
) -> Result<Response, AppError> {
    let app_key = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app_key)?;
    let data = entry.data();

    let row = sqlx::query("select filename, content_type, data from pgapp_meta.file_uploads where id = $1 and app_id = $2")
        .bind(id)
        .bind(data.app.id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to load upload: {e}")))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "no such upload".to_string()))?;

    let filename: String = row.get("filename");
    let content_type: String = row.get("content_type");
    let bytes: Vec<u8> = row.get("data");

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", filename.replace('"', ""))),
        ],
        bytes,
    )
        .into_response())
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
        let (rows, _) = fetch_collection_page(&state.pool, data.app.id, &caller.0, coll_name, entity, i64::MAX / 2, 1, None)
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
    // Same belt-and-suspenders reasoning as `admin_edit_guard`/
    // `admin_reorder_page`: this route predates the App Builder and
    // has no auth of its own when the target app declares no `auth {
    // }` block (see `require_reload_access`) — the App Builder itself
    // declares none, so without this check anyone who knows/guesses
    // this URL could view and overwrite its markup file directly,
    // bypassing every other App-Builder-specific guard.
    if workspace == instance::APP_BUILDER_WORKSPACE_SLUG && app == instance::APP_BUILDER_APP_SLUG {
        return Err((StatusCode::FORBIDDEN, "the App Builder can't edit itself".to_string()));
    }
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
    // See `admin_reload_page`'s identical check just above it.
    if workspace == instance::APP_BUILDER_WORKSPACE_SLUG && app == instance::APP_BUILDER_APP_SLUG {
        return Err((StatusCode::FORBIDDEN, "the App Builder can't edit itself".to_string()));
    }
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

/// GET /:workspace/:app/admin/pages/:page/components/:idx/structured —
/// the App Builder's structured editor's prefill data: component
/// `idx`'s kind plus its full attribute set as JSON (see
/// `RuntimeComponent::to_json`), read straight from the already-synced
/// `RuntimeApp` in memory (no file re-read/re-parse needed) — safe
/// because `meta::sync_app` always re-derives a component's ordinal
/// from file order on every sync, so this index always lines up with
/// `page_reorder`'s (file-based) idx. Client-side JS renders a real
/// per-kind form from this instead of the raw-text textarea
/// `admin_component_source` feeds `pgappSourceEditor`.
async fn admin_component_structured(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page, idx)): Path<(String, String, String, usize)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let data = entry.data();
    let found = data.app.pages.iter().find(|p| p.name == page).and_then(|p| p.components.get(idx));
    match found {
        Some(component) => {
            Ok(axum::Json(json!({"ok": true, "kind": component.kind(), "data": component.to_json()})).into_response())
        }
        None => Ok(axum::Json(json!({"ok": false, "error": format!("no component at index {idx} on page '{page}'")})).into_response()),
    }
}

/// GET /:workspace/:app/admin/pages/:page/app-meta — everything the App
/// Builder's structured component editor needs to populate its
/// dropdowns/checkboxes for a component on `page`: every entity's name
/// and field list, every query visible from this page (app-scoped plus
/// this page's own), every registered item-type/action module name,
/// the fixed chart-type list, every page name (for a `-> page <Name>`
/// target picker), and every named auth scheme. One combined endpoint
/// rather than several, since a single component's editor may need any
/// subset of these depending on its kind. Re-parses the markup file
/// fresh (like `admin_pages_list`) rather than reading `entry.data()`'s
/// already-synced `RuntimeApp`, since an *unused* entity/query (one no
/// existing component references yet) still needs to show up here for
/// a brand new component to bind to.
async fn admin_app_meta(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, page)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<serde_json::Value> = async {
        if std::path::Path::new(&entry.markup_path).is_dir() {
            anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
        }
        let app_def = crate::source::load(&entry.markup_path)?;
        let entities: Vec<serde_json::Value> = app_def
            .entities
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "fields": e.fields.iter().map(|f| json!({"name": f.name, "type": f.ty.as_str()})).collect::<Vec<_>>(),
                })
            })
            .collect();

        let mut query_names: Vec<&str> = app_def.queries.iter().map(|q| q.name.as_str()).collect();
        if let Some(page_def) = app_def.pages.iter().find(|p| p.name == page) {
            for q in &page_def.queries {
                if !query_names.contains(&q.name.as_str()) {
                    query_names.push(&q.name);
                }
            }
        }

        let mut item_type_names: Vec<&str> = state.item_types.keys().copied().collect();
        item_type_names.sort();
        let mut action_names: Vec<&str> = state.actions.keys().copied().collect();
        action_names.sort();
        let page_names: Vec<&str> = app_def.pages.iter().map(|p| p.name.as_str()).collect();
        let auth_scheme_names: Vec<&str> = app_def.auth_schemes.iter().map(|s| s.name.as_str()).collect();

        Ok(json!({
            "entities": entities,
            "queries": query_names,
            "item_types": item_type_names,
            "actions": action_names,
            "chart_types": CHART_TYPES,
            "pages": page_names,
            "auth_schemes": auth_scheme_names,
        }))
    }
    .await;

    match result {
        Ok(v) => Ok(axum::Json(json!({"ok": true, "meta": v})).into_response()),
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

/// Re-reads and re-parses the target app's markup file fresh — shared
/// by every entity/query/nav/settings route below, all of which need
/// the *current on-disk* file rather than `entry.data()`'s already-
/// synced `RuntimeApp` (an entity/query with no components referencing
/// it yet still needs to show up here, same reasoning as
/// `admin_app_meta`).
async fn read_markup_text(entry: &AppEntry) -> anyhow::Result<String> {
    if std::path::Path::new(&entry.markup_path).is_dir() {
        anyhow::bail!("this app's markup is a directory of files — the App Builder only supports single-file apps right now");
    }
    tokio::fs::read_to_string(&entry.markup_path)
        .await
        .with_context(|| format!("failed to read '{}'", entry.markup_path))
}

/// GET /:workspace/:app/admin/entities-list — every entity's name,
/// field list (name/type/required/default), and query/collection
/// binding, for the App Builder's "Data Model" panel.
async fn admin_entities_list(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<serde_json::Value> = async {
        let markup_text = read_markup_text(&entry).await?;
        let app_def = markup::parse_app(&markup_text)?;
        let entities: Vec<serde_json::Value> = app_def
            .entities
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "source_query": e.source_query,
                    "source_collection": e.source_collection,
                    "fields": e.fields.iter().map(|f| json!({
                        "name": f.name,
                        "type": f.ty.as_str(),
                        "required": f.required,
                        "default": f.default,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"entities": entities, "queries": app_def.queries.iter().map(|q| q.name.clone()).collect::<Vec<_>>()}))
    }
    .await;

    match result {
        Ok(v) => Ok(axum::Json(json!({"ok": true, "meta": v})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/entities/:name/source — entity `name`'s
/// exact current markup text (see `app_editor::entity_source`).
async fn admin_entity_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<String> = async {
        let markup_text = read_markup_text(&entry).await?;
        app_editor::entity_source(&markup_text, &name)
    }
    .await;

    match result {
        Ok(source) => Ok(axum::Json(json!({"ok": true, "source": source})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/entities/add — `source` is the caller's
/// raw markup for exactly one new `entity "..." { ... }` block (see
/// `app_editor::add_entity`), either a plain physical entity or a
/// `from query <name>`/`from collection "..."` one — the App Builder's
/// structured field-list editor just *generates* this text, it isn't
/// constrained to a fixed subset. Validated before writing.
async fn admin_add_entity(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_entity = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_entity.trim().is_empty() {
            anyhow::bail!("an entity needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::add_entity(&markup_text, &new_entity)?;
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

/// POST /:workspace/:app/admin/entities/:name/edit — replaces entity
/// `name`'s whole block outright with `source` (see
/// `app_editor::replace_entity`) — the structured field-list editor's
/// "Save": add/remove/reorder/retype fields all just regenerate this
/// one block. A field *removed* here is only ever dropped from
/// `pgapp_meta` at the next sync, never from the physical table (see
/// `meta::sync_app`'s own "pgapp adds columns but never changes or
/// drops them" rule); a field whose *type* changed against an
/// already-existing column fails at sync time with a clear mismatch
/// error, same validation every other markup edit already gets.
async fn admin_edit_entity(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_entity = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_entity.trim().is_empty() {
            anyhow::bail!("an entity needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::replace_entity(&markup_text, &name, &new_entity)?;
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

/// POST /:workspace/:app/admin/entities/:name/delete — removes entity
/// `name`'s whole block (see `app_editor::delete_entity`); its physical
/// table, if it has one, is deliberately left in place — see
/// `meta::sync_app`'s own entity-cleanup pass for why.
async fn admin_delete_entity(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;

    let result: anyhow::Result<()> = async {
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::delete_entity(&markup_text, &name)?;
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
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("deleted, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/queries-list — every app-level named
/// query's name and SQL text, for the App Builder's "Queries" panel.
async fn admin_queries_list(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<serde_json::Value> = async {
        let markup_text = read_markup_text(&entry).await?;
        let app_def = markup::parse_app(&markup_text)?;
        let queries: Vec<serde_json::Value> =
            app_def.queries.iter().map(|q| json!({"name": q.name, "sql": q.sql})).collect();
        Ok(json!({"queries": queries}))
    }
    .await;

    match result {
        Ok(v) => Ok(axum::Json(json!({"ok": true, "meta": v})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/queries/:name/source — query `name`'s
/// exact current markup text.
async fn admin_query_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<String> = async {
        let markup_text = read_markup_text(&entry).await?;
        app_editor::query_source(&markup_text, &name)
    }
    .await;

    match result {
        Ok(source) => Ok(axum::Json(json!({"ok": true, "source": source})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/queries/add — `source` is the caller's
/// raw markup for exactly one new `query <name> { sql: "..." }` block.
async fn admin_add_query(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_query = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_query.trim().is_empty() {
            anyhow::bail!("a query needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::add_query(&markup_text, &new_query)?;
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

/// POST /:workspace/:app/admin/queries/:name/edit — replaces query
/// `name`'s whole block outright with `source`.
async fn admin_edit_query(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_query = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_query.trim().is_empty() {
            anyhow::bail!("a query needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::replace_query(&markup_text, &name, &new_query)?;
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

/// POST /:workspace/:app/admin/queries/:name/delete — removes query
/// `name`'s whole block. If anything still references it, the write
/// is rejected by `validate_markup`'s own parse only when the markup
/// itself becomes malformed; a *dangling reference* (an entity `from
/// query`, a report/chart/region bound to it) instead surfaces at the
/// next sync's own validation, same as deleting a still-referenced
/// page or entity would.
async fn admin_delete_query(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, name)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;

    let result: anyhow::Result<()> = async {
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::delete_query(&markup_text, &name)?;
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
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("deleted, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/nav-list — every top-level nav item
/// (label, target page if a plain link, or a `submenu: true` flag if
/// it's a nested group — a submenu's own children aren't individually
/// listed here, same "opaque chunk" treatment as everywhere else in
/// `app_editor`'s nav functions), for the App Builder's "Navigation"
/// panel.
async fn admin_nav_list(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<serde_json::Value> = async {
        let markup_text = read_markup_text(&entry).await?;
        let app_def = markup::parse_app(&markup_text)?;
        let items: Vec<serde_json::Value> = app_def
            .nav
            .iter()
            .map(|item| {
                json!({
                    "label": item.label,
                    "target_page": item.target_page,
                    "submenu": !item.children.is_empty(),
                })
            })
            .collect();
        let page_names: Vec<&str> = app_def.pages.iter().map(|p| p.name.as_str()).collect();
        Ok(json!({"items": items, "pages": page_names}))
    }
    .await;

    match result {
        Ok(v) => Ok(axum::Json(json!({"ok": true, "meta": v})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/nav/:idx/source — top-level nav item
/// `idx`'s exact current markup text (a submenu's nested children come
/// along as part of this same chunk — see `app_editor::nav_item_source`).
async fn admin_nav_item_source(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, idx)): Path<(String, String, usize)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<String> = async {
        let markup_text = read_markup_text(&entry).await?;
        app_editor::nav_item_source(&markup_text, idx)
    }
    .await;

    match result {
        Ok(source) => Ok(axum::Json(json!({"ok": true, "source": source})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/nav/add — `source` is the caller's raw
/// markup for exactly one new top-level `item "..." -> page <Name>`
/// (or a submenu block) — creates the app's `nav { }` block itself if
/// it doesn't have one yet (see `app_editor::add_nav_item`).
async fn admin_add_nav_item(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_item = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_item.trim().is_empty() {
            anyhow::bail!("a nav item needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::add_nav_item(&markup_text, &new_item)?;
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

/// POST /:workspace/:app/admin/nav/:idx/edit — replaces top-level nav
/// item `idx` outright with `source`.
async fn admin_edit_nav_item(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, idx)): Path<(String, String, usize)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let new_item = values.get("source").map(|s| s.as_str()).unwrap_or("").to_string();

    let result: anyhow::Result<()> = async {
        if new_item.trim().is_empty() {
            anyhow::bail!("a nav item needs some markup text");
        }
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::replace_nav_item(&markup_text, idx, &new_item)?;
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

/// POST /:workspace/:app/admin/nav/:idx/delete — removes top-level nav
/// item `idx` outright.
async fn admin_delete_nav_item(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app, idx)): Path<(String, String, usize)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;

    let result: anyhow::Result<()> = async {
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::delete_nav_item(&markup_text, idx)?;
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

/// POST /:workspace/:app/admin/nav/reorder — `order` is a
/// comma-separated permutation of `0..n` (same shape/semantics as
/// `admin_reorder_page`'s component reorder, applied to the nav's
/// top-level items instead — see `app_editor::reorder_nav_items`).
async fn admin_reorder_nav(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let order_str = values.get("order").map(|s| s.as_str()).unwrap_or("");

    let result: anyhow::Result<()> = async {
        let new_order: Vec<usize> = if order_str.is_empty() {
            Vec::new()
        } else {
            order_str
                .split(',')
                .map(|s| s.trim().parse::<usize>().context("bad index in 'order'"))
                .collect::<anyhow::Result<Vec<usize>>>()?
        };
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::reorder_nav_items(&markup_text, &new_order)?;
        tokio::fs::write(&entry.markup_path, updated)
            .await
            .with_context(|| format!("failed to write '{}'", entry.markup_path))?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => match entry.reload(&state.pool, &state.item_types, &state.actions).await {
            Ok(()) => Ok(axum::Json(json!({"ok": true})).into_response()),
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("reordered, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// GET /:workspace/:app/admin/settings — the app's current
/// theme/icons/chart_lib/auth-enabled, for the App Builder's "App
/// Settings" form.
async fn admin_settings_get(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let result: anyhow::Result<serde_json::Value> = async {
        let markup_text = read_markup_text(&entry).await?;
        let app_def = markup::parse_app(&markup_text)?;
        Ok(json!({
            "theme": app_def.theme.unwrap_or_else(|| "plain".to_string()),
            "icons": app_def.icons.unwrap_or_else(|| "builtin".to_string()),
            "chart_lib": app_def.chart_lib.unwrap_or_else(|| "inline".to_string()),
            "auth_enabled": app_def.auth,
        }))
    }
    .await;

    match result {
        Ok(v) => Ok(axum::Json(json!({"ok": true, "meta": v})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/settings — sets theme/icons/chart_lib
/// and toggles the bare `auth { }` block on or off (see
/// `app_editor::set_app_settings`) — the App Builder's "Edit
/// Application Properties" equivalent. Scoped deliberately: an
/// `auth_scheme`'s own role list, and anything about *which* pages
/// require which role, both stay Advanced-editor-only — this is just
/// the instance-wide theme/icons/chart-library pick plus the
/// authentication on/off switch.
async fn admin_settings_set(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let entry = admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let theme = values.get("theme").map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("plain").to_string();
    let icons = values.get("icons").map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("builtin").to_string();
    let chart_lib = values.get("chart_lib").map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("inline").to_string();
    let auth_enabled = values.get("auth_enabled").map(|s| s == "true" || s == "on").unwrap_or(false);

    let result: anyhow::Result<()> = async {
        let markup_text = read_markup_text(&entry).await?;
        let updated = app_editor::set_app_settings(&markup_text, &theme, &icons, &chart_lib, auth_enabled)?;
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
            Err(e) => Ok(axum::Json(json!({"ok": false, "error": format!("saved, but reload failed: {e:#}")})).into_response()),
        },
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/destroy — the App Builder's "Delete
/// App": `mode` is `soft` (disable — reversible, tables/rows
/// untouched, takes effect on the next `pgapp run`, not this
/// already-running process — same semantics as `pgapp app destroy
/// --soft`) or `hard` (permanent — drops its own data tables, needs
/// `confirm` to equal the app's own slug, mirroring the CLI's
/// type-the-name confirmation). A hard delete also drops it from the
/// live registry immediately (`AppState::unregister_app`) so it starts
/// 404ing right away instead of erroring against now-missing tables.
/// No superuser connection needed either way — see
/// `control::hard_delete_app`'s own doc.
async fn admin_destroy_app(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    let hard = values.get("mode").map(|s| s.as_str()) == Some("hard");
    let confirm = values.get("confirm").map(|s| s.trim()).unwrap_or("");

    let result: anyhow::Result<()> = async {
        let app_row = control::find_app(&state.pool, &app, Some(&workspace))
            .await?
            .ok_or_else(|| anyhow::anyhow!("no app '{app}' registered in workspace '{workspace}'"))?;
        if !hard {
            control::disable(&state.pool, app_row.id).await?;
            return Ok(());
        }
        if confirm != app {
            anyhow::bail!("type '{app}' to confirm permanently dropping its data tables");
        }
        control::hard_delete_app(&state.pool, &app_row).await?;
        state.unregister_app(&format!("{workspace}/{app}"));
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(axum::Json(json!({"ok": true, "hard": hard})).into_response()),
        Err(e) => Ok(axum::Json(json!({"ok": false, "error": e.to_string()})).into_response()),
    }
}

/// POST /:workspace/:app/admin/destroy-workspace — the App Builder's
/// "Delete Workspace": `mode` is `soft` (disable, reversible) or `hard`
/// (permanent — drops the schema/role, needs `confirm` to equal the
/// workspace's own slug plus a superuser-capable `grantor_conn`, used
/// once and never persisted — see `control::hard_delete_workspace`'s
/// own doc, same contract as "New Workspace"'s existing-schema attach).
/// `:app` names whichever app is currently on-screen when this is
/// clicked (the App Builder's own `AppSettings` page) — not because the
/// *operation* is scoped to that app (it tears down the whole
/// workspace, every app in it included), but because the global
/// `auth::require_login` middleware resolves every request's auth
/// context from exactly this `/{workspace}/{app}/...` shape before any
/// route handler runs, so a workspace-wide route still needs *some*
/// registered app in the URL to borrow that context from — same
/// "everyone's an admin" fallback as every other admin route here.
/// `admin_edit_guard` both runs that check and refuses outright if
/// `:app` is the App Builder itself; the explicit workspace-slug check
/// below is belt-and-suspenders against tearing down the App Builder's
/// *own* reserved workspace by any other app slug that might ever end
/// up registered into it.
async fn admin_destroy_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((workspace, app)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    admin_edit_guard(&state, &auth_ctx, &workspace, &app)?;
    if workspace == instance::APP_BUILDER_WORKSPACE_SLUG {
        return Err((StatusCode::FORBIDDEN, "the App Builder's own workspace can't be torn down".to_string()));
    }
    let hard = values.get("mode").map(|s| s.as_str()) == Some("hard");
    let confirm = values.get("confirm").map(|s| s.trim()).unwrap_or("");
    let grantor_conn = values.get("grantor_conn").map(|s| s.as_str()).unwrap_or("");

    let result: anyhow::Result<()> = async {
        let ws = control::find_workspace(&state.pool, &workspace)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no workspace '{workspace}' registered"))?;
        if !hard {
            control::disable_workspace(&state.pool, &workspace).await?;
            return Ok(());
        }
        if confirm != workspace {
            anyhow::bail!("type '{workspace}' to confirm permanently destroying this workspace");
        }
        if grantor_conn.is_empty() {
            anyhow::bail!("a superuser-capable connection string is required to drop the schema/role");
        }
        let apps = control::list_all(&state.pool).await?;
        control::hard_delete_workspace(&state.pool, &ws, grantor_conn).await?;
        for a in apps.iter().filter(|a| a.workspace_slug.as_deref() == Some(workspace.as_str())) {
            state.unregister_app(&format!("{workspace}/{}", a.slug));
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(axum::Json(json!({"ok": true, "hard": hard})).into_response()),
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
