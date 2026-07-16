# pgapp

An Oracle APEX-inspired, no/low-code application framework built on
Postgres, written in Rust.

## Idea

- **In-database metadata**: applications, entities, fields and pages are
  rows in Postgres (`pgapp_meta.*`), not config files scattered across a
  repo. The server always serves off the database, not off whatever was
  last parsed.
- **A textual markup language** (`.app` files, APEX-flavored) is how you
  *author* an application. It's parsed once and synced into the metadata
  tables — after that, the database is the source of truth.
- **Low-code CRUD**: given an entity and a page definition, pgapp
  generates the data table, the list view, the create/edit forms, and a
  JSON endpoint, with zero per-app code.
- **Pluggable design system**: rendered HTML only ever uses a fixed set
  of `.pgapp-*` classes; a swappable `theme.css` (see "Theming" below)
  gives them their actual look. `assets/app.css`/`app.js` still exist as
  a per-app override layer on top of whatever theme is active.
- **Named queries**: reusable SQL, declared once (app-wide or scoped to
  one page) and referenced by name from LOVs, regions, and a list page's
  row source — see "Named queries" below.
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
(drag-and-drop builder UI, actions/flows, multi-app routing, auth) on
top of it.

## Markup

```text
app "Todo" {
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
    item "More" {
      item "About" -> page About
    }
  }

  # App-scoped: visible from every page's LOVs/regions.
  query assignees {
    sql: "select distinct assignee as value from pgapp_data.todo_tasks where assignee is not null order by 1"
  }

  entity "tasks" {
    field id: id
    field title: text required
    field priority: text default Medium
    field done: boolean default false
    field assignee: text
    field notes: text
    field created_at: timestamp default now
  }

  page "Tasks" as list of tasks {
    columns: title, priority, done, created_at
    form: title, priority, done, assignee, notes
    link: title -> page TaskDetail (priority: priority)
    item priority as radio ("Low", "Medium", "High")
    item assignee as popup from query assignees
    item notes as readonly

    # Page-scoped: only this page's items/LOVs can see "recent".
    query recent {
      sql: "select id, title, priority, done from pgapp_data.todo_tasks order by id desc limit 5"
    }
    items {
      text "Manage your tasks below. Click a title to see its detail page."
      region "Recently added" from query recent
    }
  }

  page "TaskDetail" as detail of tasks {
    query siblings {
      sql: "select id, title from pgapp_data.todo_tasks
            where priority = :priority::text and id != :id::integer
            order by id"
    }
    items {
      region "Other tasks with the same priority" from query siblings
    }
  }

  page "OpenTasks" as list of tasks {
    query open {
      sql: "select id, title, priority, assignee from pgapp_data.todo_tasks
            where done = false order by id"
    }
    source: query open
    columns: title, priority, assignee
    form: title, priority, done, assignee, notes
  }

  page "About" as static {
    items {
      text "pgapp is an Oracle APEX-inspired no/low-code framework built on Postgres."
      link "Back to tasks" -> page Tasks
    }
  }
}
```

- `header { }` / `footer { }` (optional, top-level) declare app-wide
  chrome shown on every page — the same `text`/`link`/`region` items as
  page `items` (below), just scoped to the whole app instead of one
  page.
- `nav { }` (optional, top-level) declares the app's navigation bar.
  Each `item "Label"` is either a leaf (`-> page <Name>`) or a group
  (`{ ... }` of nested items) — nesting groups gives you a multi-level
  menu, rendered site-wide.
- `entity` blocks describe a table: each `field` has a type, and
  optionally `required` and/or a `default`.
- Pages come in three kinds:
  - `page ... as list of <entity>` — the CRUD list/create/edit page from
    before. `columns` are shown in the list view, `form` are editable in
    create/edit forms. Fields left out of both (like `created_at` above)
    are still stored, just not user-editable — Postgres fills them via
    column defaults.
  - `page ... as detail of <entity>` — a read-only single-row page,
    selected via `?id=` on its URL. Shows every field on the entity.
  - `page ... as static` — no entity at all, just `items`.
- `link: <field> -> page <Name>` (list pages only) turns that report
  column into a link to another page, passing the row's id as `?id=`,
  optionally forwarding other columns as extra query parameters via
  `(field: param, ...)` — see "Named queries" for what those parameters
  are *for*.
- `items { }` (any page kind) adds `text "..."`, `link "Label" -> page
  <Name>`, and/or `region "Label" from query <name>` to the page body —
  content that isn't tied to the entity's table/form.
- `item <field> as <type>` (list pages only) picks how a form field is
  presented — a "page item type" in APEX terms. Types:
  - `text` — a plain input (the default for text/integer/timestamp).
  - `readonly` — displays the value but isn't editable; its value still
    round-trips via a hidden input, so it survives an update. Since
    create and edit share one form, avoid `readonly` on a field whose
    column default matters at creation time (there's no prior value to
    show or resubmit yet).
  - `checkbox` — a real `<input type=checkbox>` (the default for
    boolean fields, replacing the old true/false dropdown).
  - `radio (...)` / `popup (...)` — a radio group / a "Pop Up LOV" (a
    button opening a native `<dialog>`), each sourced from either a
    fixed `("A", "B", ...)` list or `from query <name>` (see below).
  - Fields left undeclared get `FieldItemType::default_for` their
    column type (see `src/model.rs`) — `id` fields default to
    `readonly`.
