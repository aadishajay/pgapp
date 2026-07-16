use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::meta::{RuntimeApp, RuntimeEntity, RuntimePage};
use crate::model::{FieldItemType, PageKind};
use crate::render;
use crate::theme::Theme;

pub struct AppState {
    pub pool: PgPool,
    pub app: RuntimeApp,
    pub theme: Theme,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/theme.css", get(theme_css))
        .route("/assets/*path", get(asset))
        .route("/api/:entity", get(api_list))
        .route("/:page", get(show).post(create))
        .route("/:page/:id/edit", get(edit_form))
        .route("/:page/:id/update", post(update))
        .route("/:page/:id/delete", post(delete))
        .with_state(state)
}

type AppError = (StatusCode, String);

fn err_response(e: anyhow::Error) -> AppError {
    (StatusCode::BAD_REQUEST, e.to_string())
}

/// `list`/`detail` pages always have an entity by construction (the
/// parser requires `of <entity>` for both kinds); this just turns that
/// invariant into a normal error instead of a panic if it's ever wrong.
fn entity_of(page: &RuntimePage) -> Result<&RuntimeEntity, AppError> {
    page.entity.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("page '{}' has no backing entity", page.name),
        )
    })
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

async fn fetch_rows(
    pool: &PgPool,
    entity: &RuntimeEntity,
) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    let sql = format!(
        "select {} from pgapp_data.{} order by id",
        select_columns(entity),
        entity.table_name
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::new();
    for row in rows {
        let mut map = BTreeMap::new();
        map.insert("id".to_string(), row.try_get::<Option<String>, _>("id")?);
        for f in &entity.fields {
            if f.name == "id" {
                continue;
            }
            map.insert(f.name.clone(), row.try_get::<Option<String>, _>(f.name.as_str())?);
        }
        out.push(map);
    }
    Ok(out)
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
    Ok(match row {
        None => None,
        Some(row) => {
            let mut map = BTreeMap::new();
            map.insert("id".to_string(), row.try_get::<Option<String>, _>("id")?);
            for f in &entity.fields {
                if f.name == "id" {
                    continue;
                }
                map.insert(f.name.clone(), row.try_get::<Option<String>, _>(f.name.as_str())?);
            }
            Some(map)
        }
    })
}

/// Builds (column names, value expressions, bind values) for a page's
/// form fields. Empty, non-required values become SQL `NULL` literals
/// directly (an empty string can't be cast to e.g. integer); everything
/// else is bound as text and cast in SQL, since the actual Postgres
/// column type isn't known at compile time.
///
/// Unchecked HTML checkboxes never submit their key at all, so a
/// `Checkbox` field reads "true"/"false" from whether `name` is present
/// in `values`, not from the (usually absent) value itself.
fn build_value_exprs(
    page: &RuntimePage,
    form_fields: &[String],
    values: &HashMap<String, String>,
) -> anyhow::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let entity = page.entity.as_ref().expect("list page always has an entity");
    let mut columns = Vec::new();
    let mut exprs = Vec::new();
    let mut binds = Vec::new();

    for name in form_fields {
        let field = entity
            .field(name)
            .ok_or_else(|| anyhow::anyhow!("unknown field '{name}'"))?;
        let item_type = page.item_types.get(name).unwrap_or(&FieldItemType::Text);

        let raw = if matches!(item_type, FieldItemType::Checkbox) {
            if values.contains_key(name) { "true" } else { "false" }.to_string()
        } else {
            values.get(name).map(|s| s.trim().to_string()).unwrap_or_default()
        };

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

fn page_or_404<'a>(app: &'a RuntimeApp, name: &str) -> Result<&'a RuntimePage, AppError> {
    app.page(name)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such page '{name}'")))
}

fn require_list_page<'a>(page: &'a RuntimePage) -> Result<&'a RuntimePage, AppError> {
    if page.kind != PageKind::List {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("page '{}' does not support this operation", page.name),
        ));
    }
    Ok(page)
}

async fn index(State(state): State<Arc<AppState>>) -> Html<String> {
    let pages: Vec<String> = state.app.pages.iter().map(|p| p.name.clone()).collect();
    Html(render::index_page(&state.app.name, &pages, state.app.chrome()))
}

