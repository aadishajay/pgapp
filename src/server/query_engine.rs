//! Everything about running named queries and resolving what depends on
//! them (LOV choices, regions) — kept separate from `server.rs`'s route
//! handlers, which just call into this.

use std::collections::{BTreeMap, HashMap};

use sqlx::PgPool;

use crate::meta::{RegionRows, RuntimeApp, RuntimePage, RuntimePageItem, RuntimeQuery};

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
pub async fn run_named_query(
    pool: &PgPool,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let wrapped = format!("select to_jsonb(t) as j from ({}) as t", rq.sql);
    let mut query = sqlx::query_scalar::<_, serde_json::Value>(&wrapped);
    for name in &rq.bind_names {
        query = query.bind(ctx.get(name).map(|s| s.as_str()));
    }
    Ok(query.fetch_all(pool).await?)
}

pub async fn run_named_query_rows(
    pool: &PgPool,
    rq: &RuntimeQuery,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<Vec<BTreeMap<String, Option<String>>>> {
    let rows = run_named_query(pool, rq, ctx).await?;
    Ok(rows
        .into_iter()
        .map(|row| match row {
            serde_json::Value::Object(map) => map
                .into_iter()
                .map(|(k, v)| (k, json_to_display(&v)))
                .collect(),
            _ => BTreeMap::new(),
        })
        .collect())
}

/// Resolves every `Region` item's rows across the current page's items
/// plus the app's header/footer, keyed by query name. Page items may
/// use a page-scoped query; header/footer can only see app-scoped ones
/// (there's no single page to shadow through).
pub async fn resolve_regions(
    pool: &PgPool,
    app: &RuntimeApp,
    page_items: &[RuntimePageItem],
    page: Option<&RuntimePage>,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<RegionRows> {
    let mut out = RegionRows::new();

    for item in page_items.iter().chain(app.header.iter()).chain(app.footer.iter()) {
        let RuntimePageItem::Region { query, .. } = item else {
            continue;
        };
        if out.contains_key(query) {
            continue;
        }
        let rq = page
            .and_then(|p| p.resolve_query(app, query))
            .or_else(|| app.queries.get(query))
            .ok_or_else(|| anyhow::anyhow!("region references unknown query '{query}'"))?;
        out.insert(query.clone(), run_named_query_rows(pool, rq, ctx).await?);
    }

    Ok(out)
}

/// Resolves live (value, label) choices for every form field, keyed by
/// field name — generic over item *kind*: any field whose config has a
/// `"query"` key gets its choices from that named query; a `"choices"`
/// array is used as-is; anything else gets an empty list. This doesn't
/// special-case Radio/Popup (or any other kind) at all — it just
/// implements the two reserved config keys any component may choose to
/// use.
pub async fn resolve_field_choices(
    pool: &PgPool,
    app: &RuntimeApp,
    page: &RuntimePage,
    ctx: &HashMap<String, String>,
) -> anyhow::Result<HashMap<String, Vec<(String, String)>>> {
    let mut out = HashMap::new();
    for (field_name, field_item) in &page.item_types {
        let choices = if let Some(query_name) = field_item.config.get("query").and_then(|v| v.as_str()) {
            let rq = page.resolve_query(app, query_name).ok_or_else(|| {
                anyhow::anyhow!("field '{field_name}' references unknown query '{query_name}'")
            })?;
            run_named_query(pool, rq, ctx)
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
