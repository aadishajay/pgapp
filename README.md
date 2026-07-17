# pgapp

An Oracle APEX-inspired, no/low-code application framework built on
Postgres, written in Rust.

## Idea

- **In-database metadata**: applications, entities, fields, pages and
  components are rows in Postgres (`pgapp_meta.*`), not config files
  scattered across a repo. The server always serves off the database,
  not off whatever was last parsed.
- **A textual markup language** (`.pgapp` files, APEX-flavored) is how
  you *author* an application. It's parsed once and synced into the
  metadata tables — after that, the database is the source of truth.
- **Composable pages**: a page isn't one of a fixed set of "kinds" — it's
  an ordered list of independent **components**: a paginated read-only
  `Report`, a create/edit `Form`, an inline-editable `EditableTable`, a
  `Chart`, or plain `Text`/`Link`/`Region` content. Any combination, any
  number, on one page — see "Components" below.
- **Pluggable design system**: rendered HTML only ever uses a fixed set
  of `.pgapp-*` classes; a swappable `theme.css` (see "Theming" below)
  gives them their actual look. `assets/app.css`/`app.js` still exist as
  a per-app override layer on top of whatever theme is active.
- **Pluggable item types**: a form field's widget (Text, Checkbox,
  Radio, Popup, a Slider, ...) is a component in its own file under
  `src/item_types/`, not a hardcoded match arm — see "Item types" below.
- **Pluggable charts and icons**: a `Chart` component's rendering
  backend and a `Report`/`EditableTable` row's edit/delete glyphs are
  both swappable the same way themes are — a dependency-free default,
  plus a named alternative selected in the markup. See "Charts" and
  "Icons" below.
- **Named queries**: reusable SQL, declared once (app-wide or scoped to
  one page) and referenced by name from LOVs, regions, charts, and a
  report's row source — see "Named queries" below.
- **Authentication & authorization**: an `auth { }` block in the markup
  puts the whole app behind a login (argon2-hashed passwords,
  server-side sessions, a first-run admin setup screen, a built-in
  /users admin page), and `requires: <role>` restricts individual pages
  by role — see "Authentication & authorization" below.
- **App settings live in the app definition**: `theme:`, `icons:`, and
  `chart_lib:` are markup properties synced into `pgapp_meta.apps`,
  not environment variables — the file describes the whole app,
  including how it looks.
- **A DB-stored JS runtime**: `/runtime.js` isn't a static file — it's a
  row in `pgapp_meta`, seeded from a built-in default and editable from
  there afterward. It exposes `pgapp.getItem`/`pgapp.setItem` so item
  values are captured/set by a method call, not ad hoc DOM lookups.
- **Rust instead of PostgREST**: rather than fronting Postgres with
  PostgREST, pgapp's own Axum server owns routing directly, using the
  metadata to build parameterized SQL on the fly.

## Current status: vertical slice

This is deliberately the *smallest end-to-end loop*, not the whole
framework: one markup file → one app, hardcoded single-tenant, one field
type set (`id`, `text`, `boolean`, `integer`, `timestamp`). It exists to
prove the architecture end-to-end before building the bigger pieces
(drag-and-drop builder UI, actions/flows, multi-app routing) on top of
it.

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
  # same page (as here) is the classic list+edit CRUD pattern — but
  # they're two independent, composable pieces, not one fixed page kind.
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
            where priority = :priority::text and id != :id::integer
            order by id"
    }
    region "Other tasks with the same priority" from query siblings
  }

  # A Report sourced from a query instead of the entity table directly.
  # No Form here, so it's read-only.
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

- `header { }` / `footer { }` (optional, top-level) declare app-wide
  chrome shown on every page. They reuse the same component grammar as
  a page body, but are restricted (checked at sync time) to
  `text`/`link`/`region` — chrome has no per-request entity or
  pagination context for a Report/Form/EditableTable/Chart to use.
