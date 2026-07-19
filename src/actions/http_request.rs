//! Calls an external REST API as a server-side action — the outbound
//! counterpart to `run_query`/`call_function` (which only ever talk to
//! Postgres). Every config value is a plain string (the grammar's
//! `itemconfig` doesn't support nested objects), so anything that's
//! naturally a list — extra headers — is packed into one string and
//! parsed here instead of by markup.rs.
//!
//! Markup:
//! ```text
//! action "Notify webhook" calls http_request (
//!   method: "POST",
//!   url: "https://hooks.example.com/tickets/{{id}}",
//!   body: "{\"status\": \"{{status}}\"}",
//!   headers: "X-Source: pgapp",
//!   auth: "bearer",
//!   token: "abc123"
//! )
//! ```
//! `{{item}}` in `url`/`body`/`headers`/`token`/`username`/`password`/
//! `key_value` is replaced with that page item's current value (the
//! same `ctx.values` map dynamic actions and named-query binds read
//! from) before the request is built — so a page can post its own
//! current state to an external endpoint with no extra wiring.
//! `{{secret.<name>}}` works the same way in the same fields, except
//! the value comes from `pgapp_control.secrets` (`pgapp secret set`,
//! see `src/secrets.rs`) instead of a page item — for a fixed
//! credential (an API key, a service account token) that isn't
//! user-typed and shouldn't sit in plaintext in the markup file.
//!
//! `auth`: `none` (default) | `basic` (`username`/`password`) |
//! `bearer` (`token`) | `api_key_header` (`key_name`/`key_value`, sent
//! as a request header) | `api_key_query` (`key_name`/`key_value`,
//! appended to the URL's query string). Not covered: full OAuth2
//! grant flows (client-credentials, refresh) — those need a token
//! cache with its own lifetime, which is a bigger feature than one
//! action module; `bearer` still works if you already have a token in
//! hand.
//!
//! `collection: "<name>"` captures a successful (2xx) JSON response
//! into `pgapp_meta.collections` (an APEX-collection-style scratch
//! table — see db/schema.sql) instead of just echoing it back: a JSON
//! array becomes one row per element, a bare object becomes one row.
//! `collection_mode`: `"replace"` (default, clears any existing rows
//! under that name first) or `"append"`. Rows are scoped to the
//! current caller (`server::auth::CallerKey`) — the same browser that
//! triggered the action — so nothing another visitor does can read or
//! overwrite them. Read a collection back with `entity "x" from
//! collection "<name>" { field ...: type ... }`, which any `report`
//! can bind to exactly like a table- or query-backed entity.

use std::time::Duration;

use reqwest::Method;

use crate::actions::{ActionContext, BoxFuture, ServerAction};

pub struct HttpRequest;

const DEFAULT_TIMEOUT_SECS: u64 = 10;
const MAX_ECHOED_BODY: usize = 2000;

impl ServerAction for HttpRequest {
    fn name(&self) -> &'static str {
        "http_request"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let cfg = |key: &str| ctx.config.get(key).and_then(|v| v.as_str()).unwrap_or("");

