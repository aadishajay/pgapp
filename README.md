# pgapp

An Oracle APEX-inspired, no/low-code application framework built on
Postgres, written in Rust.

## Quickstart

All you need is Postgres reachable somewhere (any role that can
`CREATE DATABASE`) and Rust installed — pgapp creates its own database
and a working starter app for you.

```bash
cargo run -- create
```

Answer the four prompts (app name, database URL — defaults to
`postgres://postgres:postgres@localhost:5432/pgapp` and is **created
automatically if it doesn't exist yet** — theme, single file or
directory). It syncs the new app into Postgres immediately and prints
the exact next command:

```bash
cargo run -- <generated-file>.pgapp
```

It prints the app's URL — `http://127.0.0.1:8080/<slug>` — and opening
the bare `http://127.0.0.1:8080` redirects there too. No manual
`createdb`, no separate migration step, nothing to hand-edit first.

For scripts/CI, skip the prompts entirely: `cargo run -- new <AppName>`
(see "Scaffolding a new app" below). To try the richer bundled demo
instead of a blank scaffold: `cargo run -- examples/helpdesk.pgapp`
(needs `examples/helpdesk_functions.sql` run first — see its header
comment).

Running against the same database again with a *different* `.pgapp`
path adds a second app alongside the first — see "Multi-app routing"
below.

## Idea

- **In-database metadata**: apps, entities, fields, pages, and
  components are rows in Postgres (`pgapp_meta.*`); the server always
  serves off the database, never off whatever was last parsed.
- **A textual markup language** (`.pgapp` files, APEX-flavored) authors
  an app; it's synced into metadata at startup and again on that
  app's `/admin/reload` — no restart needed to pick up a change.
- **Composable pages**: a page is just an ordered list of independent
  components (`Report`, `Form`, `EditableTable`, `Chart`, `Text`,
  `Link`, `Region`, `Action`) — any combination, any number, one page.
- **Pluggable everything**: theme (CSS only), item types (form
  widgets), charts, icons — each is a small file dropped into its own
  directory, selected by name in the markup, no framework code touched.
- **Named queries**: reusable SQL, bind-typed automatically from the
  schema, shared across LOVs, regions, charts, report sources, and
  whole read-only entities.
- **Server-side actions** (the PL/SQL analog): named Rust modules, or
  a plain PL/pgSQL function call, invoked from a button.
- **Dynamic actions**: declarative client-side behavior
  (`on change of x { show/hide/toggle/set/refresh }`).
- **Interactive reports**: search, per-column filter, and named saved
  views (private or public), all in metadata.
- **Auth**: an `auth { }` block puts the app behind argon2-hashed
  logins and server-side sessions; `requires: <role>` gates pages.
- **A DB-stored JS runtime**: `/:app/runtime.js` is a metadata row, not
  a static file — editable without touching the binary.
- **Rust instead of PostgREST**: one Axum binary owns routing and
  builds parameterized SQL from metadata directly.

This is deliberately the *smallest end-to-end loop*, not the whole
framework: one Postgres schema (single workspace), a handful of field
types. It exists to prove the architecture before building the bigger
pieces (drag-and-drop builder UI, multi-step flows).

## Markup

