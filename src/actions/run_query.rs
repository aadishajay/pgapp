//! The general-purpose action module: executes a named query for its
//! side effects. Unlike regions/LOVs (which wrap queries in `to_jsonb`
//! for generic decoding), this executes the compiled SQL *raw* — so the
//! named query may be an UPDATE/DELETE/INSERT statement, which Postgres
//! doesn't allow inside a subquery. Bind markers fill from the request's
//! parameter map like everywhere else, so the SQL is still never string-
//! interpolated.
//!
//! Markup: `action "Close old tickets" calls run_query (query: "close_old")`.

use crate::actions::{clean_db_error, ActionContext, BoxFuture, ServerAction};

pub struct RunQuery;

impl ServerAction for RunQuery {
    fn name(&self) -> &'static str {
        "run_query"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let query_name = ctx
                .config
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("run_query needs a (query: \"...\") config"))?;
            let rq = ctx
                .page
                .resolve_query(ctx.app, query_name)
                .ok_or_else(|| anyhow::anyhow!("run_query references unknown query '{query_name}'"))?;

            let mut query = sqlx::query(&rq.sql);
            for name in &rq.bind_names {
                query = query.bind(ctx.values.get(name).map(|s| s.as_str()));
            }
            // search_path-scoped (see meta::scoped_conn): the query's own
            // SQL may reference this app's tables unqualified, and one
            // pool serves every app/workspace in the process.
            let mut conn = crate::meta::scoped_conn(ctx.pool, &ctx.app.data_schema).await?;
            let result = query.execute(&mut *conn).await.map_err(clean_db_error)?;
            Ok(format!("Done — {} row(s) affected.", result.rows_affected()))
        })
    }
}