            // {{secret.name}} can appear anywhere {{item}} can; resolve
            // every one referenced by this config into a combined map
            // up front, so `interpolate` itself stays a plain, sync
            // string substitution over one map — same lookup either
            // way, `secret.` is just a naming convention, not special
            // syntax to the interpolator.
            let templated_fields =
                [cfg("url"), cfg("body"), cfg("headers"), cfg("token"), cfg("username"), cfg("password"), cfg("key_value")];
            let secret_names = secret_placeholders(&templated_fields);
            let mut values = ctx.values.clone();
            if !secret_names.is_empty() {
                let key = crate::secrets::load_key().map_err(|e| anyhow::anyhow!("http_request: {e}"))?;
                for name in &secret_names {
                    let value =
                        crate::secrets::resolve(ctx.pool, &key, ctx.app.control_app_id, ctx.app.workspace_id, name)
                            .await
                            .map_err(|e| anyhow::anyhow!("http_request: failed to resolve secret '{name}': {e}"))?
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "http_request: no secret named '{name}' is set for this app or its workspace \
                                     (see `pgapp secret list <dbname> --app <slug>`)"
                                )
                            })?;
                    values.insert(format!("secret.{name}"), value);
                }
            }
            let interp = |s: &str| interpolate(s, &values);

            let url = cfg("url");
            if url.is_empty() {
                anyhow::bail!("http_request needs a (url: \"...\") config");
            }
            let mut url = interp(url);

            let method_str = if cfg("method").is_empty() { "GET" } else { cfg("method") };
            let method = Method::from_bytes(method_str.to_uppercase().as_bytes())
                .map_err(|_| anyhow::anyhow!("http_request: '{method_str}' isn't a valid HTTP method"))?;

            let auth = if cfg("auth").is_empty() { "none" } else { cfg("auth") };
            if auth == "api_key_query" {
                let key_name = cfg("key_name");
                let key_value = interp(cfg("key_value"));
                if key_name.is_empty() {
                    anyhow::bail!("http_request: auth \"api_key_query\" needs key_name and key_value");
                }
                let sep = if url.contains('?') { '&' } else { '?' };
                url = format!("{url}{sep}{}={}", url_encode(key_name), url_encode(&key_value));
            }

            let timeout_secs = cfg("timeout_secs").parse::<u64>().unwrap_or(DEFAULT_TIMEOUT_SECS);
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
                .map_err(|e| anyhow::anyhow!("http_request: failed to build HTTP client: {e}"))?;

            let mut req = client.request(method.clone(), &url);

            for (name, value) in parse_headers(cfg("headers")) {
                req = req.header(name, interp(&value));
            }

            match auth {
                "none" | "api_key_query" => {}
                "basic" => {
                    let username = interp(cfg("username"));
                    let password = interp(cfg("password"));
                    req = req.basic_auth(username, Some(password));
                }
                "bearer" => {
                    let token = interp(cfg("token"));
                    if token.is_empty() {
                        anyhow::bail!("http_request: auth \"bearer\" needs a token");
                    }
                    req = req.bearer_auth(token);
                }
                "api_key_header" => {
                    let key_name = cfg("key_name");
                    let key_value = interp(cfg("key_value"));
                    if key_name.is_empty() {
                        anyhow::bail!("http_request: auth \"api_key_header\" needs key_name and key_value");
                    }
                    req = req.header(key_name, key_value);
                }
                other => anyhow::bail!(
                    "http_request: unknown auth \"{other}\" (expected none, basic, bearer, api_key_header, or api_key_query)"
                ),
            }

            let body_cfg = cfg("body");
            if !body_cfg.is_empty() {
                let content_type = if cfg("content_type").is_empty() { "application/json" } else { cfg("content_type") };
                req = req.header(reqwest::header::CONTENT_TYPE, content_type).body(interp(body_cfg));
            }

            let resp = req.send().await.map_err(|e| anyhow::anyhow!("http_request: {method} {url} failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();

            let collection = cfg("collection");
            if status.is_success() && !collection.is_empty() {
                let mode = if cfg("collection_mode").is_empty() { "replace" } else { cfg("collection_mode") };
                write_collection(ctx.pool, ctx.app.id, ctx.caller_key, collection, mode, &text).await?;
            }

            let echoed = if text.len() > MAX_ECHOED_BODY { format!("{}… ({} bytes total)", &text[..MAX_ECHOED_BODY], text.len()) } else { text };

            if status.is_success() {
                Ok(format!("{method} {url} → {status}\n{echoed}"))
            } else {
                anyhow::bail!("{method} {url} → {status}\n{echoed}")
            }
        })
    }
}