```text
app "Todo" {
  # optional app settings (all default sensibly when omitted):
  # theme: vivid          - themes/<name>/ (default: shadcn)
  # icons: fontawesome    - icons/<name>/ (default: builtin inline SVG)
  # chart_lib: canvas_bars - chart-libs/<name>/ (default: inline SVG)
  # auth { }              - require login on every page

  header {
    text "pgapp Todo Demo"
  }

  footer {
    text "Built with pgapp - a Postgres-native no/low-code framework."
    link "About" -> page About
  }

  nav {
    item "Tasks" -> page Tasks
    item "Open" -> page OpenTasks
    item "Quick edit" -> page QuickEdit
    item "More" {
      item "About" -> page About
    }
  }

  # App-scoped: visible from every page's LOVs/regions/charts.
  query assignees {
    sql: "select distinct assignee as value from pgapp_data.todo_tasks where assignee is not null order by 1"
  }
  query open {
    sql: "select id, title, priority, assignee from pgapp_data.todo_tasks where done = false order by id"
  }
  query by_priority {
    sql: "select priority as label, count(*) as value from pgapp_data.todo_tasks group by priority order by 1"
  }

  entity "tasks" {
    field id: id
    field title: text required
    field priority: text default Medium
    field done: boolean default false
    field assignee: text
    field notes: text
    field estimate_hours: integer default 4
    field created_at: timestamp default now
  }

  # A page can hold any number of components. A Report + Form on the
  # same page (as here) is the classic list+edit CRUD pattern.
  page "Tasks" {
    report "Tasks" of tasks {
      columns: title, priority, done, estimate_hours, created_at
      link: title -> page TaskDetail (priority: priority)
      page_size: 5
    }

    form "Add / edit task" of tasks {
      fields: title, priority, done, assignee, notes, estimate_hours
      item priority as radio ("Low", "Medium", "High")
      item assignee as popup from query assignees
      item notes as readonly
      item estimate_hours as slider (min: "0", max: "40", step: "1")
    }

    text "Click a title to see its detail page, or a row's edit icon to update it above."

    # Page-scoped: only this page's items/LOVs can see "recent".
    query recent {
      sql: "select id, title, priority, done from pgapp_data.todo_tasks order by id desc limit 5"
    }
    region "Recently added" from query recent
  }

  page "TaskDetail" {
    query siblings {
      sql: "select id, title from pgapp_data.todo_tasks
            where priority = :priority and id != :id
            order by id"
    }
    region "Other tasks with the same priority" from query siblings
  }

  # A Report sourced from a query instead of the entity table. No Form
  # here, so it's read-only.
  page "OpenTasks" {
    report "Open tasks" of tasks {
      source: query open
      columns: title, priority, assignee
      page_size: 3
    }
  }

  # An EditableTable: every row is inline-editable, no separate list/edit split.
  page "QuickEdit" {
    editable_table "Quick edit" of tasks {
      columns: title, priority, done
    }
  }

  page "About" {
    text "pgapp is an Oracle APEX-inspired no/low-code framework built on Postgres."
    chart "Tasks by priority" from query by_priority {
      type: bar
      x: label
      y: value
    }
    link "Back to tasks" -> page Tasks
  }
}
```

- `header { }` / `footer { }` (optional) — app-wide chrome, restricted
  to `text`/`link`/`region`.
- `nav { }` (optional) — the nav bar; each `item "Label"` is a leaf
  (`-> page <Name>`) or a group (`{ nested items }`). A leaf whose
  target page has `requires: <role>` only shows for a user holding
  that role; a group left with no visible children disappears too.
- `entity` blocks describe a table: each `field` has a type, optionally
  `required` and/or a `default`.
- `page "Name" { ... }` — a name plus an ordered list of components
  (see "Components"), and optionally `requires: <role>`.
- `theme:`/`icons:`/`chart_lib:`/`auth { }` (optional, app level) — part
  of the app definition, synced into `pgapp_meta.apps`; no environment
  variables involved.
- Anything targeting a page/entity/query by name uses a bare
  identifier (`[A-Za-z_][A-Za-z0-9_]*`), not a quoted string — the same
  charset that's safe to splice into SQL as an identifier.

See `src/markup.rs` for the full grammar. `examples/helpdesk.pgapp` is
a richer demo — two entities, a chart dashboard, both pagination modes,
auth, every built-in item type — with seed data
(`examples/helpdesk_seed.sql`), PL/pgSQL functions
(`examples/helpdesk_functions.sql`, run before first sync), a
feature-by-feature tour in `marketing/index.html`, and the `vivid`
theme.

### One file, or a directory

An app is authored as either a single `.pgapp` file or a **directory**
of them — same grammar, zero refactoring to move between the two.
Every `.pgapp` file under the directory (recursively) merges into one
app by name (Terraform-shaped, no `include`, no import graph):

- Exactly one file declares the `app "..." { }` block (settings, auth,
  nav, header/footer).
- Every other file is a *fragment*: top-level `entity`/`page`/`query`
  blocks, referencing each other by name exactly as in one file.
- The same name declared in two files is a startup error naming both.

`examples/helpdesk-modular/` is the helpdesk app split this way — run
it with `cargo run -- examples/helpdesk-modular`; it syncs to the same
metadata as the single-file version.

## Components

A page's body is `Vec<ComponentDef>` (`src/model.rs`) — there's no
fixed "page kind." Eight kinds:

