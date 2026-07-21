//! Sends an email via SMTP as a server-side action — Oracle APEX's
//! "Send Email" process type. SMTP credentials come from
//! `pgapp_control.secrets` via `{{secret.<name>}}` placeholders (see
//! `secrets.rs`), the same convention `http_request`'s `token`/
//! `password` fields use, rather than sitting in plaintext in the
//! markup file.
//!
//! Markup:
//! ```text
//! action "Notify customer" calls send_email (
//!   to: "{{customer_email}}",
//!   from: "support@example.com",
//!   subject: "Ticket #{{id}} updated",
//!   body: "Your ticket status is now {{status}}.",
//!   smtp_host: "smtp.example.com",
//!   smtp_port: "587",
//!   smtp_username: "{{secret.smtp_username}}",
//!   smtp_password: "{{secret.smtp_password}}"
//! )
//! ```
//! `{{item}}` in `to`/`from`/`subject`/`body` is replaced with that
//! page item's current value, same as every other action module;
//! `{{secret.<name>}}` works in any field (typically `smtp_username`/
//! `smtp_password`) and resolves against `pgapp_control.secrets`
//! instead. `to` may be a comma-separated list. `smtp_port` defaults
//! to 587 (STARTTLS, the common case for a real SMTP provider); a
//! provider that speaks only implicit TLS on 465 isn't covered — that
//! needs a different lettre transport setup than this connects.
//! `content_type: "html"` sends the body as `text/html` instead of the
//! default `text/plain`.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::actions::{ActionContext, BoxFuture, ServerAction};

pub struct SendEmail;

impl ServerAction for SendEmail {
    fn name(&self) -> &'static str {
        "send_email"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let cfg = |key: &str| ctx.config.get(key).and_then(|v| v.as_str()).unwrap_or("");

            let templated_fields = [cfg("to"), cfg("from"), cfg("subject"), cfg("body"), cfg("smtp_username"), cfg("smtp_password")];
            let secret_names = secret_placeholders(&templated_fields);
            let mut values = ctx.values.clone();
            if !secret_names.is_empty() {
                let key = crate::secrets::load_key().map_err(|e| anyhow::anyhow!("send_email: {e}"))?;
                for name in &secret_names {
                    let value =
                        crate::secrets::resolve(ctx.pool, &key, ctx.app.control_app_id, ctx.app.workspace_id, name)
                            .await
                            .map_err(|e| anyhow::anyhow!("send_email: failed to resolve secret '{name}': {e}"))?
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "send_email: no secret named '{name}' is set for this app or its workspace \
                                     (see `pgapp secret list <dbname> --app <slug>`)"
                                )
                            })?;
                    values.insert(format!("secret.{name}"), value);
                }
            }
            let interp = |s: &str| interpolate(s, &values);

            let to = interp(cfg("to"));
            let from = interp(cfg("from"));
            let smtp_host = cfg("smtp_host");
            if to.is_empty() || from.is_empty() || smtp_host.is_empty() {
                anyhow::bail!("send_email needs (to: \"...\", from: \"...\", smtp_host: \"...\") config");
            }
            let subject = interp(cfg("subject"));
            let body = interp(cfg("body"));
            let smtp_port: u16 = cfg("smtp_port").parse().unwrap_or(587);
            let is_html = cfg("content_type").eq_ignore_ascii_case("html");

            let mut builder = Message::builder()
                .from(from.parse().map_err(|e| anyhow::anyhow!("send_email: invalid 'from' address '{from}': {e}"))?)
                .subject(&subject);
            for addr in to.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                builder = builder.to(addr.parse().map_err(|e| anyhow::anyhow!("send_email: invalid 'to' address '{addr}': {e}"))?);
            }
            let content_type = if is_html { ContentType::TEXT_HTML } else { ContentType::TEXT_PLAIN };
            let email = builder
                .header(content_type)
                .body(body)
                .map_err(|e| anyhow::anyhow!("send_email: failed to build message: {e}"))?;

            let mut transport_builder = AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
                .map_err(|e| anyhow::anyhow!("send_email: failed to configure SMTP relay '{smtp_host}': {e}"))?
                .port(smtp_port);
            let username = interp(cfg("smtp_username"));
            if !username.is_empty() {
                transport_builder = transport_builder.credentials(Credentials::new(username, interp(cfg("smtp_password"))));
            }
            let transport = transport_builder.build();

            transport
                .send(email)
                .await
                .map_err(|e| anyhow::anyhow!("send_email: failed to send via {smtp_host}:{smtp_port}: {e}"))?;

            Ok(format!("Sent email to {to} via {smtp_host}:{smtp_port}."))
        })
    }
}

/// Every distinct `secret.<name>` referenced across a set of config
/// strings, with the `secret.` prefix stripped — same convention as
/// `http_request::secret_placeholders`, duplicated rather than shared
/// since each action module is meant to stand alone (see
/// `actions.rs`'s module doc).
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
        values.insert("id".to_string(), "42".to_string());
        let out = interpolate("Ticket #{{id}} for {{missing}}", &values);
        assert_eq!(out, "Ticket #42 for ");
    }

    #[test]
    fn secret_placeholders_finds_names_and_dedupes() {
        let names = secret_placeholders(&["{{secret.smtp_username}}", "{{secret.smtp_username}}:{{secret.smtp_password}}"]);
        assert_eq!(names, ["smtp_username".to_string(), "smtp_password".to_string()].into_iter().collect());
    }

    #[test]
    fn secret_placeholders_is_empty_with_no_secret_references() {
        assert!(secret_placeholders(&["{{to}}", "no placeholders"]).is_empty());
    }
}
