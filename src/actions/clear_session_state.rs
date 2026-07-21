//! Clears a named "session state" value previously written by
//! `set_session_state` — deletes its row(s) from `pgapp_meta.collections`
//! outright, scoped to the current caller.
//!
//! Markup:
//! ```text
//! action "Clear filter" calls clear_session_state (
//!   name: "selected_status"
//! )
//! ```

use crate::actions::{ActionContext, BoxFuture, ServerAction};

pub struct ClearSessionState;

impl ServerAction for ClearSessionState {
    fn name(&self) -> &'static str {
        "clear_session_state"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let name = ctx.config.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() {
                anyhow::bail!("clear_session_state needs a (name: \"...\") config");
            }

            sqlx::query("delete from pgapp_meta.collections where app_id = $1 and caller_key = $2 and name = $3")
                .bind(ctx.app.id)
                .bind(ctx.caller_key)
                .bind(name)
                .execute(ctx.pool)
                .await
                .map_err(|e| anyhow::anyhow!("clear_session_state: {e}"))?;

            Ok(format!("Cleared session state '{name}'."))
        })
    }
}