- **`report "Title" of <entity> { ... }`** — read-only, paginated
  table. `columns`; `source: query <name>` sources rows from a query
  instead (writes still target the entity by id); `link: <field> ->
  page <Name> (extra: param, ...)` links a column, forwarding the row's
  id plus extra params; `page_size` (default 20). A sibling `Form` for
  the same entity on the same page gets automatic Edit/Delete actions.
- **`form "Title" of <entity> { ... }`** — create/edit form. `fields`
  lists writable columns; `item <field> [as <kind> [(...)]] [attrs
  (...)]` picks a widget and/or sets `id`/`class`/attributes on that
  field's wrapper (at least one of the two required). Blank by default;
  `?edit_<n>=<id>` switches it into edit mode with a Delete button.
- **`editable_table "Title" of <entity> { ... }`** — every row rendered
  as its own inline-editable form, plus an "add new" row. Not
  paginated. Same `item` syntax as `form`.
- **`chart "Title" from query <name> { type: bar|line|area|scatter|pie|donut x: <col> y: <col> }`**
  — see "Charts".
- **`text "..."`** — static text.
- **`link "Label" -> page <Name>`** — a link to another page.
- **`region "Label" from query <name> { columns: ... }`** — a query's
  rows as a plain, non-paginated table; `columns:` narrows/orders which
  result columns show (default: every column, alphabetically).
- **`action "Label" calls <module> (config...)`** — a button running a
  server-side action module; see "Server-side actions".
- **`on <event> of <item> { ... }`** — a client-side dynamic action;
  see "Dynamic actions".
- Any component can end with **`attrs (id: "...", class: "...",
  data_foo: "bar")`** to set a custom id/extra class/attribute on its
  wrapper tag — see "Theming".

## Pagination

Backend-optimized to avoid `COUNT(*)` and `OFFSET`-skipping a large
table (`server::fetch_report_rows` / `query_engine::run_named_query_page`):

- **Entity-backed** (no `source:`): keyset ("seek") pagination on `id`
  — `?r<n>_after=<id>` / `?r<n>_before=<id>`. Zero extra queries,
  stays cheap regardless of table size.
- **Query-sourced** (`source: query <name>`): no assumed sort key, so
  it falls back to `?r<n>_page=<n>` (`OFFSET`), still avoiding
  `COUNT(*)` by fetching one extra row.

## Report search & saved views

Every `Report` gets an interactive toolbar: **search** (`r<n>_q`,
case-insensitive substring across visible columns), a **column filter**
(`r<n>_col`/`r<n>_val`, column validated against the report's own
list), and **saved views** (`pgapp_meta.report_views`) — private by
default or public via checkbox, rendered as chips, deletable by their
owner or an admin. All filter values are bind parameters; filters
compose with both pagination modes.

## Query-backed entities

`entity "name" from query <name> { field ... }` — a **read-only**
entity backed by a named query instead of a physical table (the APEX
"view" pattern). No table created; binding a `form`/`editable_table` to
it is a sync-time error; reports over it paginate by `OFFSET`.

## Deployment checks

On every sync, each table-backed entity's physical table is verified
against its declared fields via `information_schema`: a type mismatch
or missing column fails startup naming every problem (rather than a
confusing cast error at request time). Extra columns are only warned
about. pgapp adds columns but never changes or drops them.

## Server-side actions

The PL/SQL analog: named Rust modules under `src/actions/`
(`ServerAction` trait: `name()` + async `run(ctx) -> Result<String>`),
invoked via `action "Label" calls <module> (config...)` — a button
posting to `/:page/c/:idx/run`, gated by the page's `requires:` role.
`ActionContext` carries the pool, app, page, config, and request params.
Ships three modules:

- **`run_query`** — executes a named query raw (may be a plain
  `UPDATE`/`DELETE`/`INSERT`); binds are still `:name` markers, never
  interpolation.