- `nav { }` (optional, top-level) declares the app's navigation bar.
  Each `item "Label"` is either a leaf (`-> page <Name>`) or a group
  (`{ ... }` of nested items) — nesting groups gives you a multi-level
  menu, rendered site-wide.
- `entity` blocks describe a table: each `field` has a type, and
  optionally `required` and/or a `default`.
- `page "Name" { ... }` is just a name plus an ordered list of
  components (and optional page-scoped `query` blocks) — see
  "Components" below for what each one does. A page may also declare
  `requires: <role>` — see "Authentication & authorization".
- `theme:` / `icons:` / `chart_lib:` (optional, app level) select the
  pluggable theme, icon pack, and chart library directories; `auth { }`
  turns on login. These are part of the app definition and synced into
  `pgapp_meta.apps` like everything else — there are no environment
  variables for them.
- Anything that targets a page by name (`nav` items, a report's `link:`,
  a `link` component) uses a bare identifier, not a quoted string —
  restricting link targets to the same safe charset as SQL identifiers.
  Query and entity names are the same way.

See `src/markup.rs` for the full grammar and `examples/todo.pgapp` for a
working example. `examples/helpdesk.pgapp` is a richer one — two
entities, a chart dashboard, both pagination modes, auth, and every
built-in item type — with demo data in `examples/helpdesk_seed.sql`, a
feature-by-feature tour (with live screenshots) in
`marketing/index.html`, and the colorful `themes/vivid/` theme selected
by its own `theme: vivid` line: `cargo run -- examples/helpdesk.pgapp`.

### One file, or a directory

An app is authored as either a single `.pgapp` file or a **directory**
of them — same grammar, zero refactoring to move between the two.
Directory semantics are deliberately Terraform-shaped: every `.pgapp`
file under the directory (recursively) merges into one app by name.

- Exactly one file declares the `app "..." { }` block — settings,
  `auth`, `nav`, and `header`/`footer` chrome live there.
- Every other file is a *fragment*: any number of top-level `entity`,
  `page`, and `query` blocks (top-level queries are app-scoped;
  page-scoped queries stay inside their page as always). Fragments
  reference each other by name exactly as within one file — the
  metadata sync already resolves forward references, so file order
  never matters.
- There is no `include` statement and no import graph. The same name
  declared in two files is a startup error naming both files, and parse
  errors report `file (line N)`.

`examples/helpdesk-modular/` is the helpdesk app split this way — one
file for the app shell, one per entity, one per page group:

```
examples/helpdesk-modular/
  app.pgapp            app "Helpdesk" { theme, auth, nav, header, footer }
  tickets.pgapp        entity "tickets" + its app-scoped queries
  agents.pgapp         entity "agents" + the agent_names LOV query
  pages/
    dashboard.pgapp    page "Dashboard"
    tickets.pgapp      pages "Tickets" + "TicketDetail"
    agents.pgapp       page "Agents" (requires: admin)
    backlog.pgapp      pages "Backlog" + "About"
```

Run it with `cargo run -- examples/helpdesk-modular` — it syncs to the
same metadata as the single-file version. The file layout is purely an
authoring convenience; the database never sees it.

## Components

A page's body is `Vec<ComponentDef>` (`src/model.rs`) — there's no
separate "page kind" the way there used to be. Seven kinds:

- **`report "Title" of <entity> { ... }`** — a read-only, paginated
  table. `columns` are shown; `source: query <name>` sources rows from
  a named query instead of the entity table directly (writes still
  target the entity by id); `link: <field> -> page <Name> (extra: param,
  ...)` turns that column into a link to another page, forwarding the
  row's id as `?id=` plus any extra columns as named parameters;
  `page_size` (default 20) controls pagination (see "Pagination"
  below). If the same page also has a `Form` bound to the same entity,
  each row automatically gets Edit/Delete actions that target it — no
  extra config needed, just put both components on one page.
- **`form "Title" of <entity> { ... }`** — a create/edit form. `fields`
  lists the writable columns; `item <field> as <kind> [(...)]` picks
  each one's widget the same way it always has (see "Item types"
  below). Renders blank (create mode) by default; visiting the page
  with `?edit_<n>=<id>` (`<n>` is this component's position on the
  page, 0-based) switches it into edit mode for that row, with a
  Delete button alongside Save.
