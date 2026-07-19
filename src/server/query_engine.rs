//! Everything about running named queries and resolving what depends on
//! them (LOV choices, regions) — kept separate from `server.rs`'s route
//! handlers, which just call into this.

use std::collections::{BTreeMap, HashMap};

use sqlx::PgPool;

use crate::meta::{wrap_to_jsonb, RegionRows, RuntimeApp, RuntimeComponent, RuntimePage, RuntimeQuery};
use crate::model::FieldItem;

/// Turns a `to_jsonb` result value into the display string the rest of
/// the generic rendering layer expects: `null` becomes absent, other
/// scalars are stringified (strings verbatim, numbers/bools via their
/// JSON text).
fn json_to_display(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Runs a compiled named query, binding `rq.bind_names` from `ctx` (a
/// name missing from `ctx` binds SQL NULL). The query is wrapped in
/// `to_jsonb` so its result can be decoded generically regardless of
/// what columns it selects or what Postgres types they are.
/// `data_schema` scopes the connection's `search_path` (see
/// `meta::scoped_conn`) so a bare table reference in the query's own
/// SQL resolves to this app's tables specifically.
pub async fn run_named_query(
    pool: &PgPool,
    data_schema: &str,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let wrapped = wrap_to_jsonb(&rq.sql);
    let mut query = sqlx::query_scalar::<_, serde_json::Value>(&wrapped);
    for name in &rq.bind_names {
        query = query.bind(ctx.get(name).map(|s| s.as_str()));
    }
    let mut conn = crate::meta::scoped_conn(pool, data_schema).await?;
    Ok(query.fetch_all(&mut *conn).await?)
}

/// Like [`run_named_query`], but paginated with a zero-extra-query
/// OFFSET window: fetches `page_size + 1` rows so the caller can tell
/// whether there's a next page without a separate `COUNT(*)`.
/// Query-sourced reports can't assume a stable sort key exists, so this
/// (rather than keyset pagination) is what backs them — see
/// `fetch_report_rows` for the keyset version used for entity-backed
/// reports.
///
/// `where_clause` (may be empty) filters the wrapped query — its `$N`
/// placeholders must start *after* the query's own binds, and
/// `extra_binds` supplies their values in order. The clause's column
/// references are `t.<name>` where `<name>` came from markup
/// identifiers (lexer-restricted charset), never from request input.
#[allow(clippy::too_many_arguments)]
pub async fn run_named_query_page(
    pool: &PgPool,
    data_schema: &str,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
    where_clause: &str,
    extra_binds: &[String],
    page_size: i64,
    page_num: i64,
) -> anyhow::Result<(Vec<serde_json::Value>, bool)> {
    let offset = (page_num - 1).max(0) * page_size;
    let wrapped = format!(
        "select to_jsonb(t) as j from ({}) as t {} limit {} offset {}",
        rq.sql,
        where_clause,
        page_size + 1,
        offset
    );
    let mut query = sqlx::query_scalar::<_, serde_json::Value>(&wrapped);
    for name in &rq.bind_names {
        query = query.bind(ctx.get(name).map(|s| s.as_str()));
    }
    for bind in extra_binds {
        query = query.bind(bind.as_str());
    }
    let mut conn = crate::meta::scoped_conn(pool, data_schema).await?;
    let mut rows = query.fetch_all(&mut *conn).await?;
    let has_next = rows.len() as i64 > page_size;
    rows.truncate(page_size as usize);
    Ok((rows, has_next))
}

pub async fn run_named_query_rows(
    pool: &PgPool,
    data_schema: &str,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    let rows = run_named_query(pool, data_schema, rq, ctx).await?;
    Ok(rows.into_iter().map(json_row_to_map).collect())
}

pub fn json_row_to_map(row: serde_json::Value) -> BTreeMap<String, Option<String>> {
    match row {
        serde_json::Value::Object(map) => map
            .into_iter()
            .map(|(k, v)| (k, json_to_display(&v)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Resolves every `Region` component's rows across the current page's
/// components (if any — the index page has none) plus the app's
/// header/footer, keyed by query name. Page components may use a
/// page-scoped query; header/footer can only see app-scoped ones
/// (there's no single page to shadow through).
pub async fn resolve_regions(
    pool: &PgPool,
    app: &RuntimeApp,
    page: Option<&RuntimePage>,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<RegionRows> {
    let mut out = RegionRows::new();

    let page_components = page.map(|p| p.components.as_slice()).unwrap_or(&[]);
    for component in page_components.iter().chain(app.header.iter()).chain(app.footer.iter()) {
        let RuntimeComponent::Region { query, .. } = component else {
            continue;
        };
        if out.contains_key(query) {
            continue;
        }
        let rq = page
            .and_then(|p| p.resolve_query(app, query))
            .or_else(|| app.queries.get(query))
            .ok_or_else(|| anyhow::anyhow!("region references unknown query '{query}'"))?;
        out.insert(query.clone(), run_named_query_rows(pool, &app.data_schema, rq, ctx).await?);
    }

    Ok(out)
}

/// Resolves live (value, label) choices for every field in one Form's
/// or EditableTable's `item_types`, keyed by field name — generic over
/// item *kind*: any field whose config has a `"query"` key gets its
/// choices from that named query; a `"choices"` array is used as-is;
/// anything else gets an empty list. This doesn't special-case Radio/
/// Popup (or any other kind) at all — it just implements the two
/// reserved config keys any component may choose to use.
pub async fn resolve_field_choices(
    pool: &PgPool,
    app: &RuntimeApp,
    page: &RuntimePage,
    item_types: &HashMap<String, FieldItem>,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<HashMap<String, Vec<(String, String)>>> {
    let mut out = HashMap::new();
    for (field_name, field_item) in item_types {
        let choices = if let Some(query_name) = field_item.config.get("query").and_then(|v| v.as_str()) {
            let rq = page.resolve_query(app, query_name).ok_or_else(|| {
                anyhow::anyhow!("field '{field_name}' references unknown query '{query_name}'")
            })?;
            run_named_query(pool, &app.data_schema, rq, ctx)
                .await?
                .into_iter()
                .filter_map(|row| {
                    let value = row.get("value")?.as_str()?.to_string();
                    let label = row
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&value)
                        .to_string();
                    Some((value, label))
                })
                .collect()
        } else if let Some(list) = field_item.config.get("choices").and_then(|v| v.as_array()) {
            list.iter()
                .filter_map(|v| v.as_str())
                .map(|s| (s.to_string(), s.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        out.insert(field_name.clone(), choices);
    }
    Ok(out)
}

/// Bind context available to named queries on one request: the URL's
/// query-string parameters, plus — when editing or viewing a specific
/// row — that row's own field values, so e.g. a popup LOV can filter by
/// another field on the same row. Query-string values win on conflict.
pub fn bind_context(
    query_params: &HashMap<String, String>,
    row: Option<&BTreeMap<String, Option<String>>>,
) -> HashMap<String, String> {
    let mut ctx = HashMap::new();
    if let Some(row) = row {
        for (k, v) in row {
            if let Some(v) = v {
                ctx.insert(k.clone(), v.clone());
            }
        }
    }
    for (k, v) in query_params {
        ctx.insert(k.clone(), v.clone());
    }
    ctx
}
