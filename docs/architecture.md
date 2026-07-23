[← Back to README](../README.md)

# Architecture

- [The pipeline](#the-pipeline)
- [Source layout](#source-layout)
- [Runtime JS](#runtime-js)
- [Hot reload](#hot-reload)
- [Routes](#routes)
- [Multi-app routing](#multi-app-routing)

## The pipeline

```
 .pgapp markup file
        │  markup::parse_app
        ▼
    AppDef (in memory)
        │  meta::sync_app (validates entity/page/query refs by name,
        │                  item kinds against the item_types registry)
        ▼
 pgapp_meta.* tables  ──creates──▶  <workspace_schema>.<table> (the real data table)
        │  meta::load_app (reloads from the DB, not from AppDef)
        ▼
    RuntimeApp { pages: Vec<RuntimePage { components: Vec<RuntimeComponent> }> }
        │
        ▼
   Axum router (src/server.rs) ── generic, metadata-driven CRUD + JSON
```

Every SQL identifier (table/column) used at request time comes from
`pgapp_meta`, populated only from markup identifiers the lexer already
restricted to `[A-Za-z_][A-Za-z0-9_]*` — safe to splice into generated
SQL. All user *values* are always bind parameters, cast to the field's
declared type. A page's components live in one generic table
(`pgapp_meta.components (..., kind, config jsonb)`) — the same
generic-JSON-config pattern item types use (see [Forms](./forms.md)), extended
to whole components, so a new component kind never needs a schema
change.

## Source layout

```
src/
  lib.rs              the library crate every binary below depends on
  main.rs             the `pgapp` server binary
  bin/cargo-pgapp.rs  the `cargo-pgapp` binary (`cargo pgapp` -> scaffold::run)
  scaffold.rs         `pgapp new`/`pgapp create` (see "Scaffolding a new app")
  markup.rs           lexer + parser: .pgapp text -> model::AppDef (or a Fragment)
  source.rs           loads a file/dir into one AppDef, or a dir-of-dirs into a workspace of several
  control.rs          pgapp_control.apps/workspaces: the durable registry (see "Multi-app routing", "Instance mode")
  instance.rs         instance file, pgapp_admin role, CLI-admin auth (see "Instance mode")
  model.rs            parsed-markup types (AppDef, PageDef, ComponentDef, FieldItem, ...)
  html.rs             shared escape/js_escape/url_encode helpers
  theme.rs            theme.css/theme.json loading (see "Theming")
  icons.rs            icon pack loading (see "Icons")
  chart_lib.rs        chart library loading (see "Charts")
  runtime.js          seed content for the DB-stored JS runtime
  item_types.rs       the ItemType trait + registry() (see "Item types")
  item_types/         one file per component: text, readonly, checkbox, radio, popup, slider
  actions.rs          the ServerAction trait + registry() (see "Server-side actions")
  actions/            one file per module: run_query, call_function, log_values, http_request, create_app (see "App Builder")
  meta.rs             module root: ensure_schema + re-exports
  meta/
    types.rs          the runtime model (RuntimeApp, RuntimePage, RuntimeComponent, Chrome, ...)
    sync.rs           AppDef -> pgapp_meta.* (+ physical data tables)
    load.rs           pgapp_meta.* -> RuntimeApp, compile_named_query
  server.rs           module root: AppState/AppEntry, /:workspace/:app routes, HTTP handlers, pagination
  server/
    query_engine.rs   named-query execution (+ paginated), bind context, LOV/region resolution
  render.rs           HTML generation; delegates field widgets to item_types, charts to chart_lib
  page_reorder.rs     splices a page's components (reorder/add/delete/edit-label/edit-columns) in its own .pgapp file (see "App Builder")
themes/               pluggable design systems (see "Theming")
icons/                pluggable icon packs: fontawesome/, material/
chart-libs/           pluggable chart libraries: canvas-bars/
```

## Runtime JS

`GET /:workspace/:app/runtime.js` is a row in `pgapp_meta.app_runtime_js`, seeded from
`src/runtime.js` on first sync and left alone after — editable in the
database without touching the binary. Defines
`window.pgapp.getItem(name)`/`.setItem(name, value)`, working the same
regardless of whether `name` is a plain input, checkbox, radio group,
or popup's hidden input. Also owns `pgapp.alert`/`pgapp.confirm`
(promise-based, themed `.pgapp-dialog-*` overlays instead of native
browser dialogs — any `<form data-pgapp-confirm="...">` uses this
automatically), the dynamic-action dispatcher, and the popup search
filter.

Since it's a DB row, editing it directly takes effect on the next
request — no restart. To pick up a newer *built-in* default after
changing `src/runtime.js`, delete the app's `pgapp_meta.app_runtime_js`
row and hit that app's `/admin/reload` (sync only seeds that row once).

## Hot reload

`GET`/`POST /:workspace/:app/admin/reload` re-syncs that app's markup file into
`pgapp_meta` and reloads it in place — no restart, and no effect on any
other app sharing the process. `AppState` holds what's shared and can't
change without a rebuild (`pool`, `item_types`, `actions`, the
`apps: HashMap<slug, AppEntry>` registry itself); each `AppEntry` splits
off everything markup-derived (`app`, `theme`, `runtime_js`, `icons`,
`chart_lib`) behind its own `RwLock<Arc<AppData>>`. Each request
snapshots that one app's data once at the top, so a concurrent reload
can't mix a new `RuntimeApp` with a stale `Theme`. A failed reload (bad
markup) never swaps in — the old snapshot keeps serving, and the error
shows on the reload page. Gated to the `admin` role when auth is enabled.

The page's markup editor (`textarea.pgapp-source-textarea`, upgraded
in-place by `pgappUpgradeCodeEditor` in `src/runtime.js`) is a small
IDE, not a plain text box: a line-number gutter, syntax highlighting
(comments/strings/keywords/numbers via a hand-rolled tokenizer —
`pgappTokenizeMarkup`, no external JS library), Tab-to-indent,
Enter auto-indent, and grammar-aware autosuggestions
(`pgappAutocompleteContext`) that pop up as you type — the full
keyword list generally, narrowed to just the five field types after
`field x: ` and just the item kinds after `as ` — accepted with
Tab/Enter/click, navigated with the arrow keys, dismissed with Escape.

For a single-file app this editor sits directly on the page; for a
directory app, `/admin/reload` instead renders a VS-Code-style
expandable file tree (`bindFileTree` in `src/runtime.js`, backed by
`GET /:workspace/:app/admin/files-list` and
`GET`/`POST /:workspace/:app/admin/files/*path`) — pick any `.pgapp`
file to load it into the same editor, edit, and "Save & reload" writes
that one file to disk and re-syncs the whole app. This is raw file
editing only: the structured entity/query/page panels in the [App
Builder](./app-builder.md) are still single-file-only (splicing a change across a
directory app's files isn't implemented), so a directory app's
day-to-day editing goes through this tree instead.

Not covered: new item types/actions, or the routing table itself, are
still Rust code — those need `cargo build` + restart. Markup changes
(new page/field/entity, a changed `theme:`, a new dynamic action) don't.

## Routes

Every route lives under `/:workspace/:app` — the workspace an app is
registered into, plus its own URL slug (see [Multi-app routing](#multi-app-routing)). Since
the slug only has to be unique *within* a workspace, not instance-wide,
two workspaces can each register an app called `reports` without
colliding. On startup it prints each app's full URL and its pages'
component kinds, e.g.
`http://127.0.0.1:8080/erp/todo/Tasks  [report, form, text, region]`.

- `GET  /`                                   — one app: redirects there; several: a plain list of them
- `GET  /:workspace/:app`                               — redirects to the app's first page
- `GET  /:workspace/:app/:page`                         — renders every component on the page, in order
- `POST /:workspace/:app/:page/c/:idx/create`           — create a row (`Form`/`EditableTable` only, by component index)
- `POST /:workspace/:app/:page/c/:idx/update/:id`       — update a row
- `POST /:workspace/:app/:page/c/:idx/delete/:id`       — delete a row
- `GET  /:workspace/:app/api/:entity`                   — JSON list for that entity
- `POST /:workspace/:app/uploads`                       — multipart upload for `file_browse` fields; returns `{"id", "filename"}` JSON
- `GET  /:workspace/:app/uploads/:id`                   — streams a previously uploaded file back
- `GET  /:workspace/:app/:page/region/:query`           — one region re-rendered as a fragment (dynamic-action refresh)
- `POST /:workspace/:app/:page/c/:idx/run`              — run an `action` component's server-side module
- `POST /:workspace/:app/:page/c/:idx/call/:op_idx`     — the ajax callback: run a dynamic action's `call` op, JSON response
- `POST /:workspace/:app/:page/c/:idx/views` (+ delete) — save / delete a report's saved view
- `GET  /:workspace/:app/login` / `POST /:workspace/:app/login`    — sign-in page (or first-run admin setup) — apps with `auth { }` only
- `POST /:workspace/:app/setup`                         — one-time admin bootstrap; refuses once any user exists
- `POST /:workspace/:app/logout`                        — deletes the server-side session
- `GET  /:workspace/:app/users` (+ create/delete POSTs) — built-in user management, admin role only
- `GET  /:workspace/:app/admin/reload` (+ POST)         — re-syncs that app's markup file into `pgapp_meta` and reloads it, no restart
- `GET  /:workspace/:app/admin/files-list`               — every `.pgapp` file in a directory app, for its file-tree editor
- `GET  /:workspace/:app/admin/files/*path` (+ POST)     — read/write one file's raw content by path within that app's own directory
- `GET  /:workspace/:app/admin/pages-list`              — every page name in that app, for the App Builder's "Target page" dropdown
- `POST /:workspace/:app/admin/pages/:page/reorder`     — the App Builder's drag-and-drop save
- `POST /:workspace/:app/admin/pages/add`                          — the App Builder's "Add Page"
- `POST /:workspace/:app/admin/pages/:page/rename`                 — the App Builder's "Rename page"
- `POST /:workspace/:app/admin/pages/:page/delete`                 — the App Builder's "Delete page"
- `POST /:workspace/:app/admin/pages/:page/components/add`         — the App Builder's "Add Component" (raw markup, any kind)
- `GET  /:workspace/:app/admin/pages/:page/components/:idx/source` — a component's exact current markup, for the App Builder's "Edit" panel
- `POST /:workspace/:app/admin/pages/:page/components/:idx/edit`   — the App Builder's full-property "Edit"
- `POST /:workspace/:app/admin/pages/:page/components/:idx/delete` — the App Builder's per-row "Delete"
- `POST /pgapp/builder/admin/apps/create-pending`                  — the App Builder's "New App" processing step
- `GET  /:workspace/:app/runtime.js`                    — the DB-stored `pgapp` JS runtime
- `GET  /:workspace/:app/chart-lib.js`                  — the active pluggable chart library's JS (404 when `chart_lib` is the built-in `inline`)

A `Form` switches into edit mode via `?edit_<n>=<id>` (`<n>` = its
0-based position on the page); a `Report`'s pagination uses
`?r<n>_after=`/`?r<n>_before=` (entity-backed) or `?r<n>_page=`
(query-sourced) the same way. See [App Builder](./app-builder.md) for the full walk-through
of what each admin route is for.

## Multi-app routing

One process, one shared `PgPool`, any number of apps — closer to how
Oracle APEX actually pools connections (one pool per workspace, shared
by every application in it) than to a separate server per app. Every
app keeps its own tables (in its own workspace's schema), sessions, and
users; only the connection pool and Rust process are shared.

The pool defaults to **20 connections** — comfortably above a
handful of toy connections without assuming "bigger is always faster"
(a Postgres backend is a full process, not a lightweight thread, so a
few dozen is already generous for one server); override with
`PGAPP_MAX_CONNECTIONS`. Same default and override for the
`pgapp_admin` connection `pgapp run` serves through.

A `tower::limit::ConcurrencyLimitLayer` wraps the whole router, capped
at the same number as the pool: since almost every route needs a
connection to do anything, admitting more requests than the pool can
serve in parallel doesn't add throughput, just piles up in-flight work
that queues for the same fixed number of connections anyway — with
each one holding its own row buffers and rendered-HTML strings in the
meantime. Load-testing confirmed this actually helps latency under a
big concurrent spike, not just tidiness (p50 dropped ~40% at 1,000
concurrent requests in one run). It doesn't fully bound memory though
— hyper still accepts and buffers every incoming TCP connection before
the limiter's gate, so a connection flood is still a job for whatever
sits in front of this in production (the same reverse proxy handling
TLS termination is the natural place for connection-level limits too).

**What's registered, not what's on the command line, decides what's
served.** `pgapp_control.apps` (a schema of its own — pgapp managing
itself, distinct from `pgapp_meta`'s per-app metadata) is the durable
list of `(slug, markup_path, workspace_id, data_schema, enabled)` rows,
unique on `(workspace_id, slug)` rather than on `slug` alone — a slug
only has to be unique *within* its own workspace, matching the URL
scheme above. `pgapp run <path> --workspace <slug>`
registers (or re-points) one slug into that workspace and then serves
*every enabled row across every workspace in the instance*, not just
that one — so pointing the server at a new app one run adds it
alongside whatever was already registered, without needing to name
every app again each time. There's no register-only command, though —
`run` always binds a port and blocks — so laying out several apps that
don't already live under one directory (see below for the case where
they do) means registering them one at a time, stopping each before
starting the next since two `run`s can't hold the same port at once,
and leaving only the *last* one up (it serves everything registered so
far, including the ones from the runs you already killed):

```bash
pgapp run helpdesk.pgapp --workspace erp &   # registers "helpdesk", starts serving it
sleep 1 && kill %1                           # stop it — just needed the registration
pgapp run inventory.pgapp --workspace erp    # registers "inventory" too, then serves BOTH — leave this one running
pgapp app list   # slug  enabled/disabled  name  workspace=...  schema=...  markup_path, one per line
pgapp app destroy inventory --soft   # disables it — helpdesk keeps serving
```

If the same slug happens to be registered in more than one workspace,
`app destroy <slug>` needs `--workspace <slug>` to say which one — it
errors out naming every workspace that slug matches rather than
guessing. `secret ... --app <slug>` has no `--workspace` fallback (it's
mutually exclusive with `--app`), so an ambiguous slug there just
errors — give the app a workspace-unique slug if you need to
secret-scope it.

If the apps you're laying out can be arranged under one parent
directory, there's a way to skip the run+kill dance above entirely —
a single `pgapp run` invocation can also register several apps at once:
if the given path is a directory containing only subdirectories (no
loose `.pgapp` files of its own), each subdirectory is loaded as an
independent app, slugged from its own declared name — `pgapp run
workspace-dir/ --workspace erp` where
`workspace-dir/helpdesk/` and `workspace-dir/inventory/` each look like
a normal single-app directory (see [Getting started](./getting-started.md#scaffolding-a-new-app)), and both
land in the same `erp` Postgres workspace/schema. A directory with any
loose `.pgapp` file directly inside it is still just one app, exactly
as before — this only kicks in for a directory of nothing but
subdirectories. (Note this is a different sense of "workspace" than the
Postgres-schema one above — it just means "a directory that declares
several apps at once"; which apps land in which *schema* is still
whatever `--workspace <slug>` says.)

Sessions are app-scoped even though the cookie name is shared: the
`Set-Cookie` carries `Path=/<workspace>/<slug>`, so a browser never
sends one app's token to another's routes, and `pgapp_meta.sessions`/
`.users` are looked up by `app_id` regardless.

---

Next: [App Builder](./app-builder.md) · [Getting started](./getting-started.md)