/// Stashes a successful JSON response into `pgapp_meta.collections`,
/// scoped to this caller (never a hand-written WHERE clause — see
/// `EntityDef::source_collection`). A JSON array becomes one row per
/// element; a bare object becomes a single row. `mode: "replace"`
/// (the default) clears any existing rows under this name first, in
/// the same transaction, so the collection never sits half-old/half-
/// new; `"append"` keeps them and continues the `seq` numbering.
async fn write_collection(
    pool: &sqlx::PgPool,
    app_id: i32,
    caller_key: &str,
    name: &str,
    mode: &str,
    body: &str,
) -> anyhow::Result<()> {
    let parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        anyhow::anyhow!("http_request: collection \"{name}\" needs a JSON response body to store, got invalid JSON: {e}")
    })?;
    let items: Vec<serde_json::Value> = match parsed {
        serde_json::Value::Array(items) => items,
        other => vec![other],
    };

    let mut tx = pool.begin().await?;
    let mut next_seq: i32 = if mode == "append" {
        sqlx::query_scalar(
            "select coalesce(max(seq), -1) + 1 from pgapp_meta.collections
              where app_id = $1 and caller_key = $2 and name = $3",
        )
        .bind(app_id)
        .bind(caller_key)
        .bind(name)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query("delete from pgapp_meta.collections where app_id = $1 and caller_key = $2 and name = $3")
            .bind(app_id)
            .bind(caller_key)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        0
    };
    for item in items {
        sqlx::query("insert into pgapp_meta.collections (app_id, caller_key, name, seq, data) values ($1, $2, $3, $4, $5)")
            .bind(app_id)
            .bind(caller_key)
            .bind(name)
            .bind(next_seq)
            .bind(&item)
            .execute(&mut *tx)
            .await?;
        next_seq += 1;
    }
    tx.commit().await?;
    Ok(())
}

/// Every distinct `secret.<name>` referenced across a set of config
/// strings, with the `secret.` prefix stripped — what `run` resolves
/// against `pgapp_control.secrets` before interpolating. A plain
/// `{{item}}` placeholder (no `secret.` prefix) is left alone here;
/// `interpolate` still handles those from `ctx.values` as always.
fn secret_placeholders(templates: &[&str]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for template in templates {
        let bytes = template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'{') {
                if let Some(end) = template[i + 2..].find("}}") {
                    let name = template[i + 2..i + 2 + end].trim();
                    if let Some(secret_name) = name.strip_prefix("secret.") {
                        names.insert(secret_name.to_string());
                    }
                    i += 2 + end + 2;
                    continue;
                }
            }
            i += 1;
        }
    }
    names
}

/// `{{item}}` → that item's current value from the page's bind
/// context (empty string if unset) — a plain string substitution, not
/// SQL-bind casting, since this has nothing to do with Postgres.
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

/// The `headers` config packs multiple `Name: Value` pairs into one
/// string (the grammar has no repeated/nested config shape), separated
/// by `;` — matching how a real header block reads, just on one line.
fn parse_headers(packed: &str) -> Vec<(String, String)> {
    packed
        .split(';')
        .filter_map(|pair| {
            let (name, value) = pair.split_once(':')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), value.to_string()))
            }
        })
        .collect()
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_replaces_known_items_and_leaves_unknown_ones_blank() {
        let mut values = std::collections::HashMap::new();
        values.insert("id".to_string(), "42".to_string());
        values.insert("status".to_string(), "Open".to_string());
        let out = interpolate("https://api.example.com/tickets/{{id}}?state={{status}}&x={{missing}}", &values);
        assert_eq!(out, "https://api.example.com/tickets/42?state=Open&x=");
    }

    #[test]
    fn interpolate_is_a_no_op_with_no_placeholders() {
        assert_eq!(interpolate("https://api.example.com/health", &Default::default()), "https://api.example.com/health");
    }

    #[test]
    fn secret_placeholders_finds_names_across_multiple_templates_and_dedupes() {
        let names = secret_placeholders(&["Bearer {{secret.api_token}}", "{\"key\": \"{{secret.api_token}}\"}", "{{item}}"]);
        assert_eq!(names, ["api_token".to_string()].into_iter().collect());
    }

    #[test]
    fn secret_placeholders_is_empty_with_no_secret_references() {
        assert!(secret_placeholders(&["{{id}}", "no placeholders here"]).is_empty());
    }

    #[test]
    fn parse_headers_splits_semicolon_packed_pairs() {
        let pairs = parse_headers("Content-Type: application/json; X-Source: pgapp");
        assert_eq!(pairs, vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Source".to_string(), "pgapp".to_string()),
        ]);
    }

    #[test]
    fn parse_headers_ignores_blank_input() {
        assert!(parse_headers("").is_empty());
    }

    #[test]
    fn url_encode_escapes_reserved_characters() {
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
        assert_eq!(url_encode("safe-._~123"), "safe-._~123");
    }
}
