//! Reloads a [`RuntimeApp`](super::RuntimeApp) straight from
//! `pgapp_meta`, proving the database (not the parsed markup struct) is
//! the authority once the server starts handling requests.

use anyhow::{Context, Result};
use sqlx::{Executor, PgPool, TypeInfo};
use std::collections::HashMap;

use super::types::{
    wrap_to_jsonb, ButtonBehavior, LinkColumn, NavNode, RuntimeApp, RuntimeComponent, RuntimeEntity,
    RuntimeField, RuntimePage, RuntimeQuery,
};
use crate::model::{AggregateFn, ComputedColumn, FieldItem, FieldType, FormatMask, HtmlAttrs, PreAction};

/// One piece of a named query's SQL text, as split by `tokenize_binds`:
/// either literal SQL or a `:name` bind marker.
#[derive(Debug, PartialEq)]
enum Segment {
    Text(String),
    Bind(String),
}

/// Splits `sql` into literal-text/bind-marker segments, plus the
/// distinct bind names in first-occurrence order (a name repeated
/// later in the query reuses the same position — see
/// `compile_named_query`). A literal `::` (Postgres's own cast
/// operator) is left untouched, so a hand-written cast is never
/// mistaken for a bind marker.
fn tokenize_binds(sql: &str) -> (Vec<Segment>, Vec<String>) {
    let chars: Vec<char> = sql.chars().collect();
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' && chars.get(i + 1) == Some(&':') {
            literal.push_str("::");
            i += 2;
        } else if c == ':' && chars.get(i + 1).is_some_and(|c| c.is_alphabetic() || *c == '_') {
            if !literal.is_empty() {
                segments.push(Segment::Text(std::mem::take(&mut literal)));
            }
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            if !names.contains(&name) {
                names.push(name.clone());
            }
            segments.push(Segment::Bind(name));
            i = j;
        } else {
            literal.push(c);
            i += 1;
        }
    }
    if !literal.is_empty() {
        segments.push(Segment::Text(literal));
    }
    (segments, names)
}

/// Reassembles `segments` into SQL text, turning each `Bind(name)` into
/// its positional parameter — bare `$N` when `casts` is `None`, or
/// `$N::TYPE` when it's the resolved type for that position (see
/// `compile_named_query`).
fn render_segments(segments: &[Segment], names: &[String], casts: Option<&[String]>) -> String {
    let mut out = String::new();
    for seg in segments {
        match seg {
            Segment::Text(t) => out.push_str(t),
            Segment::Bind(name) => {
                // Always found: every Bind segment's name was pushed
                // into `names` by the same tokenize_binds call.
                let idx = names.iter().position(|n| n == name).unwrap();
                match casts {
                    Some(casts) => out.push_str(&format!("${}::{}", idx + 1, casts[idx])),
                    None => out.push_str(&format!("${}", idx + 1)),
                }
            }
        }
    }
    out
}

