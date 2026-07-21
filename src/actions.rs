//! Pluggable server-side action modules — pgapp's PL/SQL analog.
//!
//! An action is a named piece of server-side Rust, defined in its own
//! file under `src/actions/` and registered here, exactly like item
//! types: adding one means writing the file and adding one line to
//! [`registry`]. A page invokes it with an `action "Label" calls
//! <name> (config...)` component, which renders as a button posting to
//! `/:page/c/:idx/run`; the module gets the pool, the app, its generic
//! JSON config from the markup, and the request's parameter map, and
//! returns a human-readable outcome message (shown as a notice banner)
//! or an error.
//!
//! Same caveat as item types: this is a *compile-time* plugin point —
//! write the file, register it, rebuild, restart.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;

use crate::meta::{RuntimeApp, RuntimePage};

mod call_function;
mod clear_session_state;
/// Not a `ServerAction` (see its own doc) — `pub` so `server.rs`'s
/// dedicated create-app route can call it directly, `AppState` access
/// an action module doesn't have.
pub mod create_app;
mod http_request;
mod log_values;
mod run_query;
mod send_email;
mod set_session_state;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Everything one action invocation gets to work with.
pub struct ActionContext<'a> {
    pub pool: &'a PgPool,
    pub app: &'a RuntimeApp,
    /// The page the action component lives on — page-scoped queries
    /// resolve through it.
    pub page: &'a RuntimePage,
    /// The component's generic config blob from the markup, e.g.
    /// `(query: "close_resolved")` → `{"query": "close_resolved"}`.
    pub config: &'a serde_json::Value,
    /// The request's merged parameter map: the POSTed form fields plus
    /// the page's URL query parameters (form wins on conflict) — the
    /// same shape named-query bind contexts use.
    pub values: &'a HashMap<String, String>,
    /// This request's collection-scoping identity (`server::auth::CallerKey`)
    /// — a module that writes into `pgapp_meta.collections` (see
    /// `http_request`) scopes every row to this, so one caller's data
    /// never becomes visible to another's.
    pub caller_key: &'a str,
}

/// One pluggable server-side action module.
pub trait ServerAction: Send + Sync {
    /// The markup name, e.g. `"run_query"` for `action ... calls run_query`.
    fn name(&self) -> &'static str;

    /// Runs the action. The returned string is shown to the user as a
    /// success notice; an Err becomes the page's error banner.
    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>>;
}

/// Strips sqlx's "error returned from database: " wrapper off a
/// Postgres error, so a PL/pgSQL `raise exception 'Ticket already
/// closed'` shows up on the page as exactly that — "Ticket already
/// closed" — rather than as a client-library-flavored sentence. Only
/// applies to actual database errors (a raised exception, a
/// constraint violation, ...); anything else (a pool timeout, a
/// connection drop) keeps its normal message.
pub(crate) fn clean_db_error(e: sqlx::Error) -> anyhow::Error {
    match e.as_database_error() {
        Some(db_err) => anyhow::anyhow!("{}", db_err.message()),
        None => e.into(),
    }
}

pub type Registry = HashMap<&'static str, Box<dyn ServerAction>>;

/// Builds the registry of every known action module — the one line a
/// new module needs outside of its own file.
pub fn registry() -> Registry {
    let modules: Vec<Box<dyn ServerAction>> = vec![
        Box::new(run_query::RunQuery),
        Box::new(call_function::CallFunction),
        Box::new(log_values::LogValues),
        Box::new(http_request::HttpRequest),
        Box::new(send_email::SendEmail),
        Box::new(set_session_state::SetSessionState),
        Box::new(clear_session_state::ClearSessionState),
    ];
    modules.into_iter().map(|m| (m.name(), m)).collect()
}