- **`call_function`** — calls a plain PL/pgSQL function (`select
  my_function()` as the query's SQL) and shows back whatever the
  function itself returns; `raise exception '...'` inside it becomes
  the error banner verbatim (`actions::clean_db_error`). The function
  must already exist when the app is (first) synced/reloaded.
- **`log_values`** — trivial demo, logs the parameter map.

Rust and PL/pgSQL aren't a migration path away from each other: anything
leaving the database (HTTP, email) stays Rust; row-level logic already
living beside the data is cheaper as a function via `call_function`.
Both share the exact same `:name` → schema-typed bind compilation as
every other named query. No `apex_util`-style grab-bag package is
shipped — `clean_db_error` + `raise exception` covers the one thing
that genuinely generalizes.

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
(re-fetches one region via `GET /:app/:page/region/:query`, sending current
item values as query params). Dispatched by the DB-stored runtime.js;
`setItem` fires `change` events so actions chain (depth-guarded).

## Item types

A form field's widget is one small Rust file (`src/item_types/`)
implementing:

```rust
pub trait ItemType: Send + Sync {
    fn kind(&self) -> &'static str;                      // the markup keyword
    fn render(&self, args: RenderArgs) -> String;         // the <input>/etc, unwrapped
    fn read_value(&self, field_name: &str,                // default: raw submitted value
                   values: &HashMap<String, String>) -> String { ... }
}
```

`RenderArgs` carries name/value/required/column-type, raw JSON
`config`, and (for `choices`/`query` configs) already-resolved
`(value, label)` pairs.

```
text.rs      default for text/integer/timestamp
readonly.rs  visible, not editable, round-trips via hidden input
checkbox.rs  default for boolean
radio.rs     radio group over args.choices
popup.rs     "Pop Up LOV": <dialog> + search filter + pgapp.setItem(...)
slider.rs    <input type=range>, reads min/max/step from config
```

Adding one (say a date picker): write `src/item_types/date_picker.rs`,
add one line to `registry()` — nothing else changes, since everything
goes through the registry by kind string and a generic JSON config.
Compile-time plugin point (rebuild + restart, not hot-loaded); a
misspelled `kind` is caught at sync time.

`popup` renders every choice into the dialog up front; its search box
(`pgapp.filterPopup`/`pgapp.openPopup` in `/runtime.js`) filters
client-side by substring, resets on every open, and shows "No matches"
instead of a blank list — all with no server round trip.

## Charts

`type:` is one of six (`model::CHART_TYPES`), checked at sync time:

| type | rendering |
|---|---|
| `bar` | one rect per row |
| `line` | polyline + marker dots |
| `area` | `line`, filled to baseline |
| `scatter` | marker dots only |
| `pie` | wedges from 12 o'clock, side legend |
| `donut` | `pie` with the center punched out |

`bar`/`line`/`area`/`scatter` use `x` (category)/`y` (value); `pie`/
`donut` reuse the same two props as a slice's label/value.

Rendering backend is pluggable (`src/chart_lib.rs`): **`inline`**
(default) computes straight to SVG server-side, no JS; any other name
loads `chart-libs/<name>/chart.js`, and the server instead emits a
`<script type="application/json" class="pgapp-chart-data">` blob
(`{rows, x, y, type}`) for that JS to render however it likes.
Selected with `chart_lib: <name>`. `chart-libs/canvas-bars/` ships as
a working `<canvas>` example.

## Icons

A `Report`/`EditableTable` row's Edit/Delete glyphs come from a
pluggable icon pack (`src/icons.rs`): **`builtin`** (default, two
inline SVGs, no network) or `icons/<name>/pack.json` (`{"stylesheet":
"<url>", "icons": {"edit": {"class": "...", "content": "..."}}}` — Font
Awesome-style or ligature-style like Material Icons both fit). Selected
with `icons: <name>`.

## Named queries

`query <name> { sql: "..." }` — reusable SQL, referenced from a
`radio`/`popup` item (`from query <name>`), a `region`/`chart`
component, or a `report`'s `source: query <name>`. App-scoped (top of
`app { }`, visible everywhere) or page-scoped (visible only there,
shadowing an app-scoped name).

`:name` bind markers (a literal `::` cast is left alone) become real
bind parameters, never interpolation. Values come from the page's
query-string params, plus — when editing a row — that row's own
field values. A report's `link: <field> -> page <Name> (field: param)`
is how a value crosses pages, forwarding it as `?param=...` for the
target page's queries to read back as `:param`.

**Binds are typed automatically from the schema**, APEX-style but not
hand-declared: write `:project_id`, not `:project_id::integer`.
`meta::compile_named_query` asks Postgres's own `Describe` what type
each bind needs to be (fresh every load — startup or `/admin/reload`
 on that app),
so schema drift (`alter column ... type bigint`) is picked up
automatically or rejected loudly at sync time, never silently wrong at
runtime. An explicit `:name::type` cast still works (a redundant no-op
under the automatic one).