/// Turns `sql` (which may contain `:name` bind markers) into Postgres
/// positional-parameter SQL plus the ordered list of names each `$N`
/// stands for — APEX-style bind items: the query author never writes
/// a cast, because the bind's type isn't guessed or hand-declared, it's
/// asked directly from Postgres.
///
/// `:project_id` in `where project_id = :project_id` becomes `$1::INT4`
/// (or whatever `project_id`'s real column type is) automatically: this
/// runs the exact wrapped shape the query is later executed in (see
/// `wrap_to_jsonb`) through Postgres's own `Describe`, the same
/// mechanism the wire protocol already uses to type an unadorned `$1`
/// — so the result is never a guess and never goes stale. It's asked
/// fresh every time the app loads (startup, or `/admin/reload`), so a
/// column changed from `integer` to `bigint` under a query's feet is
/// picked up on the next reload with no markup change, instead of
/// silently miscomparing until some request happens to hit it.
///
/// A bind whose type genuinely can't be inferred (compared against
/// nothing, or against two incompatible columns) makes this fail with
/// Postgres's own error — at load time, not at first request — and a
/// hand-written cast (`:project_id::integer`, the old style) still
/// works exactly as before: it's just a redundant no-op layered under
/// the auto-detected one.
pub async fn compile_named_query(pool: &PgPool, data_schema: &str, sql: &str) -> Result<(String, Vec<String>)> {
    let (segments, names) = tokenize_binds(sql);
    if names.is_empty() {
        return Ok((sql.to_string(), names));
    }

    let bare = render_segments(&segments, &names, None);
    // search_path-scoped (see meta::scoped_conn): an unqualified table
    // reference in the query's own SQL only resolves to this app's
    // tables if the connection Describe runs on has this set — the
    // classic `pool.describe(...)` call used the pool's own default
    // search_path, which knows nothing about any particular app.
    let mut conn = crate::meta::scoped_conn(pool, data_schema).await?;
    let described = (&mut *conn)
        .describe(&wrap_to_jsonb(&bare))
        .await
        .with_context(|| {
            format!(
                "couldn't infer bind parameter types for named query SQL: {sql}\n\
                 (Postgres couldn't tell what type one of the `:name` binds should be — \
                 add an explicit cast, e.g. `:{}::text`, to disambiguate it)",
                names[0]
            )
        })?;

    let casts: Vec<String> = match described.parameters() {
        Some(sqlx::Either::Left(params)) if params.len() == names.len() => {
            params.iter().map(|p| p.name().to_string()).collect()
        }
        // Postgres always returns concrete types for every parameter
        // position on the Postgres driver; this is just a safe
        // fallback to the old behavior if that ever isn't true.
        _ => names.iter().map(|_| "text".to_string()).collect(),
    };

    Ok((render_segments(&segments, &names, Some(&casts)), names))
}

/// Loads the current `runtime.js` content for `app_name` — whatever's in
/// `pgapp_meta.app_runtime_js` now, not necessarily the built-in default
/// (the row is only seeded once; edits to it afterward stick).
pub async fn load_runtime_js(pool: &PgPool, app_name: &str) -> Result<String> {
    sqlx::query_scalar(
        "select r.content from pgapp_meta.app_runtime_js r
           join pgapp_meta.apps a on a.id = r.app_id
          where a.name = $1",
    )
    .bind(app_name)
    .fetch_one(pool)
    .await
    .with_context(|| format!("runtime.js not found for app '{app_name}'"))
}