- **`editable_table "Title" of <entity> { ... }`** — every row rendered
  as its own inline-editable form (one per row) plus an "add new" row —
  no separate list/edit split. A good fit for a small table you want to
  bulk-tweak in place. Not paginated.
- **`chart "Title" from query <name> { type: bar|line x: <col> y: <col> }`**
  — renders the query's rows as a chart; see "Charts" below for the
  pluggable rendering backend.
- **`text "..."`** — static text.
- **`link "Label" -> page <Name>`** — a link to another page.
- **`region "Label" from query <name>`** — a named query's rows
  rendered as a plain, non-paginated table; sugar for a small
  fixed-shape report without entity/pagination machinery.

## Pagination

`Report` pagination is backend-optimized to avoid the two classic
expensive patterns (`COUNT(*)` and `OFFSET`-skipping over a large
table), via `server::fetch_report_rows` / `query_engine::run_named_query_page`:

- **Entity-backed** (no `source:`): **keyset ("seek") pagination** on
  `id`. `?r<n>_after=<id>` / `?r<n>_before=<id>` (`<n>` = the report's
  position on the page) fetch `page_size + 1` rows in that direction —
  the extra row tells you whether *that* direction has more, and the
  direction you arrived *from* always has more (reaching a page via a
  cursor implies a page on the other side of it). Zero extra queries,
  and it stays cheap regardless of table size.
- **Query-sourced** (`source: query <name>`): an arbitrary query has no
  assumed stable sort key, so this falls back to `?r<n>_page=<n>`
  (`OFFSET`-based), but still avoids `COUNT(*)` by fetching
  `page_size + 1` rows the same way.

## Item types

A form field's widget is one small Rust file implementing this trait
(`src/item_types.rs`):

```rust
pub trait ItemType: Send + Sync {
    fn kind(&self) -> &'static str;                      // the markup keyword
    fn render(&self, args: RenderArgs) -> String;         // the <input>/etc, unwrapped
    fn read_value(&self, field_name: &str,                // default: raw submitted value
                   values: &HashMap<String, String>) -> String { ... }
}
```

`RenderArgs` carries the field's name/value/required-ness/column type,
its raw JSON `config`, and (for anything whose config used `choices`/
`query`) the already-resolved `(value, label)` pairs — resolving a
`query` means an async DB call, so that happens once up front in
`server::query_engine`, before any (synchronous) rendering runs.
`read_value` only needs overriding when a field doesn't submit a plain
value the usual way — Checkbox is the one built-in example, since an
unchecked box never sends its key at all.

```
src/item_types.rs        registry() + the ItemType trait + default_kind_for
src/item_types/
  text.rs                default for text/integer/timestamp
  readonly.rs             visible, not editable, round-trips via hidden input
  checkbox.rs             default for boolean; read_value overridden
  radio.rs                radio group over args.choices
  popup.rs                "Pop Up LOV": <dialog> + pgapp.setItem(...)
  slider.rs               <input type=range>, reads min/max/step from config
```

Adding a new one (say a date picker) means writing
`src/item_types/date_picker.rs` implementing the trait and adding one
line to `registry()` in `src/item_types.rs` — nothing in `markup.rs`,
`meta/`, `server.rs`, or `render.rs` needs to change, since all of them
only ever go through the registry by kind string and a generic JSON
config. That said, this is a *compile-time* plugin point: Rust has no
way to pick up a dropped-in `.rs` file without a rebuild. "Drop in a
file" here means write it, register it, `cargo build`, restart — not
hot-loading into a running process. A wrong/misspelled `kind` in markup
is caught at sync time (`meta::sync_app` checks every declared kind
against the registry) rather than failing silently at render time.