`radio`/`popup` queries alias columns `value` and optionally `label`
(defaults to `value`). A `report`'s `source:` query needs an `id`
column plus whatever `columns` reference. A `region` has no
requirements. A `chart` needs whatever `x`/`y` name. Query SQL
references the entity's real physical table
(`pgapp_data.<app>_<entity>`, printed at startup) and is decoded
generically via `to_jsonb`, so there's no compile-time column-type
checking beyond what Postgres itself enforces.

## Authentication & authorization

Opt in with an `auth { }` block:

- Every page requires a signed-in user; only `/:app/login` and static
  assets stay public.
- First run bootstraps the admin (`POST /:app/setup`, one-time); after
  that, admins manage accounts on the built-in `/:app/users` page.
  Users are never declared in markup.
- Passwords are argon2id hashes in `pgapp_meta.users`; sessions are
  server-side rows in `pgapp_meta.sessions` (an HttpOnly, `SameSite=Lax`
  cookie holds only a random token — revoking a session means deleting
  its row).
- A user has one `role` (free-form string). `requires: <role>` gates a
  page (and its create/update/delete routes) to that role or admin.

Without `auth { }`, an app stays fully public
(`examples/todo.pgapp`); `examples/helpdesk.pgapp` runs behind a login
with an admin-only Agents page.

## Runtime JS

`GET /:app/runtime.js` is a row in `pgapp_meta.app_runtime_js`, seeded from
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

`GET`/`POST /:app/admin/reload` re-syncs that app's markup file into
`pgapp_meta` and reloads it in place — no restart, and no effect on any
other app sharing the process. `AppState` holds what's shared and can't
change without a rebuild (`pool`, `item_types`, `actions`, the
`apps: HashMap<slug, AppEntry>` registry itself); each `AppEntry` splits
off everything markup-derived (`app`, `theme`, `runtime_js`, `icons`,
`chart_lib`) behind its own `RwLock<Arc<AppData>>`. Each request
snapshots that one app's data once at the top, so a concurrent reload
can't mix a new `RuntimeApp` with a stale `Theme`. A failed reload (bad
markup) never swaps in — the old snapshot keeps serving, and the error
shows on the reload page. The page itself offers an editable textarea
(single-file apps) or "reload from disk" (directory apps), gated to the
`admin` role when auth is enabled.

Not covered: new item types/actions, or the routing table itself, are
still Rust code — those need `cargo build` + restart. Markup changes
(new page/field/entity, a changed `theme:`, a new dynamic action) don't.

## Report edit/create popup

A `Form` that's a `Report`'s edit/create companion (same entity, same
page) renders as a `position: fixed`, non-modal popup
(`.pgapp-form-floating`) only when its `?edit_<idx>=<id>`/`?new_<idx>=1`
flag is present — not as a block that pushes the table down on every
load. The report gets a "+ New" button; the popup has a `×`/Cancel to
close, with no dimming backdrop so the report stays usable behind it.
A standalone `Form` (no sibling `Report`) is unaffected. Every mutating
action redirects to `#pgapp-c<idx>` instead of the bare page URL, so
scroll position is preserved.

## Mobile

No per-app work required: a viewport meta tag, horizontally-scrolling
table wrappers, and each shipped theme's `@media (max-width: 640px)`
rules (nav wraps, report toolbar stacks, floating form becomes a
near-full-width sheet). Multi-level `nav` also has a click-to-toggle
caret (touch has no `:hover`), working alongside the existing hover
behavior.

## Architecture

```
 .pgapp markup file
        │  markup::parse_app
        ▼
    AppDef (in memory)
        │  meta::sync_app (validates entity/page/query refs by name,
        │                  item kinds against the item_types registry)
        ▼
 pgapp_meta.* tables  ──creates──▶  pgapp_data.<table> (the real data table)
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
generic-JSON-config pattern item types use, extended to whole
components, so a new component kind never needs a schema change.

### Source layout

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
  actions/            one file per module: run_query, call_function, log_values
  meta.rs             module root: ensure_schema + re-exports
  meta/
    types.rs          the runtime model (RuntimeApp, RuntimePage, RuntimeComponent, Chrome, ...)
    sync.rs           AppDef -> pgapp_meta.* (+ physical data tables)
    load.rs           pgapp_meta.* -> RuntimeApp, compile_named_query
  server.rs           module root: AppState/AppEntry, /:app routes, HTTP handlers, pagination
  server/
    query_engine.rs   named-query execution (+ paginated), bind context, LOV/region resolution
  render.rs           HTML generation; delegates field widgets to item_types, charts to chart_lib
themes/               pluggable design systems (see "Theming")
icons/                pluggable icon packs: fontawesome/, material/
chart-libs/           pluggable chart libraries: canvas-bars/
```