- Anything that targets a page by name (`nav` items, `link:`, `link`
  items) uses a bare identifier, not a quoted string — restricting link
  targets to the same safe charset as SQL identifiers. Query names are
  the same way.

See `src/markup.rs` for the full grammar and `examples/todo.app` for a
working example.

## Named queries

A `query <name> { sql: "..." }` block is reusable SQL, referenced by
name from a `radio`/`popup` item type (`from query <name>`), a page
`region` item, or a list page's `source: query <name>`. Two scopes:

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
a value on one page reaches a query on another: `link: <field> -> page
<Name> (field: param)` forwards a row's column as `?param=...` on the
target page's URL, where its named queries can read it as `:param`. The
"other tasks with the same priority" region above demonstrates this —
try changing the forwarded `?priority=` in the URL and the region's
results change with it, independent of the row actually being viewed.

`radio`/`popup` queries must alias their columns `value` and, optionally,
`label` (defaulting to `value` if omitted) — e.g. `select distinct
assignee as value from ...`. A list page's `source: query <name>` needs
an `id` column plus whatever the page's `columns`/`form` reference by
name; writes (create/update/delete) still always target the underlying
entity by id, regardless of what the report itself selects. A `region`
item has no column requirements — it renders whatever the query returns.

Query SQL isn't translated from logical entity names — it references
the entity's real physical table (`pgapp_data.<app slug>_<entity
slug>`, printed at startup for each page). This also means query results
are decoded generically (via Postgres's `to_jsonb`) rather than through
the same typed pipeline as entity-bound CRUD, so there's no column-type
checking on a query's own SELECT list beyond what Postgres itself
enforces.

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
 .app markup file
        │  markup::parse_app
        ▼
    AppDef (in memory)
        │  meta::sync_app
        ▼
 pgapp_meta.* tables  ──creates──▶  pgapp_data.<table> (the real data table)
        │  meta::load_app (reloads from the DB, not from AppDef)
        ▼
    RuntimeApp
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

## Theming

pgapp doesn't hardcode a look. Every server-rendered element carries one
of a fixed set of classes — `pgapp-nav`, `pgapp-link`, `pgapp-title`,
`pgapp-subtitle`, `pgapp-table`, `pgapp-form`, `pgapp-field`,
`pgapp-label`, `pgapp-input`, `pgapp-select`, `pgapp-btn` (+
`pgapp-btn-primary` / `pgapp-btn-destructive` / `pgapp-btn-secondary`),
`pgapp-inline-form`, `pgapp-alert` (+ `pgapp-alert-error`), `pgapp-list`,
`pgapp-navbar` (+ `pgapp-navbar-item` / `pgapp-navbar-label` /
`pgapp-navbar-submenu`), `pgapp-items`, `pgapp-text`, `pgapp-header`,
`pgapp-footer`, `pgapp-checkbox`, `pgapp-readonly`, `pgapp-radio-group`
(+ `pgapp-radio-option`), `pgapp-popup` (+ `pgapp-popup-dialog` /
`pgapp-popup-list`), `pgapp-region` (+ `pgapp-region-title`) — and
nothing else. A **theme** is what gives those classes an actual
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

Select a theme with `PGAPP_THEME=<name>` (default: `shadcn`). If
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
with `PGAPP_THEME=<name>`. No Rust or markup changes needed — theming is
fully decoupled from both.

## Running it

Requires a reachable Postgres instance.

```bash
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/pgapp
createdb -U postgres pgapp   # if it doesn't exist yet

cargo run                     # serves examples/todo.app on 127.0.0.1:8080
# or: cargo run -- path/to/your.app
```

On startup it prints the URL for each page, e.g. `http://127.0.0.1:8080/Tasks`.

- `GET  /`                        — index of pages in the app
- `GET  /:page`                   — a `list` page (table + "add new" form), a
  `static` page (items only), or a `detail` page (needs `?id=`)
- `POST /:page`                   — create a row (`list` pages only)
- `GET  /:page/:id/edit`          — edit form (`list` pages only)
- `POST /:page/:id/update`        — update a row (`list` pages only)
- `POST /:page/:id/delete`        — delete a row (`list` pages only)
- `GET  /api/:entity`             — JSON list for that entity
- `GET  /runtime.js`              — the DB-stored `pgapp` JS runtime

## Roadmap (not in this slice)

- Multi-app routing (`/:app/:page`) instead of one app per process
- More field types and real relationships (foreign keys) — named
  queries cover ad hoc joins/filters today, but there's no schema-level
  concept of one entity referencing another yet
- A real drag-and-drop builder UI that edits the markup/metadata
- `action`/`flow` blocks in the markup for pluggable server-side logic
  (theming, the CSS/JS asset hooks, nav/page items/linking/item types,
  and now named queries + the JS runtime are the pluggable extension
  points so far; actions and flows are next — likely built as a second
  runtime.js convention plus a way to declare which item/event triggers
  which named query or action)
- Auth/roles at the page and field level
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
