//! The App Builder's "New Workspace" page — two action modules, not
//! one, because a plain `action` component only ever renders a bare
//! button (see `render::action_html`), with nowhere to put the typed
//! fields (schema name, mode, password/connection string) this needs:
//!
//! - [`NewWorkspaceForm`] is a `dynamic_content` module: it renders a
//!   real `<form>` with those fields, `action`-ing at a sibling
//!   `action ... calls create_workspace` component's own `/c/<idx>/run`
//!   route (kept out of sight with `attrs (style: "display: none")` —
//!   it exists only to be that POST target, not to show its own
//!   default button too). `action_idx` in its config names which
//!   component index that sibling sits at, since a `dynamic_content`
//!   module has no framework-given way to discover another component's
//!   index by kind.
//! - [`CreateWorkspace`] is that target: reads the posted fields and
//!   calls the matching `control::create_workspace_*` — no request row
//!   ever gets written (unlike the App Builder's "New App" flow),
//!   which matters here specifically because "attach to an existing
//!   schema" mode carries a superuser-capable Postgres connection
//!   string the caller typed in. That string must never be persisted,
//!   even transiently — an entity-bound `Form` (the only other way
//!   this framework accepts typed input) would `INSERT` it into a row
//!   before anything could read it back out, which is exactly the
//!   "even briefly" this design avoids. Here it only ever lives in
//!   this one request's in-memory parameter map (`ActionContext::values`)
//!   and is dropped the moment `run` returns — never written to a
//!   table, and (since every error path below is a plain
//!   `anyhow::bail!`/`?` on `control`'s own already-safe error
//!   messages, never the connection error's own possibly-detailed
//!   source) never echoed into an error message either, which matters
//!   because `server::run_action` puts that message straight into a
//!   redirect URL on failure.

use crate::actions::{ActionContext, BoxFuture, ServerAction};
use crate::html::escape;

pub struct NewWorkspaceForm;

impl ServerAction for NewWorkspaceForm {
    fn name(&self) -> &'static str {
        "new_workspace_form"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let action_idx: u64 = ctx
                .config
                .get("action_idx")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("new_workspace_form needs an (action_idx: <n>) config"))?;
            let action_url = format!(
                "/{}/{}/{}/c/{action_idx}/run",
                crate::instance::APP_BUILDER_WORKSPACE_SLUG,
                crate::instance::APP_BUILDER_APP_SLUG,
                escape(&ctx.page.name),
            );
            Ok(format!(
                r#"<form method="post" action="{action_url}" class="pgapp-form">
<div class="pgapp-field"><label class="pgapp-label">Schema name</label>
<input class="pgapp-input" type="text" name="schema_name" required></div>
<div class="pgapp-field"><label class="pgapp-label">Workspace slug (optional — defaults to the schema name)</label>
<input class="pgapp-input" type="text" name="slug"></div>
<div class="pgapp-field">
<label class="pgapp-label"><input type="radio" name="mode" value="new" checked onchange="pgappCreateWorkspaceSync()"> Create a new schema</label>
<label class="pgapp-label"><input type="radio" name="mode" value="existing" onchange="pgappCreateWorkspaceSync()"> Attach to an existing schema</label>
</div>
<div class="pgapp-field" id="pgapp-cw-new"><label class="pgapp-label">Password for the new role</label>
<input class="pgapp-input" type="password" name="password"></div>
<div class="pgapp-field" id="pgapp-cw-existing" style="display: none"><label class="pgapp-label">Superuser-capable connection string (used once to grant access, never stored)</label>
<input class="pgapp-input" type="text" name="grantor_conn" placeholder="postgres://postgres:postgres@localhost:5432/postgres"></div>
<button class="pgapp-btn pgapp-btn-primary" type="submit">Create Workspace</button>
</form>
<script>
function pgappCreateWorkspaceSync() {{
  var mode = document.querySelector('input[name="mode"]:checked').value;
  document.getElementById('pgapp-cw-new').style.display = mode === 'new' ? '' : 'none';
  document.getElementById('pgapp-cw-existing').style.display = mode === 'existing' ? '' : 'none';
}}
</script>"#
            ))
        })
    }
}

pub struct CreateWorkspace;

impl ServerAction for CreateWorkspace {
    fn name(&self) -> &'static str {
        "create_workspace"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let schema_name = ctx.values.get("schema_name").map(|s| s.trim()).unwrap_or("");
            if schema_name.is_empty() {
                anyhow::bail!("a schema name is required");
            }
            let slug = ctx
                .values
                .get("slug")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(schema_name)
                .to_string();

            match ctx.values.get("mode").map(|s| s.as_str()) {
                Some("existing") => {
                    let conn = ctx.values.get("grantor_conn").map(|s| s.as_str()).unwrap_or("");
                    if conn.is_empty() {
                        anyhow::bail!("a superuser-capable connection string is required to attach to an existing schema");
                    }
                    crate::control::create_workspace_existing_schema(ctx.pool, &slug, schema_name, conn).await?;
                }
                _ => {
                    let password = ctx.values.get("password").map(|s| s.as_str()).unwrap_or("");
                    if password.is_empty() {
                        anyhow::bail!("a password for the new role is required");
                    }
                    crate::control::create_workspace_new_schema(ctx.pool, &slug, schema_name, password).await?;
                }
            }
            Ok(format!("Workspace '{slug}' created."))
        })
    }
}
