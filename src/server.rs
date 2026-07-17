//! Route handlers and the generic entity CRUD they're built on. Named
//! query execution and everything that depends on it (LOV choices,
//! regions, paginated query rows) lives in `server::query_engine`,
//! which this module just calls into.
//!
//! A page is an ordered list of components, rendered top to bottom by
//! `show` (`GET /:page`). `Form` and `EditableTable` are the only
//! *writable* component kinds; both are addressed by their index on the
//! page (`/:page/c/:idx/...`) since a page may have more than one. A
//! `Report`'s row actions (Edit/Delete) only appear when the same page
//! also has a `Form` bound to the same entity — `sibling_form_idx`
//! finds it by scanning the page's own components, no extra metadata
//! needed.

pub mod auth;
mod query_engine;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde_json::json;
use sqlx::{PgPool, Row};

use auth::AuthCtx;

use crate::chart_lib::ChartLib;
use crate::html::url_encode;
use crate::icons::Icons;
use crate::item_types;
use crate::meta::{RegionRows, RuntimeApp, RuntimeComponent, RuntimeEntity, RuntimePage};
use crate::model::FieldItem;
use crate::render;
use crate::theme::Theme;
use query_engine::{bind_context, resolve_field_choices, resolve_regions, run_named_query_page, run_named_query_rows};

pub struct AppState {
    pub pool: PgPool,
    pub app: RuntimeApp,
    pub theme: Theme,
    pub runtime_js: String,
    pub item_types: item_types::Registry,
    pub icons: Icons,
    pub chart_lib: ChartLib,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/theme.css", get(theme_css))
        .route("/runtime.js", get(runtime_js))
        .route("/chart-lib.js", get(chart_lib_js))
        .route("/assets/*path", get(asset))
        .route("/api/:entity", get(api_list))
        .route("/login", get(auth::login_form).post(auth::login))
        .route("/setup", post(auth::setup))
        .route("/logout", post(auth::logout))
        .route("/users", get(auth::users_page).post(auth::users_create))
        .route("/users/:id/delete", post(auth::users_delete))
        .route("/:page", get(show))
        .route("/:page/c/:idx/create", post(create))
        .route("/:page/c/:idx/update/:id", post(update))
        .route("/:page/c/:idx/delete/:id", post(delete))
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_login))
        .with_state(state)
}

type AppError = (StatusCode, String);

fn err_response(e: anyhow::Error) -> AppError {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn page_or_404<'a>(app: &'a RuntimeApp, name: &str) -> Result<&'a RuntimePage, AppError> {
    app.page(name).ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such page '{name}'")))
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

async fn fetch_rows(pool: &PgPool, entity: &RuntimeEntity) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    // `order by t.id`, qualified: the select list aliases `id::text as
    // id`, and an unqualified ORDER BY id would bind to that *text*
    // output column, sorting "10" before "2".
    let sql = format!("select {} from pgapp_data.{} t order by t.id", select_columns(entity), entity.table_name);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(|r| row_from_sqlx(r, entity)).collect()
}