Two config keys are reserved by convention, generic across every kind
(see `server::query_engine::resolve_field_choices`): `choices` (a fixed
list) and `query` (a named query's rows instead) — a component only
needs to read `args.choices` to get either, without caring which one it
was.

## Charts

A `chart` component's rendering backend is pluggable the same way a
theme is (`src/chart_lib.rs`):

- **`inline`** (default) — a dependency-free bar/line chart computed
  straight to inline SVG server-side (`render::inline_svg_chart`). No
  JS, no network fetch, works everywhere.
- **any other name** — loads `chart-libs/<name>/chart.js`, served at
  `GET /chart-lib.js` and linked from every page. For each chart, the
  server instead emits a `<div class="pgapp-chart">` placeholder plus a
  `<script type="application/json" class="pgapp-chart-data">` blob
  (`{rows, x, y, type}`); the library's JS reads that and renders into
  the div however it likes — canvas, its own SVG, a real charting
  library. The server never needs to know how.

Selected per app with `chart_lib: <name>` in the markup (default
`inline`). `chart-libs/canvas-bars/`
ships as a second, working example — a small dependency-free `<canvas>`
bar-chart renderer — proving the plug point without requiring a CDN.

## Icons

A `Report`/`EditableTable` row's Edit/Delete glyphs come from a
pluggable "icon pack" (`src/icons.rs`), mirroring the theme contract:

- **`builtin`** (default) — two hard-coded inline SVGs (a pencil, a
  trash can). No network fetch, no font.
- **any other name** — loads `icons/<name>/pack.json`:
  `{"stylesheet": "<url>", "icons": {"edit": {"class": "...", "content":
  "..."}, ...}}`. Icons render as `<i class="pgapp-icon <class>">
  <content></i>`, and `stylesheet` is linked once in `<head>`. `content`
  is optional and covers both flavors of font-based icon system: Font
  Awesome-style packs give every icon its own class and leave `content`
  empty (`icons/fontawesome/`); ligature-style packs like Material
  Icons share one class across all icons and put the icon's name in
  `content` (`icons/material/`). Either way nothing is fetched by this
  server at render time — it only ever emits a class name and maybe a
  word of text; the browser fetches the actual font/CSS from
  `stylesheet`.

Selected per app with `icons: <name>` in the markup (default `builtin`).

## Named queries

A `query <name> { sql: "..." }` block is reusable SQL, referenced by
name from a `radio`/`popup` item type (`from query <name>`), a `region`
or `chart` component, or a `report`'s `source: query <name>`. Two
scopes:

- Declared at the top of `app { }`: visible from every page.
- Declared inside a `page { }` block: visible only there, shadowing an
  app-scoped query of the same name.

`sql` can contain `:name` bind markers (a literal `::` Postgres cast is
left alone, so ordinary casts in hand-written SQL are never mistaken for
one). Every marker becomes a genuine bind parameter — never string
interpolation — always cast as `$N::text`, so a query comparing against
a non-text column needs its own trailing cast: `where project_id =
:project_id::integer`. Bind values come from a page's incoming
query-string parameters, plus — when editing/viewing one row — that
row's own field values (query-string wins on conflict). That's also how
a value on one page reaches a query on another: a report's `link:
<field> -> page <Name> (field: param)` forwards a row's column as
`?param=...` on the target page's URL, where its named queries can read
it as `:param`. The "other tasks with the same priority" region above
demonstrates this — try changing the forwarded `?priority=` in the URL
and the region's results change with it, independent of the row
actually being viewed.

`radio`/`popup` queries must alias their columns `value` and, optionally,
`label` (defaulting to `value` if omitted) — e.g. `select distinct
assignee as value from ...`. A `report`'s `source: query <name>` needs
an `id` column plus whatever the report's `columns` reference by name;
writes (create/update/delete) still always target the underlying
entity by id, regardless of what the report itself selects. A `region`
has no column requirements — it renders whatever the query returns. A
`chart` needs whatever columns its `x`/`y` name.

Query SQL isn't translated from logical entity names — it references
the entity's real physical table (`pgapp_data.<app slug>_<entity
slug>`, printed at startup for each page). This also means query results
are decoded generically (via Postgres's `to_jsonb`) rather than through
the same typed pipeline as entity-bound CRUD, so there's no column-type
checking on a query's own SELECT list beyond what Postgres itself
enforces.

## Authentication & authorization

Opt in per app with an `auth { }` block (the block is empty today,
reserved for future options). With it present:

- **Every page requires a signed-in user.** Unauthenticated requests
  redirect to `/login`. Only the login flow and static assets
  (`/theme.css`, `/runtime.js`, `/chart-lib.js`, `/assets/*`) stay
  public.
- **First run bootstraps the admin.** When the app has no users, the
  login page becomes a one-time "create the admin account" form
  (`POST /setup`, which refuses the moment any user exists). After
  that, admins manage accounts on the built-in `/users` page — users
  are deliberately *not* declarable in markup, because passwords don't
  belong in a source file.
- **Passwords are argon2id hashes**, never plaintext, in
  `pgapp_meta.users`. Sessions are server-side rows in
  `pgapp_meta.sessions`; the browser holds only a random token in an
  HttpOnly `SameSite=Lax` cookie, so any session can be revoked by
  deleting its row. Sign out deletes the row, not just the cookie.
- **Roles gate pages.** A user has one `role` (a free-form string —
  `admin`, `support`, whatever your pages need). A page declaring
  `requires: support` is visible only to users holding that role;
  `admin` passes every check. Reads and writes through the page are
  gated alike (a 403 on the page is a 403 on its create/update/delete
  routes too). Pages without `requires:` need any signed-in user.

Apps without an `auth { }` block skip all of this and stay public —
`examples/todo.pgapp` runs open, `examples/helpdesk.pgapp` runs behind
a login with an admin-only Agents page.

## Runtime JS

`GET /runtime.js` isn't a static file: it's a row in
`pgapp_meta.app_runtime_js`, seeded from a built-in default (`src/runtime.js`)
the first time an app is synced and left alone after that — so it's
metadata like everything else, editable in the database without
touching the binary. It's auto-linked into every rendered page and
defines `window.pgapp.getItem(name)` / `.setItem(name, value)`, which
work the same way regardless of whether `name` is a plain input, a
checkbox, a radio group, or a popup LOV's hidden input. The popup LOV's
"pick a value" buttons call `pgapp.setItem(...)` rather than touching
the DOM directly — capturing/setting an item's value by a method call is
the point, so any future custom action code has one consistent API.

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

Because every SQL identifier (table/column name) used at request time
comes from `pgapp_meta` — populated only from markup identifiers that
were already restricted to `[A-Za-z_][A-Za-z0-9_]*` by the lexer — it's
safe to splice them into generated SQL. All user-supplied *values* are
always sent as bind parameters, cast in SQL to the field's declared type
(e.g. `$1::boolean`), since the generic layer doesn't know column types
at compile time.

A page's components live in one generic table,
`pgapp_meta.components (id, app_id, page_id, slot, kind, ordinal,
config jsonb)` — the same "generic JSON config" pattern used for item
types, extended up to the whole-component level, so adding a new
component kind never requires a schema change. `config` embeds
page/entity/query *names* directly (not ids), validated once at sync
time (mirroring how item kinds are checked against the registry) —
nothing downstream needs another join to resolve them.

### Source layout

```
src/
  main.rs             wires everything together: registry, theme, icons, chart lib
  markup.rs           lexer + parser: .pgapp text -> model::AppDef (or a Fragment)
  source.rs           loads a file or merges a directory of .pgapp files into one AppDef
  model.rs            the parsed-markup types (AppDef, PageDef, ComponentDef, FieldItem, ...)
  html.rs             shared escape/js_escape/url_encode helpers
  theme.rs            theme.css/theme.json loading (see "Theming")
  icons.rs            icon pack loading (see "Icons")
  chart_lib.rs        chart library loading (see "Charts")
  runtime.js          seed content for the DB-stored JS runtime
  item_types.rs       the ItemType trait + registry() (see "Item types")
  item_types/         one file per component: text, readonly, checkbox, radio, popup, slider
  meta.rs             module root: ensure_schema + re-exports
  meta/
    types.rs          the runtime model (RuntimeApp, RuntimePage, RuntimeComponent, Chrome, ...)
    sync.rs           AppDef -> pgapp_meta.* (+ physical data tables)
    load.rs           pgapp_meta.* -> RuntimeApp, compile_named_query
  server.rs           module root: AppState, routes, HTTP handlers, pagination
  server/
    query_engine.rs   named-query execution (+ paginated), bind context, LOV/region resolution
  render.rs           HTML generation; delegates field widgets to item_types, charts to chart_lib
themes/               pluggable design systems (see "Theming")
icons/                pluggable icon packs: fontawesome/, material/
chart-libs/           pluggable chart libraries: canvas-bars/
```

## Theming

pgapp doesn't hardcode a look. Every server-rendered element carries one
of a fixed set of classes — `pgapp-nav`, `pgapp-link`, `pgapp-title`,
`pgapp-subtitle`, `pgapp-table`, `pgapp-form`, `pgapp-field`,
`pgapp-label`, `pgapp-input`, `pgapp-select`, `pgapp-btn` (+
`pgapp-btn-primary` / `pgapp-btn-destructive` / `pgapp-btn-secondary` /
`pgapp-btn-disabled`), `pgapp-inline-form`, `pgapp-alert` (+
`pgapp-alert-error`), `pgapp-list`, `pgapp-navbar` (+
`pgapp-navbar-item` / `pgapp-navbar-label` / `pgapp-navbar-submenu`),
`pgapp-items`, `pgapp-text`, `pgapp-header`, `pgapp-footer`,
`pgapp-checkbox`, `pgapp-readonly`, `pgapp-radio-group` (+
`pgapp-radio-option`), `pgapp-popup` (+ `pgapp-popup-dialog` /
`pgapp-popup-list`), `pgapp-region` (+ `pgapp-region-title`),
`pgapp-report`, `pgapp-form-panel`, `pgapp-editable-table` (+
`pgapp-editable-row-wrap` / `pgapp-editable-row`), `pgapp-row-actions`,
`pgapp-pagination`, `pgapp-icon`, `pgapp-chart` (+ `pgapp-chart-svg`) —
and nothing else. A **theme** is what gives those classes an actual
appearance. This is the whole contract; anything that satisfies it is a
valid theme, regardless of what design system it's built on.

### The contract

A theme is a directory at `themes/<name>/` containing:

- `theme.css` (required) — styles the `.pgapp-*` classes above however
  it wants (CSS variables + utility-style rules, plain selectors,
  whatever the source design system uses). Served at `GET /theme.css`
  and linked first in `<head>`, before `assets/app.css`.
- `theme.json` (optional) — `{ "label": "...", "description": "..." }`,
  human-facing metadata, printed at startup. Doesn't affect rendering.

Select a theme with `theme: <name>` in the app's markup (default:
`shadcn`) — the app definition owns its look, not the process
environment. If
`themes/<name>/theme.css` doesn't exist, the server refuses to start
with a clear error rather than silently falling back.

### Shipped themes

- `themes/shadcn/` (default) — shadcn/ui's default zinc palette:
  HSL custom properties (`--background`, `--primary`, `--border`,
  `--radius`, ...) with light/dark handled via
  `prefers-color-scheme`.
- `themes/plain/` — the same classes styled with plain CSS and zero
  design-system assumptions, proving the contract isn't shadcn-specific.

To add another design system (Bootstrap, Material, a custom brand kit,
...), create `themes/<name>/theme.css` styling the classes above and run
with `theme: <name>` in the app's markup. No Rust changes needed —
theming is fully decoupled from the framework.

## Running it

Requires a reachable Postgres instance.

```bash
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/pgapp
createdb -U postgres pgapp   # if it doesn't exist yet

cargo run                     # serves examples/todo.pgapp on 127.0.0.1:8080
# or: cargo run -- path/to/your.pgapp
```

On startup it prints the URL and component kinds for each page, e.g.
`http://127.0.0.1:8080/Tasks  [report, form, text, region]`.

- `GET  /`                              — index of pages in the app
- `GET  /:page`                         — renders every component on the page, in order
- `POST /:page/c/:idx/create`           — create a row (`Form`/`EditableTable` only, by component index)
- `POST /:page/c/:idx/update/:id`       — update a row
- `POST /:page/c/:idx/delete/:id`       — delete a row
- `GET  /api/:entity`                   — JSON list for that entity
- `GET  /login` / `POST /login`         — sign-in page (or first-run admin setup) — apps with `auth { }` only
- `POST /setup`                         — one-time admin bootstrap; refuses once any user exists
- `POST /logout`                        — deletes the server-side session
- `GET  /users` (+ create/delete POSTs) — built-in user management, admin role only
- `GET  /runtime.js`                    — the DB-stored `pgapp` JS runtime
- `GET  /chart-lib.js`                  — the active pluggable chart library's JS (404 when `chart_lib` is the built-in `inline`)

A `Form` switches into edit mode for one row via `?edit_<n>=<id>` on its
page's URL (`<n>` = the form's 0-based position on the page); a
`Report`'s pagination uses `?r<n>_after=`/`?r<n>_before=` (entity-backed)
or `?r<n>_page=` (query-sourced) the same way — see "Pagination" above.

## Roadmap (not in this slice)

- Multi-app routing (`/:app/:page`) instead of one app per process
- More field types and real relationships (foreign keys) — named
  queries cover ad hoc joins/filters today, but there's no schema-level
  concept of one entity referencing another yet
- A real drag-and-drop builder UI that edits the markup/metadata
- `action`/`flow` blocks in the markup for pluggable server-side logic
  (theming, charts, icons, the CSS/JS asset hooks, nav/components/
  linking, named queries, the JS runtime, and item types are the
  pluggable extension points so far; actions and flows are next —
  likely a second runtime.js convention plus a way to declare which
  item/event triggers which named query or action)
- Field-level authorization (page-level `requires:` exists; hiding
  individual columns/fields by role doesn't yet), plus password change/
  reset flows — today a forgotten password means an admin deletes and
  recreates the account
- Login sessions are plain HTTP-compatible (no `Secure` cookie
  attribute), fine for localhost; a real deployment behind TLS should
  add it
- Item type config is currently always flat strings (`serde_json::Value`
  values are strings even for Slider's numeric-looking `min`/`max`); a
  component that wanted structured config (nested objects, arrays of
  objects) would have to encode/decode that itself
- Migrating existing data tables when field definitions change beyond
  adding a column: `ensure_data_table` now runs `ADD COLUMN IF NOT
  EXISTS` for new fields on an existing table (without `NOT NULL`, to
  avoid breaking on existing rows), but doesn't handle renames, type
  changes, or drops
- Separate field lists for create vs. edit forms, so e.g. a `readonly`
  field with a meaningful default doesn't get nulled out on create
- Region/LOV resolution shares one `RegionRows` map per request keyed
  only by query name; a page-scoped query and an app-scoped one (used
  by header/footer) that happen to share a name on the same request
  would collide there — rare, and documented rather than guarded against
- No compile-time or startup-time validation of a named query's SQL
  beyond the bind-marker scan — a typo in `sql` surfaces as a runtime
  error the first time that query actually runs
- A `Report`'s edit/delete row actions only wire up to a `Form` bound to
  the *same entity on the same page*; a `Report` that should edit
  through a form on a different page isn't supported yet
- CSS-class icon packs whose stylesheet is a remote CDN URL (Font
  Awesome, Material Icons) need outbound network access to actually
  render glyphs in the browser — the mechanism (class name + stylesheet
  link) is wired and verified, but a network-restricted environment
  shows blank icons until that stylesheet loads
