//! Calls a plain SQL/PL/pgSQL function and shows its own return value
//! as the outcome — the PL/pgSQL-native counterpart to `run_query`
//! (which is built for bulk `UPDATE`/`DELETE`, not "run one function
//! and report what it says"). Markup: `action "Close ticket" calls
//! call_function (query: "close_ticket")`, where `close_ticket` is a
//! named query whose SQL is a single function call — `sql: "select
//! close_ticket(:id, :note)"`.
//!
//! The whole point is that the *decision* (did this succeed, and what
//! should the user be told) lives in the function, in PL/pgSQL, not in
//! this Rust module: `raise exception '...'` becomes the page's error
//! banner (see `actions::clean_db_error`) and whatever the function
//! `return`s (any scalar type — text, integer, boolean, a `record`
//! that quotes as text, ...) becomes the success notice. Binds go
//! through the same `:name` compilation and automatic type inference
//! as every other named query — see `meta::compile_named_query` — so
//! calling a two-argument PL/pgSQL function needs no casts either.

use crate::actions::{clean_db_error, ActionContext, BoxFuture, ServerAction};
use crate::meta::wrap_to_jsonb;

pub struct CallFunction;

impl ServerAction for CallFunction {
    fn name(&self) -> &'static str {
        "call_function"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let query_name = ctx
                .config
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("call_function needs a (query: \"...\") config"))?;
            let rq = ctx
                .page
                .resolve_query(ctx.app, query_name)
                .ok_or_else(|| anyhow::anyhow!("call_function references unknown query '{query_name}'"))?;

            // Wrapped in to_jsonb like every other generically-decoded
            // query result, so the function's return type never needs
            // to be known ahead of time — see query_engine::run_named_query.
            let wrapped = wrap_to_jsonb(&rq.sql);
            let mut query = sqlx::query_scalar::<_, serde_json::Value>(&wrapped);
            for name in &rq.bind_names {
                query = query.bind(ctx.values.get(name).map(|s| s.as_str()));
            }
            // search_path-scoped (see meta::scoped_conn): the query's own
            // SQL may reference this app's tables unqualified, and one
            // pool serves every app/workspace in the process.
            let mut conn = crate::meta::scoped_conn(ctx.pool, &ctx.app.data_schema).await?;
            let row = query.fetch_one(&mut *conn).await.map_err(clean_db_error)?;

            // `row` is a one-column object, e.g. {"close_ticket": "Closed
            // ticket 5: fixed the thing"} — the function's own message,
            // whatever type it actually returned.
            let message = match row {
                serde_json::Value::Object(map) => map.into_values().next().and_then(|v| match v {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => Some(s),
                    other => Some(other.to_string()),
                }),
                _ => None,
            };
            Ok(message.unwrap_or_else(|| "Done.".to_string()))
        })
    }
}
