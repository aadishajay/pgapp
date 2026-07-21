//! Sets one named "session state" value — an approximation of Oracle
//! APEX's per-item session state, since pgapp has no server-tracked
//! item state outside the database. Stores a single row into
//! `pgapp_meta.collections` (the same generic scratch store
//! `http_request`'s `collection: ` config writes into — see
//! `db/schema.sql`), scoped to the current caller, so it's readable
//! back with `entity "x" from collection "<name>" { field value: text
//! }` like any other collection — no new read mechanism needed.
//!
//! Markup:
//! ```text
//! action "Save filter" calls set_session_state (
//!   name: "selected_status",
//!   value: "{{status}}"
//! )
//! ```
//! `{{item}}` in `value` interpolates that page item's current value,
//! same convention as every other action module. Setting the same
//! `name` again replaces the previous value outright (this is a single
//! scalar, not an appendable list like `http_request`'s
//! `collection_mode: "append"`).

use crate::actions::{ActionContext, BoxFuture, ServerAction};

pub struct SetSessionState;

impl ServerAction for SetSessionState {
    fn name(&self) -> &'static str {
        "set_session_state"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let cfg = |key: &str| ctx.config.get(key).and_then(|v| v.as_str()).unwrap_or("");
            let name = cfg("name");
            if name.is_empty() {
                anyhow::bail!("set_session_state needs a (name: \"...\") config");
            }
            let value = interpolate(cfg("value"), ctx.values);

            let mut tx = ctx.pool.begin().await.map_err(|e| anyhow::anyhow!("set_session_state: {e}"))?;
            sqlx::query("delete from pgapp_meta.collections where app_id = $1 and caller_key = $2 and name = $3")
                .bind(ctx.app.id)
                .bind(ctx.caller_key)
                .bind(name)
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("set_session_state: {e}"))?;
            sqlx::query("insert into pgapp_meta.collections (app_id, caller_key, name, seq, data) values ($1, $2, $3, 0, $4)")
                .bind(ctx.app.id)
                .bind(ctx.caller_key)
                .bind(name)
                .bind(serde_json::json!({ "value": value }))
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("set_session_state: {e}"))?;
            tx.commit().await.map_err(|e| anyhow::anyhow!("set_session_state: {e}"))?;

            Ok(format!("Set session state '{name}'."))
        })
    }
}

/// `{{item}}` → that item's current value from the page's bind
/// context (empty string if unset) — same as `http_request::interpolate`.
fn interpolate(template: &str, values: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'{') {
            if let Some(end) = template[i + 2..].find("}}") {
                let name = template[i + 2..i + 2 + end].trim();
                out.push_str(values.get(name).map(String::as_str).unwrap_or(""));
                i += 2 + end + 2;
                continue;
            }
        }
        out.push(template[i..].chars().next().unwrap());
        i += template[i..].chars().next().unwrap().len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_replaces_known_items_and_leaves_unknown_ones_blank() {
        let mut values = std::collections::HashMap::new();
        values.insert("status".to_string(), "Open".to_string());
        assert_eq!(interpolate("{{status}}/{{missing}}", &values), "Open/");
    }
}