## Theming

Every server-rendered element carries one of a fixed set of
`.pgapp-*` classes (`pgapp-nav`, `pgapp-table`, `pgapp-form`,
`pgapp-field`, `pgapp-input`, `pgapp-btn` + variants, `pgapp-report`,
`pgapp-chart`, `pgapp-popup` + subparts, `pgapp-region`,
`pgapp-editable-table`, `pgapp-alert`, `pgapp-pagination`, `pgapp-icon`,
and a few more — grep `render.rs` for the exhaustive list) and nothing
else. A **theme** just gives those classes an appearance; that's the
whole contract.

**Per-instance overrides**: any component can end with `attrs (id:
"...", class: "...", data_foo: "bar")` — `id`/`class` are reserved
(`class` *appends* to the required class, never replacing it), any
other key becomes an attribute (`_` → `-`). On `form`/`editable_table`,
`item <field> attrs (...)` does the same one level deeper, for just
that field's wrapper, independent of (or combined with) an `as <kind>`
override. Both are pure opt-in — unset, a component/field renders
exactly as before.

A theme is `themes/<name>/theme.css` (required) + `theme.json`
(optional, `{"label", "description"}`), selected with `theme: <name>`
(default `shadcn`) — a missing theme refuses startup with a clear
error.

**Shipped**: `shadcn` (default zinc palette, HSL vars, light/dark),
`plain` (zero design-system assumptions), `vivid` (colorful demo
theme, used by `examples/helpdesk.pgapp`), `google_m3` (Material
Design 3 — tonal surfaces, pill buttons, 4px field radius, 28px dialog
corners; selected as `google_m3` since markup identifiers can't
contain hyphens).

To add another design system: `themes/<name>/theme.css` + `theme:
<name>` — no Rust changes.

## Routes

Every route lives under `/:app` — an app's URL slug (see "Multi-app
routing"). On startup it prints each app's full URL and its
pages' component kinds, e.g.
`http://127.0.0.1:8080/todo/Tasks  [report, form, text, region]`.

- `GET  /`                                   — one app: redirects there; several: a plain list of them
- `GET  /:app`                               — redirects to the app's first page
- `GET  /:app/:page`                         — renders every component on the page, in order
- `POST /:app/:page/c/:idx/create`           — create a row (`Form`/`EditableTable` only, by component index)
- `POST /:app/:page/c/:idx/update/:id`       — update a row
- `POST /:app/:page/c/:idx/delete/:id`       — delete a row
- `GET  /:app/api/:entity`                   — JSON list for that entity
- `GET  /:app/:page/region/:query`           — one region re-rendered as a fragment (dynamic-action refresh)
- `POST /:app/:page/c/:idx/run`              — run an `action` component's server-side module
- `POST /:app/:page/c/:idx/views` (+ delete) — save / delete a report's saved view
- `GET  /:app/login` / `POST /:app/login`    — sign-in page (or first-run admin setup) — apps with `auth { }` only
- `POST /:app/setup`                         — one-time admin bootstrap; refuses once any user exists
- `POST /:app/logout`                        — deletes the server-side session
- `GET  /:app/users` (+ create/delete POSTs) — built-in user management, admin role only
- `GET  /:app/admin/reload` (+ POST)         — re-syncs that app's markup file into `pgapp_meta` and reloads it, no restart
- `GET  /:app/runtime.js`                    — the DB-stored `pgapp` JS runtime
- `GET  /:app/chart-lib.js`                  — the active pluggable chart library's JS (404 when `chart_lib` is the built-in `inline`)

A `Form` switches into edit mode via `?edit_<n>=<id>` (`<n>` = its
0-based position on the page); a `Report`'s pagination uses
`?r<n>_after=`/`?r<n>_before=` (entity-backed) or `?r<n>_page=`
(query-sourced) the same way.

## Multi-app routing

