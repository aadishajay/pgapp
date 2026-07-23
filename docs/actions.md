[← Back to README](../README.md)

# Server-side & dynamic actions

- [Server-side actions](#server-side-actions)
- [Dynamic actions](#dynamic-actions)
- [Ajax callback](#ajax-callback)

## Server-side actions

The PL/SQL analog: named Rust modules under `src/actions/`
(`ServerAction` trait: `name()` + async `run(ctx) -> Result<String>`),
invoked via `action "Label" calls <module> (config...)` — a button
posting to `/:page/c/:idx/run`, gated by the page's `requires:` role.
`ActionContext` carries the pool, app, page, config, and request params.
Ships seven modules:

- **`run_query`** — executes a named query raw (may be a plain
  `UPDATE`/`DELETE`/`INSERT`, with no `RETURNING`); binds are still
  `:name` markers, never interpolation. A query meant only for this
  (never a report/region/chart/LOV source) needs no `RETURNING`
  clause — `meta::compile_named_query`'s own load-time bind-type check
  describes the *bare* SQL, not the `select to_jsonb(t) from (<sql>) as
  t` shape `query_engine.rs`/`call_function` execute against (which
  only works for SELECT-shaped queries; Postgres rejects a bare DML
  statement inside a `FROM (...) AS t` subquery outright).
- **`call_function`** — calls a plain PL/pgSQL function (`select
  my_function()` as the query's SQL) and shows back whatever the
  function itself returns; `raise exception '...'` inside it becomes
  the error banner verbatim (`actions::clean_db_error`). The function
  must already exist when the app is (first) synced/reloaded.
- **`log_values`** — trivial demo, logs the parameter map.
- **`http_request`** — calls an external REST API; the one action
  module that leaves Postgres. Any method (`GET`/`POST`/.../anything
  `reqwest::Method::from_bytes` accepts), a request body with a
  `content_type`, and `auth: "none" | "basic" | "bearer" |
  "api_key_header" | "api_key_query"`. Since the config grammar is
  flat string key/value pairs only (no nested objects), multiple
  extra headers pack into one `headers: "Name: Value; Name2: Value2"`
  string, parsed at runtime rather than by markup.rs. `{{item}}` in
  `url`/`body`/`headers`/`token`/`username`/`password`/`key_value`
  interpolates that page item's current value — plain string
  substitution, not SQL-bind casting, since it has nothing to do with
  Postgres:

  ```text
  action "Notify webhook" calls http_request (
    method: "POST",
    url: "https://hooks.example.com/tickets/{{id}}",
    body: "{\"status\": \"{{status}}\"}",
    headers: "X-Source: pgapp",
    auth: "bearer",
    token: "abc123"
  )
  ```

  Not covered: full OAuth2 grant flows (client-credentials, token
  refresh) — those need a token cache with its own lifetime, a bigger
  feature than one action module; `bearer` still works with a token
  you already have in hand. A non-2xx response or a network failure
  (bad host, timeout — default 10s, override with `timeout_secs`)
  becomes the page's error banner, same as a PL/pgSQL exception would.
  A `collection: "name"` config captures the (JSON) response body into
  a collection instead of just echoing it back — see [Collections](./markup.md#collections).
  A fixed credential (an API key, a service token) that isn't
  user-typed belongs in `{{secret.<name>}}` instead of a literal in the
  markup — see [Secrets](./secrets.md).
- **`send_email`** — Oracle APEX's "Send Email" process type, sent over
  SMTP (via `lettre`, STARTTLS on `smtp_port` — default 587). `to` may
  be a comma-separated list; `{{item}}` interpolates in `to`/`from`/
  `subject`/`body`, same convention as `http_request`, and
  `{{secret.<name>}}` works in any field — typically `smtp_username`/
  `smtp_password`, so credentials never sit in plaintext in the markup:

  ```text
  action "Notify customer" calls send_email (
    to: "{{customer_email}}",
    from: "support@example.com",
    subject: "Ticket #{{id}} updated",
    body: "Your ticket status is now {{status}}.",
    smtp_host: "smtp.example.com",
    smtp_port: "587",
    smtp_username: "{{secret.smtp_username}}",
    smtp_password: "{{secret.smtp_password}}"
  )
  ```

  `content_type: "html"` sends the body as `text/html` instead of the
  default `text/plain`. Implicit-TLS-only providers (port 465) aren't
  covered — only STARTTLS.
- **`set_session_state`** / **`clear_session_state`** — an
  approximation of APEX's per-item session state, a less natural fit
  here than the other modules since pgapp has no server-tracked item
  state outside the database. Writes/deletes a single row in
  `pgapp_meta.collections` under `name:` (scoped to the current
  caller), readable back with `entity "x" from collection "<name>" {
  field value: text }` like any other collection — no new read
  mechanism. `{{item}}` in `value` interpolates that page item's
  current value, same convention as every other action module:

  ```text
  action "Save filter" calls set_session_state (
    name: "selected_status",
    value: "{{status}}"
  )
  action "Clear filter" calls clear_session_state (
    name: "selected_status"
  )
  ```

Rust and PL/pgSQL aren't a migration path away from each other: HTTP
calls belong in Rust (`http_request`) since Postgres has no native
notion of the outside world; row-level logic already living beside the
data is cheaper as a function via `call_function`. `run_query`/
`call_function` share the exact same `:name` → schema-typed bind
compilation as every other named query. No `apex_util`-style grab-bag
package is shipped — `clean_db_error` + `raise exception` covers the
one thing among the SQL-side actions that genuinely generalizes.

## Dynamic actions

Declarative client-side behavior, APEX-style:

```text
on change of priority {
  set urgent to "pgapp.getItem('priority') === 'High'"
}
on change of urgent {
  toggle notes when "pgapp.getItem('urgent') === 'true'"
}
on change of agent {
  refresh agent_load
}
```

Ops: `show`/`hide <item>`, `toggle <item> when "<js expr>"`, `set
<item> to "<js expr>"` (may call `pgapp.getItem`), `refresh <query>`
(re-fetches one region via `GET /:workspace/:app/:page/region/:query`, sending current
item values as query params). Dispatched by the DB-stored runtime.js;
`setItem` fires `change` events so actions chain (depth-guarded).

## Ajax callback

`call <action> (config...) into <target>` — the one op that reaches
the server without a page reload, APEX's "ajax callback" process type:

```text
on change of trigger_val {
  call log_values into result_val
}
on click of refresh_button {
  call run_query (query: "bump_counter") into widget_count
}
```

Posts to `POST /:workspace/:app/:page/c/:idx/call/:op_idx` (`idx`
addresses the `DynamicAction` component, `op_idx` which of its `ops`
entries — one dynamic action can hold more than one `call`) with the
page's current item values as the body, and runs the exact same
`ActionContext`/module dispatch `action`/`button calls` use — any
registered module works here, validated against the action registry at
sync time just like they are. The response is JSON (`{"ok", "result"}`
or `{"ok": false, "error"}`), not a redirect, since the caller is
client-side JS (`pgapp.runDynamicActionCall` in `/runtime.js`), not a
full-page form POST. `target` is resolved client-side: if it names a
region/query currently on the page, that region gets refreshed (the
callback's own result string is just the trigger, not injected
directly into the region); otherwise `target` is treated as an item and
set to the result string via `pgapp.setItem`.

---

Next: [Authentication](./authentication.md) · [Secrets](./secrets.md) · [App Builder](./app-builder.md)