pub async fn load_app(pool: &PgPool, app_name: &str) -> Result<RuntimeApp> {
    let (app_id, data_schema, theme, icons, chart_lib, auth_enabled): (
        i32,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        "select id, data_schema, theme, icons, chart_lib, auth_enabled from pgapp_meta.apps where name = $1",
    )
    .bind(app_name)
    .fetch_one(pool)
    .await
    .with_context(|| format!("app '{app_name}' not found in pgapp_meta"))?;

    let entities = load_entities(pool, app_id).await?;

    let page_rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "select id, name, required_role from pgapp_meta.pages where app_id = $1 order by id",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;
    let page_names: HashMap<i32, String> =
        page_rows.iter().map(|(id, name, _)| (*id, name.clone())).collect();

    let (app_queries, mut page_queries) = load_queries(pool, app_id, &data_schema).await?;
    let schemes = load_auth_schemes(pool, app_id).await?;

    let mut pages = Vec::new();
    for (page_id, name, required_role) in &page_rows {
        let components = load_components(pool, "page_id", *page_id, &entities).await?;
        pages.push(RuntimePage {
            name: name.clone(),
            components,
            queries: page_queries.remove(page_id).unwrap_or_default(),
            required_role: required_role.clone(),
        });
    }

    let nav_rows: Vec<(i32, Option<i32>, String, Option<i32>)> = sqlx::query_as(
        "select id, parent_id, label, target_page_id
           from pgapp_meta.nav_items where app_id = $1 order by ordinal",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;
    let nav = build_nav_tree(&nav_rows, None, &page_names);

    let header = load_chrome(pool, app_id, "header", &entities).await?;
    let footer = load_chrome(pool, app_id, "footer", &entities).await?;

    Ok(RuntimeApp {
        id: app_id,
        name: app_name.to_string(),
        data_schema,
        theme,
        icons,
        chart_lib,
        auth_enabled,
        pages,
        nav,
        header,
        footer,
        queries: app_queries,
        schemes,
        // Neither is known here — pgapp_control isn't this function's
        // concern (see RuntimeApp's doc comment). Whoever calls
        // load_app sets these from the control-plane registry
        // afterward.
        control_app_id: 0,
        workspace_id: None,
    })
}

/// Loads every entity for the app, keyed by name, so components can
/// resolve their `entity` config field into a full [`RuntimeEntity`]
/// without another round trip per component.
async fn load_entities(pool: &PgPool, app_id: i32) -> Result<HashMap<String, RuntimeEntity>> {
    let entity_rows: Vec<(i32, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "select id, name, table_name, source_query, source_collection from pgapp_meta.entities where app_id = $1",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut entities = HashMap::new();
    for (entity_id, name, table_name, source_query, source_collection) in entity_rows {
        let field_rows: Vec<(String, String, bool)> = sqlx::query_as(
            "select name, data_type, is_required from pgapp_meta.fields
              where entity_id = $1 order by ordinal",
        )
        .bind(entity_id)
        .fetch_all(pool)
        .await?;

        let fields = field_rows
            .into_iter()
            .map(|(name, data_type, required)| RuntimeField {
                name,
                data_type: FieldType::from_str_lossy(&data_type),
                required,
            })
            .collect();

        entities.insert(name.clone(), RuntimeEntity { name, table_name, fields, source_query, source_collection });
    }
    Ok(entities)
}

/// Loads and compiles every named query for the app, split into the
/// app-scoped map and a page id -> (name -> query) map for page-scoped
/// ones.
async fn load_queries(
    pool: &PgPool,
    app_id: i32,
    data_schema: &str,
) -> Result<(HashMap<String, RuntimeQuery>, HashMap<i32, HashMap<String, RuntimeQuery>>)> {
    let rows: Vec<(Option<i32>, String, String)> = sqlx::query_as(
        "select page_id, name, sql_text from pgapp_meta.named_queries where app_id = $1",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let mut app_queries = HashMap::new();
    let mut page_queries: HashMap<i32, HashMap<String, RuntimeQuery>> = HashMap::new();
    for (page_id, name, sql_text) in rows {
        let (sql, bind_names) = compile_named_query(pool, data_schema, &sql_text)
            .await
            .with_context(|| format!("named query '{name}'"))?;
        let rq = RuntimeQuery { sql, bind_names };
        match page_id {
            Some(pid) => {
                page_queries.entry(pid).or_default().insert(name, rq);
            }
            None => {
                app_queries.insert(name, rq);
            }
        }
    }
    Ok((app_queries, page_queries))
}

/// Loads every named auth scheme for the app, keyed by name — see
/// `model::AuthScheme` / `server::auth::authorize`.
async fn load_auth_schemes(pool: &PgPool, app_id: i32) -> Result<HashMap<String, Vec<String>>> {
    let rows: Vec<(String, Vec<String>)> =
        sqlx::query_as("select name, roles from pgapp_meta.auth_schemes where app_id = $1")
            .bind(app_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Loads the components owned by one page, in order.
async fn load_components(
    pool: &PgPool,
    owner_col: &str,
    owner_id: i32,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<Vec<RuntimeComponent>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(&format!(
        "select kind, config from pgapp_meta.components
          where {owner_col} = $1 and slot is null order by ordinal"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(kind, config)| decode_component(&kind, config, entities))
        .collect()
}

/// Loads the app-wide header/footer chrome (`slot` = "header"/"footer",
/// `page_id` null) — restricted to Text/Link/Region at sync time, but
/// decoded through the same generic path as page components.
async fn load_chrome(
    pool: &PgPool,
    app_id: i32,
    slot: &str,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<Vec<RuntimeComponent>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "select kind, config from pgapp_meta.components
          where app_id = $1 and slot = $2 and page_id is null order by ordinal",
    )
    .bind(app_id)
    .bind(slot)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(kind, config)| decode_component(&kind, config, entities))
        .collect()
}

fn json_strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

/// Decodes one `{"id": ..., "class": ..., "attrs": {...}}` blob (the
/// wire shape `meta::sync::html_attrs_to_json` writes) back into an
/// [`HtmlAttrs`].
fn html_attrs_from_json(html: &serde_json::Value) -> HtmlAttrs {
    let attrs = html
        .get("attrs")
        .and_then(|v| v.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect();
    HtmlAttrs {
        id: html.get("id").and_then(|v| v.as_str()).map(String::from),
        class: html.get("class").and_then(|v| v.as_str()).map(String::from),
        attrs,
    }
}

/// Decodes the `"html"` key `meta::sync::merge_html_into_config` writes
/// into every component's config. Missing (no `attrs (...)` in the
/// markup) decodes to the all-`None`/empty default, same as a
/// freshly-parsed component that never set one.
fn decode_html_attrs(config: &serde_json::Value) -> HtmlAttrs {
    match config.get("html") {
        Some(html) => html_attrs_from_json(html),
        None => HtmlAttrs::default(),
    }
}

/// Decodes a Form/EditableTable's `"field_html"` key — one
/// `html_attrs_from_json`-shaped entry per field that used a trailing
/// `attrs (...)` on its `item` line. Missing/empty decodes to an empty
/// map, same as a component where no field ever set one.
fn decode_field_html(config: &serde_json::Value) -> HashMap<String, HtmlAttrs> {
    config
        .get("field_html")
        .and_then(|v| v.as_object())
        .into_iter()
        .flatten()
        .map(|(field, html)| (field.clone(), html_attrs_from_json(html)))
        .collect()
}

/// Decodes a report's `"before_load"` key — the inverse of
/// `meta::sync::before_load_json`. Missing/null (no `before_load:` in
/// the markup) decodes to `None`.
fn decode_before_load(config: &serde_json::Value) -> Option<PreAction> {
    let v = config.get("before_load")?;
    if v.is_null() {
        return None;
    }
    Some(PreAction {
        name: v.get("name").and_then(|x| x.as_str())?.to_string(),
        config: v.get("config").cloned().unwrap_or(serde_json::json!({})),
    })
}

/// Decodes a report's `"computed"` array — the inverse of the
/// `computed_json` built in `meta::sync::build_component_config`.
/// Missing/empty decodes to an empty `Vec`.
fn decode_computed(config: &serde_json::Value) -> Vec<ComputedColumn> {
    config
        .get("computed")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(ComputedColumn {
                name: c.get("name")?.as_str()?.to_string(),
                sql: c.get("sql")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// Decodes a report's `"formats"` object — the inverse of
/// `FormatMask::to_json`. Missing/empty decodes to an empty map; an
/// entry that doesn't decode (shouldn't happen for anything sync itself
/// wrote) is silently skipped rather than failing the whole app load.
fn decode_formats(config: &serde_json::Value) -> HashMap<String, FormatMask> {
    config
        .get("formats")
        .and_then(|v| v.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(col, mask)| FormatMask::from_json(mask).map(|m| (col.clone(), m)))
        .collect()
}

fn decode_aggregates(config: &serde_json::Value) -> HashMap<String, AggregateFn> {
    config
        .get("aggregates")
        .and_then(|v| v.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(col, kind)| AggregateFn::parse(kind.as_str()?).map(|a| (col.clone(), a)))
        .collect()
}

fn decode_item_types(v: &serde_json::Value) -> HashMap<String, FieldItem> {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| {
            let kind = val.get("kind").and_then(|x| x.as_str()).unwrap_or("text").to_string();
            let config = val.get("config").cloned().unwrap_or(serde_json::json!({}));
            (k.clone(), FieldItem { kind, config })
        })
        .collect()
}

fn resolve_entity(
    entities: &HashMap<String, RuntimeEntity>,
    name: &str,
) -> Result<RuntimeEntity> {
    entities
        .get(name)
        .cloned()
        .with_context(|| format!("component references unknown entity '{name}'"))
}

/// Decodes one `(kind, config)` row back into a [`RuntimeComponent`].
/// The inverse of `meta::sync::build_component_config`.
fn decode_component(
    kind: &str,
    config: serde_json::Value,
    entities: &HashMap<String, RuntimeEntity>,
) -> Result<RuntimeComponent> {
    let html = decode_html_attrs(&config);
    let requires = config.get("requires").and_then(|v| v.as_str()).map(String::from);
    match kind {
        "report" => {
            let entity = resolve_entity(entities, &json_str(&config, "entity"))?;
            let link_column = match config.get("link") {
                Some(v) if !v.is_null() => Some(LinkColumn {
                    field: json_str(v, "field"),
                    target_page: json_str(v, "target_page"),
                    extra_params: decode_extra_params(v.get("extra_params")),
                }),
                _ => None,
            };
            Ok(RuntimeComponent::Report {
                title: json_str(&config, "title"),
                entity,
                columns: json_strings(&config["columns"]),
                source_query: config.get("source_query").and_then(|v| v.as_str()).map(String::from),
                link_column,
                page_size: config.get("page_size").and_then(|v| v.as_i64()).unwrap_or(20),
                before_load: decode_before_load(&config),
                computed: decode_computed(&config),
                formats: decode_formats(&config),
                aggregates: decode_aggregates(&config),
                break_on: config.get("break_on").and_then(|v| v.as_str()).map(String::from),
                display: config.get("display").and_then(|v| v.as_str()).unwrap_or("table").to_string(),
                requires,
                html,
            })
        }
        "form" => Ok(RuntimeComponent::Form {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            fields: json_strings(&config["fields"]),
            item_types: decode_item_types(&config["item_types"]),
            field_html: decode_field_html(&config),
            requires,
            html,
        }),
        "editable_table" => Ok(RuntimeComponent::EditableTable {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            columns: json_strings(&config["columns"]),
            item_types: decode_item_types(&config["item_types"]),
            field_html: decode_field_html(&config),
            requires,
            html,
        }),
        "chart" => Ok(RuntimeComponent::Chart {
            title: json_str(&config, "title"),
            query: json_str(&config, "query"),
            chart_type: json_str(&config, "chart_type"),
            x: json_str(&config, "x"),
            y: json_str(&config, "y"),
            requires,
            html,
        }),
        "text" => Ok(RuntimeComponent::Text { text: json_str(&config, "text"), requires, html }),
        "link" => Ok(RuntimeComponent::Link {
            label: json_str(&config, "label"),
            target_page: json_str(&config, "target_page"),
            requires,
            html,
        }),
        "region" => Ok(RuntimeComponent::Region {
            label: json_str(&config, "label"),
            query: json_str(&config, "query"),
            columns: json_strings(&config["columns"]),
            requires,
            html,
        }),
        "dynamic_content" => Ok(RuntimeComponent::DynamicContent {
            label: json_str(&config, "label"),
            name: json_str(&config, "name"),
            config: config.get("config").cloned().unwrap_or(serde_json::json!({})),
            requires,
            html,
        }),
        "action" => Ok(RuntimeComponent::Action {
            label: json_str(&config, "label"),
            name: json_str(&config, "name"),
            config: config.get("config").cloned().unwrap_or(serde_json::json!({})),
            requires,
            html,
        }),
        "button" => {
            let behavior = match config.get("behavior").and_then(|v| v.as_str()) {
                Some("run_action") => ButtonBehavior::RunAction {
                    name: json_str(&config, "name"),
                    config: config.get("config").cloned().unwrap_or(serde_json::json!({})),
                },
                _ => ButtonBehavior::Redirect {
                    target_page: json_str(&config, "target_page"),
                    extra_params: decode_extra_params(config.get("extra_params")),
                },
            };
            Ok(RuntimeComponent::Button { label: json_str(&config, "label"), behavior, requires, html })
        }
        "dynamic_action" => Ok(RuntimeComponent::DynamicAction { config }),
        "calendar" => Ok(RuntimeComponent::Calendar {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            date_field: json_str(&config, "date_field"),
            title_field: json_str(&config, "title_field"),
            link_page: config.get("link_page").and_then(|v| v.as_str()).map(String::from),
            requires,
            html,
        }),
        "map" => Ok(RuntimeComponent::Map {
            title: json_str(&config, "title"),
            entity: resolve_entity(entities, &json_str(&config, "entity"))?,
            lat_field: json_str(&config, "lat_field"),
            lng_field: json_str(&config, "lng_field"),
            title_field: json_str(&config, "title_field"),
            link_page: config.get("link_page").and_then(|v| v.as_str()).map(String::from),
            requires,
            html,
        }),
        other => anyhow::bail!("unknown component kind '{other}' in pgapp_meta.components"),
    }
}

/// Decodes a JSON array of `[field, param]` pairs (or an absent/null
/// value) into `Vec<(String, String)>` — the wire shape both a
/// report's `LinkColumn::extra_params` and a button's
/// `ButtonBehavior::Redirect::extra_params` share.
fn decode_extra_params(v: Option<&serde_json::Value>) -> Vec<(String, String)> {
    v.and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let arr = pair.as_array()?;
            Some((arr.first()?.as_str()?.to_string(), arr.get(1)?.as_str()?.to_string()))
        })
        .collect()
}

fn build_nav_tree(
    rows: &[(i32, Option<i32>, String, Option<i32>)],
    parent: Option<i32>,
    page_names: &HashMap<i32, String>,
) -> Vec<NavNode> {
    rows.iter()
        .filter(|(_, p, ..)| *p == parent)
        .map(|(id, _, label, target_page_id)| NavNode {
            label: label.clone(),
            target_page: target_page_id.map(|tid| page_names[&tid].clone()),
            children: build_nav_tree(rows, Some(*id), page_names),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The actual `Describe`-based type resolution in `compile_named_query`
    // needs a live Postgres connection (see the live-verified example
    // apps instead — every named query with binds in examples/ exercises
    // it), but the pure lexer underneath it — splitting `:name` markers
    // out of hand-written SQL, deduping repeats, leaving a literal `::`
    // alone — doesn't, so that's what these test directly.

    #[test]
    fn tokenizes_bind_markers_and_dedupes_repeats() {
        let (segments, names) = tokenize_binds(
            "select * from t where a = :id::integer or b = :id::integer or c::text = 'x' and d = :status",
        );
        assert_eq!(names, vec!["id".to_string(), "status".to_string()]);
        assert_eq!(segments.iter().filter(|s| **s == Segment::Bind("id".to_string())).count(), 2);
        assert_eq!(segments.iter().filter(|s| **s == Segment::Bind("status".to_string())).count(), 1);
        // A literal `::` (someone's own cast, on the bind or elsewhere)
        // is never consumed as part of a bind marker.
        assert!(segments.iter().any(|s| matches!(s, Segment::Text(t) if t.contains("::integer"))));
        assert!(segments.iter().any(|s| matches!(s, Segment::Text(t) if t.contains("c::text"))));
    }

    #[test]
    fn renders_bare_and_typed_placeholders_from_the_same_segments() {
        let (segments, names) = tokenize_binds("where a = :id or b = :id or c = :status");
        assert_eq!(render_segments(&segments, &names, None), "where a = $1 or b = $1 or c = $2");
        let casts = vec!["INT4".to_string(), "TEXT".to_string()];
        assert_eq!(
            render_segments(&segments, &names, Some(&casts)),
            "where a = $1::INT4 or b = $1::INT4 or c = $2::TEXT"
        );
    }

    #[test]
    fn sql_with_no_binds_is_returned_unchanged_by_the_tokenizer() {
        let (segments, names) = tokenize_binds("select * from t where a::integer = 1");
        assert!(names.is_empty());
        assert_eq!(render_segments(&segments, &names, None), "select * from t where a::integer = 1");
    }
}