async fn theme_css(State(state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read(&state.theme.css_path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/css")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
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

/// Serves all three page kinds behind `GET /:page`: a CRUD list, a
/// single-row read-only detail (via `?id=`), or a pure page-items page.
async fn show(
    State(state): State<Arc<AppState>>,
    Path(page_name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let page = page_or_404(&state.app, &page_name)?;
    match page.kind {
        PageKind::List => {
            let entity = entity_of(page)?;
            let rows = fetch_rows(&state.pool, entity).await.map_err(err_response)?;
            Ok(Html(render::list_page(page, &rows, None, state.app.chrome())))
        }
        PageKind::Detail => {
            let entity = entity_of(page)?;
            let id = query.get("id").ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "missing '?id=' query parameter".to_string(),
                )
            })?;
            let row = fetch_row(&state.pool, entity, id)
                .await
                .map_err(err_response)?
                .ok_or_else(|| (StatusCode::NOT_FOUND, "row not found".to_string()))?;
            Ok(Html(render::detail_page(page, &row, state.app.chrome())))
        }
        PageKind::Static => Ok(Html(render::static_page(page, state.app.chrome()))),
    }
}

async fn create(
    State(state): State<Arc<AppState>>,
    Path(page_name): Path<String>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let page = require_list_page(page_or_404(&state.app, &page_name)?)?;
    let entity = entity_of(page)?;

    let build = build_value_exprs(page, &page.form, &values);
    let (columns, exprs, binds) = match build {
        Ok(v) => v,
        Err(e) => {
            let rows = fetch_rows(&state.pool, entity).await.map_err(err_response)?;
            return Ok(Html(render::list_page(
                page,
                &rows,
                Some(&e.to_string()),
                state.app.chrome(),
            ))
            .into_response());
        }
    };

    let sql = format!(
        "insert into pgapp_data.{} ({}) values ({})",
        entity.table_name,
        columns.join(", "),
        exprs.join(", ")
    );
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query.execute(&state.pool).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("failed to create row: {e}"),
        )
    })?;

    Ok(Redirect::to(&format!("/{page_name}")).into_response())
}

async fn edit_form(
    State(state): State<Arc<AppState>>,
    Path((page_name, id)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    let page = require_list_page(page_or_404(&state.app, &page_name)?)?;
    let entity = entity_of(page)?;
    let row = fetch_row(&state.pool, entity, &id)
        .await
        .map_err(err_response)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "row not found".to_string()))?;
    Ok(Html(render::edit_page(page, &id, &row, None, state.app.chrome())))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path((page_name, id)): Path<(String, String)>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let page = require_list_page(page_or_404(&state.app, &page_name)?)?;
    let entity = entity_of(page)?;

    let build = build_value_exprs(page, &page.form, &values);
    let (columns, exprs, mut binds) = match build {
        Ok(v) => v,
        Err(e) => {
            let row = fetch_row(&state.pool, entity, &id)
                .await
                .map_err(err_response)?
                .unwrap_or_default();
            return Ok(Html(render::edit_page(
                page,
                &id,
                &row,
                Some(&e.to_string()),
                state.app.chrome(),
            ))
            .into_response());
        }
    };

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
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query.execute(&state.pool).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("failed to update row: {e}"),
        )
    })?;

    Ok(Redirect::to(&format!("/{page_name}")).into_response())
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Path((page_name, id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let page = require_list_page(page_or_404(&state.app, &page_name)?)?;
    let entity = entity_of(page)?;
    let sql = format!(
        "delete from pgapp_data.{} where id = $1::integer",
        entity.table_name
    );
    sqlx::query(&sql)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to delete row: {e}")))?;
    Ok(Redirect::to(&format!("/{page_name}")).into_response())
}

/// Minimal JSON API, keyed by entity rather than page — a stand-in for
/// the REST routing PostgREST would otherwise provide.
async fn api_list(
    State(state): State<Arc<AppState>>,
    Path(entity_name): Path<String>,
) -> Result<Response, AppError> {
    let page = state
        .app
        .pages
        .iter()
        .find(|p| p.entity.as_ref().is_some_and(|e| e.name == entity_name))
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("no such entity '{entity_name}'")))?;
    let entity = entity_of(page)?;
    let rows = fetch_rows(&state.pool, entity).await.map_err(err_response)?;
    Ok(axum::Json(json!(rows)).into_response())
}