async fn fetch_row(
    pool: &PgPool,
    entity: &RuntimeEntity,
    id: &str,
) -> anyhow::Result<Option<BTreeMap<String, Option<String>>>> {
    let sql = format!(
        "select {} from pgapp_data.{} where id = $1::integer",
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

/// Keyset ("seek") pagination for an entity-backed `Report`: `after`/
/// `before` cursor on `id`, fetching `page_size + 1` rows in the
/// query's own direction. Zero extra queries: the direction we fetched
/// tells us whether *it* has more (the extra row); the direction we
/// arrived *from* always has more, because reaching this page via a
/// cursor implies a page on the other side of it. `COUNT(*)`/`OFFSET`
/// never enter into it, so this stays cheap no matter how large the
/// table gets.
async fn fetch_report_rows(
    pool: &PgPool,
    entity: &RuntimeEntity,
    page_size: i64,
    after: Option<&str>,
    before: Option<&str>,
) -> anyhow::Result<ReportPage> {
    let cols = select_columns(entity);
    let lim = page_size + 1;
    // ORDER BY is qualified (`t.id`) for the same reason as in
    // `fetch_rows`: the select list re-exports `id` as text, and the
    // cursor comparison below is numeric — mixing the two orderings
    // would make pages skip/repeat rows.
    let (sql, bind, reverse) = if let Some(after) = after {
        (
            format!(
                "select {cols} from pgapp_data.{} t where t.id > $1::integer order by t.id asc limit {lim}",
                entity.table_name
            ),
            Some(after),
            false,
        )
    } else if let Some(before) = before {
        (
            format!(
                "select {cols} from pgapp_data.{} t where t.id < $1::integer order by t.id desc limit {lim}",
                entity.table_name
            ),
            Some(before),
            true,
        )
    } else {
        (
            format!("select {cols} from pgapp_data.{} t order by t.id asc limit {lim}", entity.table_name),
            None,
            false,
        )
    };

    let query = sqlx::query(&sql);
    let query = match bind {
        Some(b) => query.bind(b),
        None => query,
    };
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

async fn index(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
) -> Result<Html<String>, AppError> {
    let pages: Vec<String> = state.app.pages.iter().map(|p| p.name.clone()).collect();
    let ctx = HashMap::new();
    let regions = resolve_regions(&state.pool, &state.app, None, &ctx).await.map_err(err_response)?;
    Ok(Html(render::index_page(
        &state.app.name,
        &pages,
        state.app.chrome(&regions),
        &state.icons,
        &state.chart_lib,
        auth_ctx.display(),
    )))
}

async fn theme_css(State(state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read(&state.theme.css_path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/css")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The pgapp runtime JS library — stored in `pgapp_meta`, not a static
/// file (see `AppState::runtime_js` / `main.rs`), so it's part of the
/// same in-database metadata as everything else.
async fn runtime_js(State(state): State<Arc<AppState>>) -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        state.runtime_js.clone(),
    )
        .into_response()
}

/// The active pluggable chart library's JS, if one is configured
/// (`PGAPP_CHART_LIB` other than the built-in "inline" backend, which
/// needs no JS at all — see `src/chart_lib.rs`).
async fn chart_lib_js(State(state): State<Arc<AppState>>) -> Response {
    match &state.chart_lib.js_path {
        Some(path) => match tokio::fs::read(path).await {
            Ok(bytes) => ([(header::CONTENT_TYPE, "application/javascript")], bytes).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn asset(Path(path): Path<String>) -> Response {
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

/// Renders one component into its HTML body, fetching whatever data it
/// needs along the way.
async fn render_component(
    state: &AppState,
    page_name: &str,
    page: &RuntimePage,
    idx: usize,
    component: &RuntimeComponent,
    query: &HashMap<String, String>,
    regions: &RegionRows,
) -> anyhow::Result<String> {
    match component {
        RuntimeComponent::Text(text) => Ok(render::text_html(text)),
        RuntimeComponent::Link { label, target_page } => Ok(render::link_html(label, target_page)),
        RuntimeComponent::Region { label, query: qname } => Ok(render::region_html(label, qname, regions)),

        RuntimeComponent::Chart { title, query: qname, chart_type, x, y } => {
            let rq = page
                .resolve_query(&state.app, qname)
                .ok_or_else(|| anyhow::anyhow!("chart '{title}' references unknown query '{qname}'"))?;
            let ctx = bind_context(query, None);
            let rows = run_named_query_rows(&state.pool, rq, &ctx).await?;
            Ok(render::chart_html(title, chart_type, x, y, &rows, &state.chart_lib))
        }

        RuntimeComponent::Report { title, entity, columns, source_query, link_column, page_size } => {
            let form_idx = sibling_form_idx(page, &entity.name);
            let p_after = format!("r{idx}_after");
            let p_before = format!("r{idx}_before");
            let p_page = format!("r{idx}_page");

            let (rows, prev_href, next_href) = match source_query {
                None => {
                    let after = query.get(&p_after).map(|s| s.as_str());
                    let before = query.get(&p_before).map(|s| s.as_str());
                    let rp = fetch_report_rows(&state.pool, entity, *page_size, after, before).await?;
                    let prev_href = rp.has_prev.then(|| {
                        let id = rp.rows.first().and_then(|r| r.get("id")).and_then(|v| v.clone()).unwrap_or_default();
                        format!("/{page_name}?{p_before}={}", url_encode(&id))
                    });
                    let next_href = rp.has_next.then(|| {
                        let id = rp.rows.last().and_then(|r| r.get("id")).and_then(|v| v.clone()).unwrap_or_default();
                        format!("/{page_name}?{p_after}={}", url_encode(&id))
                    });
                    (rp.rows, prev_href, next_href)
                }
                Some(qname) => {
                    let rq = page
                        .resolve_query(&state.app, qname)
                        .ok_or_else(|| anyhow::anyhow!("report '{title}' sources from unknown query '{qname}'"))?;
                    let ctx = bind_context(query, None);
                    let page_num: i64 = query.get(&p_page).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
                    let (json_rows, has_next) = run_named_query_page(&state.pool, rq, &ctx, *page_size, page_num).await?;
                    let rows: Vec<_> = json_rows.into_iter().map(query_engine::json_row_to_map).collect();
                    let prev_href = (page_num > 1).then(|| format!("/{page_name}?{p_page}={}", page_num - 1));
                    let next_href = has_next.then(|| format!("/{page_name}?{p_page}={}", page_num + 1));
                    (rows, prev_href, next_href)
                }
            };

            Ok(render::report_html(
                page_name,
                title,
                columns,
                &rows,
                link_column.as_ref(),
                prev_href.as_deref(),
                next_href.as_deref(),
                form_idx,
                &state.icons,
            ))
        }

        RuntimeComponent::Form { title, entity, fields, item_types } => {
            let edit_param = format!("edit_{idx}");
            match query.get(&edit_param) {
                Some(id) => {
                    let row = fetch_row(&state.pool, entity, id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("row '{id}' not found"))?;
                    let ctx = bind_context(query, Some(&row));
                    let choices = resolve_field_choices(&state.pool, &state.app, page, item_types, &ctx).await?;
                    Ok(render::form_html(page_name, idx, title, fields, entity, &row, Some(id), &choices, item_types, &state.item_types))
                }
                None => {
                    let ctx = bind_context(query, None);
                    let choices = resolve_field_choices(&state.pool, &state.app, page, item_types, &ctx).await?;
                    let empty = BTreeMap::new();
                    Ok(render::form_html(page_name, idx, title, fields, entity, &empty, None, &choices, item_types, &state.item_types))
                }
            }
        }

        RuntimeComponent::EditableTable { title, entity, columns, item_types } => {
            let ctx = bind_context(query, None);
            let choices = resolve_field_choices(&state.pool, &state.app, page, item_types, &ctx).await?;
            let rows = fetch_rows(&state.pool, entity).await?;
            Ok(render::editable_table_html(
                page_name,
                idx,
                title,
                columns,
                entity,
                &rows,
                &choices,
                item_types,
                &state.item_types,
                &state.icons,
            ))
        }
    }
}

async fn show(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path(page_name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let page = page_or_404(&state.app, &page_name)?;
    auth::authorize(&state, page.required_role.as_deref(), &auth_ctx)?;
    let ctx = bind_context(&query, None);
    let regions = resolve_regions(&state.pool, &state.app, Some(page), &ctx)
        .await
        .map_err(err_response)?;

    let mut body = String::new();
    for (idx, component) in page.components.iter().enumerate() {
        body.push_str(
            &render_component(&state, &page_name, page, idx, component, &query, &regions)
                .await
                .map_err(err_response)?,
        );
    }

    Ok(Html(render::page_layout(
        &page.name,
        &body,
        query.get("error").map(|s| s.as_str()),
        state.app.chrome(&regions),
        &state.icons,
        &state.chart_lib,
        auth_ctx.display(),
    )))
}

async fn create(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((page_name, idx)): Path<(String, usize)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let page = page_or_404(&state.app, &page_name)?;
    auth::authorize(&state, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let (entity, fields, item_types) = writable_fields(component, &page_name, idx)?;

    match build_value_exprs(entity, fields, item_types, &values, &state.item_types) {
        Ok((columns, exprs, binds)) => {
            let sql = format!(
                "insert into pgapp_data.{} ({}) values ({})",
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
            Ok(Redirect::to(&format!("/{page_name}")).into_response())
        }
        Err(e) => Ok(Redirect::to(&format!("/{page_name}?error={}", url_encode(&e.to_string()))).into_response()),
    }
}

async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((page_name, idx, id)): Path<(String, usize, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let page = page_or_404(&state.app, &page_name)?;
    auth::authorize(&state, page.required_role.as_deref(), &auth_ctx)?;
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
                "update pgapp_data.{} set {set_clause} where id = ${where_placeholder}::integer",
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
            Ok(Redirect::to(&format!("/{page_name}")).into_response())
        }
        Err(e) => {
            // A Form component re-enters edit mode on error (so the
            // user doesn't lose their place); an EditableTable has no
            // separate edit mode to return to.
            let extra = match component {
                RuntimeComponent::Form { .. } => format!("&edit_{idx}={}", url_encode(&id)),
                _ => String::new(),
            };
            Ok(Redirect::to(&format!("/{page_name}?error={}{extra}", url_encode(&e.to_string()))).into_response())
        }
    }
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(auth_ctx): Extension<AuthCtx>,
    Path((page_name, idx, id)): Path<(String, usize, String)>,
) -> Result<Response, AppError> {
    let page = page_or_404(&state.app, &page_name)?;
    auth::authorize(&state, page.required_role.as_deref(), &auth_ctx)?;
    let component = component_at(page, idx)?;
    let (entity, _, _) = writable_fields(component, &page_name, idx)?;

    let sql = format!("delete from pgapp_data.{} where id = $1::integer", entity.table_name);
    sqlx::query(&sql)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to delete row: {e}")))?;
    Ok(Redirect::to(&format!("/{page_name}")).into_response())
}

/// Minimal JSON API, keyed by entity rather than page — a stand-in for
/// the REST routing PostgREST would otherwise provide. Looks for the
/// entity on any Report/Form/EditableTable component across every page.
async fn api_list(
    State(state): State<Arc<AppState>>,
    Path(entity_name): Path<String>,
) -> Result<Response, AppError> {
    let entity = state
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
    let rows = fetch_rows(&state.pool, entity).await.map_err(err_response)?;
    Ok(axum::Json(json!(rows)).into_response())
}