One process, one shared `PgPool`, any number of apps — closer to how
Oracle APEX actually pools connections (one pool per workspace, shared
by every application in it) than to a separate server per app. Every
app keeps its own tables (`pgapp_data.<app>_<entity>`), sessions, and
users; only the connection pool and Rust process are shared.

The pool defaults to **20 connections** — comfortably above a
handful of toy connections without assuming "bigger is always faster"
(a Postgres backend is a full process, not a lightweight thread, so a
few dozen is already generous for one server); override with
`PGAPP_MAX_CONNECTIONS`. Same default and override for the
`pgapp_admin` connection Instance mode's `pgapp run` serves through.

**What's registered, not what's on the command line, decides what's
served.** `pgapp_control.apps` (a schema of its own — pgapp managing
itself, distinct from `pgapp_meta`'s per-app metadata) is the durable
list of `(slug, markup_path, enabled)` rows. `cargo run -- <path>`
registers (or re-points) one slug and then serves *every enabled row*,
not just that one — so pointing the server at a new app one run adds
it alongside whatever was already registered, without needing to name
every app again each time:

```bash
cargo run -- helpdesk.pgapp   # registers + serves "helpdesk"
cargo run -- inventory.pgapp  # registers "inventory" too; both now serve
cargo run -- apps             # slug  enabled/disabled  markup_path, one per line
cargo run -- remove inventory # disables it — helpdesk keeps serving
```

A directory can also declare a whole workspace up front: if it
contains only subdirectories (no loose `.pgapp` files of its own), each
subdirectory is loaded as an independent app, slugged from its own
declared name — `cargo run -- workspace/` where `workspace/helpdesk/`
and `workspace/inventory/` each look like a normal single-app directory
(see "Scaffolding a new app"). A directory with any loose `.pgapp` file
directly inside it is still just one app, exactly as before — this
only kicks in for a directory of nothing but subdirectories.

Sessions are app-scoped even though the cookie name is shared: the
`Set-Cookie` carries `Path=/<slug>`, so a browser never sends one app's
token to another's routes, and `pgapp_meta.sessions`/`.users` are
looked up by `app_id` regardless.

Note: "workspace" above just means "a directory that declares several
apps at once" — every app still shares the single global `pgapp_data`
schema. For apps whose *data* actually needs to live in separate
Postgres schemas (different teams, different access grants), see
"Instance mode" below, which uses the same word for something stronger.

## Scaffolding a new app

`pgapp new`/`pgapp create` generates a minimal, runnable starter app —
one entity, one page with the classic Report+Form CRUD pattern, a nav
link to it:

```bash
# Non-interactive, DB-free — for scripts/CI:
cargo run -- new "My Project"                    # -> my_project.pgapp
cargo run -- new Inventory inventory.pgapp        # explicit path
cargo run -- new Inventory --dir --theme vivid    # a directory scaffold instead

# Interactive (see "Quickstart" above):
cargo run -- create
```

`cargo install --path .` builds and installs **both** binaries this
crate defines — `pgapp` itself (so every `cargo run -- <args>` example
in this README becomes just `pgapp <args>`, no `cargo run --` needed)
and `cargo-pgapp`, reachable as a real `cargo` subcommand:

```bash
cargo install --path .
pgapp new "My Project"          # same as `cargo run -- new "My Project"`
pgapp create                    # or: cargo pgapp create
```

(`cargo install --path . --bin cargo-pgapp` installs just the `cargo
pgapp` alias, if that's all you want.)

See `pgapp new --help` for every flag.

## Instance mode

A durable, database-backed deployment model on top of everything
above — for when apps genuinely need separate Postgres schemas (a
team's own credentials, different access grants), not just separate
`pgapp_control` rows. Entirely opt-in: `cargo run -- <path>` and
friends are unaffected and keep working exactly as documented above.

The commands below assume `pgapp` is installed (`cargo install --path
.` — see "Scaffolding a new app"); swap in `cargo run --` if you'd
rather not install it (e.g. `cargo run -- instance init`).

**Instance** = one target database, one dedicated `pgapp_admin`
Postgres login role the server operates as from then on:

```bash
pgapp instance init
```

Prompts for a superuser-capable connection string, the database name
(auto-created if missing, same as the Quickstart flow), a password to
set for the new `pgapp_admin` role, and a separate local CLI admin
password. Two different secrets, two different fates:

- `pgapp_admin`'s Postgres password is **never written to disk** — a
  one-way hash can't be used to reconnect, so every later command reads
  it fresh from `PGAPP_ADMIN_DB_PASSWORD`.
- The CLI admin password *is* stored, but only as an argon2 hash, in
  `~/.pgapp/instances/<dbname>.json` (`0600`) — it just gates who's
  allowed to run instance/workspace/app commands against this instance
  at all, checked interactively (or via `PGAPP_CLI_ADMIN_PASSWORD` for
  scripts), and has nothing to do with Postgres auth.

**Workspace** = a Postgres schema an app's data tables live in:

```bash
pgapp workspace create <dbname>
```

Asks for a name, then whether to use an existing schema or create one.
A new schema gets its own owning login role (password prompted,
`pgapp_admin` is granted membership + USAGE/CREATE); an existing schema
just asks whether to grant `pgapp_admin` USAGE/CREATE into it (using a
connection that can actually perform that grant — `pgapp_admin` has no
privileges of its own on a schema it didn't create).

**App** = scaffolded and registered into a chosen workspace:

```bash
pgapp app create <dbname> [--workspace <slug>]
```

Same prompts as `pgapp new`'s interactive flow, plus a workspace
picker (lists every registered workspace and lets you choose) when
`--workspace` isn't given. Then serve it — and every other enabled app
across the whole instance, classic and workspace-scoped alike, same
"the registry decides what's served" rule as multi-app routing:

```bash
pgapp run <file>.pgapp --instance <dbname> [--workspace <slug>]
```

**Destroy**, for all three, always needs the CLI admin password first:

- `pgapp instance destroy <dbname>` — always a hard delete: drops
  every workspace schema/role pgapp itself created, `pgapp_meta`/
  `pgapp_data`/`pgapp_control`, the `pgapp_admin` role, and the local
  instance file. Asks for a superuser connection fresh (never stored)
  and requires typing the database name to confirm.
- `pgapp workspace destroy <dbname> <slug> [--hard|--soft]` — soft
  just disables the registry row (schema/data untouched, reversible);
  hard drops the schema and, if pgapp created it, its owning role too
  (again via a fresh superuser connection) — refuses without an extra
  typed confirmation if apps are still registered in it.
- `pgapp app destroy <dbname> <slug> [--hard|--soft]` — soft disables;
  hard drops its entity tables and `pgapp_meta` rows (using
  `pgapp_admin`'s own connection — it already owns whatever it created).

`pgapp workspace list <dbname>` and `pgapp apps` show what's currently
registered.

## Roadmap (not in this slice)

- Separate connection *pool* per workspace — "Instance mode" gives
  every workspace its own schema/role, but all of them still share one
  `PgPool` per process (matches how APEX itself pools connections; a
  true pool-per-workspace would be a bigger, probably unnecessary,
  change)
- No CLI-driven credential rotation — a workspace/pgapp_admin password
  is set once at creation; changing it means an ad hoc `ALTER ROLE`
  today, no `pgapp instance rotate-password`-style command yet
- More field types and real relationships (foreign keys) — named
  queries cover ad hoc joins today, but no schema-level entity-to-entity
  references yet
- A real drag-and-drop builder UI
- Multi-step `flow` blocks chaining actions/dynamic actions with
  conditions
- runtime.js is seeded once per app; picking up a newer built-in seed
  needs deleting the `pgapp_meta.app_runtime_js` row — no versioned
  upgrade story yet
- Field-level authorization (page-level `requires:` exists, per-column
  doesn't), plus password reset flows (today an admin deletes/recreates
  the account)
- Login sessions have no `Secure` cookie attribute — fine for
  localhost, add it behind TLS
- Item type config is always flat strings, even for numeric-looking
  values (Slider's `min`/`max`)
- `ensure_data_table` adds columns to an existing table but doesn't
  handle renames, type changes, or drops
- Separate create vs. edit field lists (a `readonly` field with a
  meaningful default doesn't get nulled out on create)
- `RegionRows` is keyed only by query name per request — a page-scoped
  and an app-scoped query sharing a name would collide (rare, not
  guarded against)
- No validation of a named query's SQL beyond the bind-marker scan — a
  typo surfaces as a runtime error on first use
- A `Report`'s row actions only wire to a `Form` on the *same page*
- CSS-icon packs whose stylesheet is a remote CDN URL need outbound
  network access to actually render glyphs
