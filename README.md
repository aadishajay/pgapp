# pgapp

An Oracle APEX-inspired, no/low-code application framework built on
Postgres, written in Rust.

## Quickstart

All you need is Postgres reachable somewhere (any role that can
`CREATE DATABASE`) and Rust installed. `pgapp` itself talks to Postgres
directly (via `sqlx`, no client binary required), but the `psql`/
`createdb` client tools are still handy for the seed-data steps below —
on macOS via Postgres.app or Homebrew's `libpq`/`postgresql` formulas,
they aren't on `PATH` by default, so add them yourself (e.g.
`export PATH="/Applications/Postgres.app/Contents/Versions/latest/bin:$PATH"`
or `brew link --force libpq`). Build and install the `pgapp`
binary once — cargo's only job here is compiling it, every command
after this is `pgapp` itself, never `cargo run`:

```bash
cargo install --path .
```

(installs `cargo-pgapp` too, reachable as `cargo pgapp` — see
"Scaffolding a new app"; `cargo install --path . --bin pgapp` skips
that second binary if you don't want it.)

Every app pgapp serves lives in exactly one **instance** (a Postgres
database) → **workspace** (a schema its tables live in) → **app** (a
registered `.pgapp` file) — there's no lighter-weight mode than this,
but each step is one command:

```bash
pgapp instance init                          # once, ever — one instance per machine
pgapp workspace create --schema <name>       # once per schema (creates it if missing)
pgapp app create --workspace <slug>
```

`instance init` prompts for a superuser-capable connection string, the
database name (auto-created if it doesn't exist yet), a Postgres
password for the new `pgapp_admin` role, and a separate local CLI admin
password (see "Instance mode" below for what each one guards).
`workspace create` sets up the schema an app's tables will live in.
`app create` scaffolds a starter app (name, theme, single file or
directory) the same way `pgapp new` does, then registers it into the
workspace you picked and prints the exact next command:

```bash
pgapp run <generated-file>.pgapp --workspace <slug>
```

It prints the app's URL — `http://127.0.0.1:8080/<workspace>/<slug>` —
and, as long as this is the *only* app registered, opening the bare
`http://127.0.0.1:8080` redirects there too. Once a second app is
registered (see "Multi-app routing" below), `/` instead serves a plain
index page listing every registered app as a link — there's no single
app left to redirect to.

For scripts/CI, skip the prompts entirely: `pgapp new <AppName>`
scaffolds a `.pgapp` file with no database interaction (see
"Scaffolding a new app" below), then register it explicitly with `app
create`/`run` above. To try the richer bundled demo instead of a blank
scaffold, point `run` at `examples/helpdesk.pgapp` — but its
`call_function` action needs a PL/pgSQL function that has to exist
*before* the first sync, so run these in order (`$DATABASE_URL` isn't
set by pgapp itself — export it as the same connection string you gave
`pgapp instance init`):

```bash
export DATABASE_URL=postgres://user:pass@host:5432/<dbname>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/helpdesk_functions.sql
pgapp run examples/helpdesk.pgapp --workspace <slug>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/helpdesk_seed.sql   # after, once the tables exist
```

Running `pgapp run` again with a *different* `.pgapp` path adds a
second app alongside the first, in the same or a different workspace —
see "Multi-app routing" below.

`examples/venpay.pgapp` is a third demo: a vendor-payment tracker hand-
ported from a real Oracle APEX application export, showcasing the
migration approach in "Migrating from Oracle APEX" below (a joined,
read-only view entity for a report that needs columns beyond its own,
and the `button` component's redirect-with-forwarded-params behavior).
No functions/seed script needed — `pgapp run examples/venpay.pgapp
--workspace <slug>` and add data through its own forms.

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
- **A DB-stored JS runtime**: `/:workspace/:app/runtime.js` is a metadata row, not
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
    sql: "select distinct assignee as value from todo_tasks where assignee is not null order by 1"
  }
  query open {
    sql: "select id, title, priority, assignee from todo_tasks where done = false order by id"
  }
  query by_priority {
    sql: "select priority as label, count(*) as value from todo_tasks group by priority order by 1"
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
      sql: "select id, title, priority, done from todo_tasks order by id desc limit 5"
    }
    region "Recently added" from query recent
  }

  page "TaskDetail" {
    query siblings {
      sql: "select id, title from todo_tasks
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

This merge-by-name is deliberately unopinionated about *where* a
declaration lives — any file, anywhere in the tree, merges the same
way, so an existing directory can be organized however suits the app
(by module, by feature, whatever). `pgapp new --dir` (see
"Scaffolding") follows one specific convention on top of that
mechanism, though: `pages/<name>.pgapp` (one file per page) and
`shared_components/<kind>/<name>.pgapp` (entities today; queries and
auth_schemes too, once an app has any) — the same split-by-kind shape
a real Oracle APEX application export uses
(`pages/page_NNNNN.sql`, `shared_components/<kind>/...`). It's a
starting-point convention the scaffold follows, not something
`src/source.rs` enforces — nothing stops an author from reorganizing
afterward.

`examples/helpdesk-modular/` is the helpdesk app split this way — run
it with `pgapp run examples/helpdesk-modular
--workspace <slug>`; it syncs to the same metadata as the single-file
version.

`examples/nexus-erp/` pushes the same mechanism to scale: a 200-page,
60-entity app modeling a medium company's core systems (CRM, sales,
inventory, purchasing, HR, finance, projects, support, marketing,
operations, facilities, compliance) across 15 files under one
`app.pgapp` — every entity gets a list+form, detail, and quick-edit
page, plus 12 module dashboards, cross-module reports, and admin
pages. Seed it with `examples/nexus-erp/seed.sql` after the first sync
(run `pgapp run examples/nexus-erp --workspace <slug>` first so the
tables exist, then — with `$DATABASE_URL` exported to the same
connection string you gave `pgapp instance init`, since pgapp itself
never sets it — `psql "$DATABASE_URL" -v schema=<workspace_schema> -f
examples/nexus-erp/seed.sql`). It's also the fixture used to
confirm the server holds up under load: 30 parallel threads sweeping
all 200 pages sustained ~900 req/s with zero errors (p50 ~27ms, p99
~94ms on a single shared connection pool), and 30 threads doing
concurrent writes across 4 entities landed every row with zero
conflicts.

## Components

A page's body is `Vec<ComponentDef>` (`src/model.rs`) — there's no
fixed "page kind." Eleven kinds:

- **`report "Title" of <entity> { ... }`** — read-only, paginated
  table. `columns`; `source: query <name>` sources rows from a query
  instead (writes still target the entity by id); `link: <field> ->
  page <Name> (extra: param, ...)` links a column, forwarding the row's
  id plus extra params; `page_size` (default 20); `display: table |
  cards | list` (default `table`) — Oracle APEX's separate Card and List
  regions, folded in as a display mode since everything else about a
  report (entity binding, pagination, search/filter, the sibling-Form
  edit/delete wiring) stays identical, only the per-row markup changes.
  A sibling `Form` for the same entity on the same page gets automatic
  Edit/Delete actions in every display mode. In `table` mode, every
  column header is a clickable Interactive-Report-style sort link
  (`?r<n>_sort=<col>:asc|desc`) — clicking toggles that column's
  direction and shows a ▲/▼ indicator; sorting an entity-backed report
  switches it from keyset to offset pagination for the duration (a
  custom order breaks the keyset cursor's `id`-order assumption), and a
  saved view remembers its sort along with its filters. `aggregate
  <col>: sum | avg | count | min | max` (repeatable, one per column)
  adds Interactive Report's footer aggregates row — computed over the
  report's whole *filtered* result set (every page, not just the one on
  screen), independent of pagination and sort; a `format:` mask on the
  same column also formats its aggregate value. Works for entity- and
  query-backed reports; not yet for collection-backed ones (an internal
  pgapp extension, not something a real APEX migration needs).
  `break_on: <col>` adds Interactive Report's Control Break: consecutive
  rows sharing that column's value show it only on the first row of the
  group (table display mode only); a report with `break_on` and no
  explicit column-header sort defaults to sorting by that column
  ascending, since the rows need to actually be grouped together for
  the break to read as intended. `highlight (when: "<sql bool expr>",
  color: "<css color>")` (repeatable) adds Interactive Report's row
  highlighting: `when` is a raw SQL boolean expression referencing
  `t.<field>` (same trust level as a `computed` column), evaluated per
  row; the first rule (in declared order) whose expression is true wins
  and colors that row's background. Entity-backed reports only (same
  restriction as `computed` columns). Every report also gets a
  "Download CSV" link (`GET .../c/<idx>/csv`), no markup needed:
  it streams every row matching the report's *current* filters and
  sort as CSV — not just the page on screen — with the same columns
  and `format:` masks the table shows. `heading <col>: "Label"`
  overrides a column's Classic-Report header (a column not listed
  here just shows its own name); the label also replaces the raw
  column name in the search toolbar's column picker and the CSV
  header row. `align <col>: left | center | right` sets a column's
  Classic-Report alignment (default left).
- **`faceted_search "Title" of <entity> { facet <col> as (checkbox_list | range | date_range) }`**
  — Oracle APEX's Faceted Search: a panel of facets filtering a sibling
  `Report` bound to the same entity, on the same page (found by entity
  match, same convention as a Report's companion `Form`). `checkbox_list`
  shows every distinct value in that column with a live row count
  (recomputed against the Report's own filters plus every *other*
  active facet, so checking a box narrows the rest without making its
  own unchecked options disappear); the column can be any type.
  `range` is a min/max number-input pair over an `integer` column;
  `date_range` a from/to date-input pair over a `timestamp` column.
  Selecting any combination of facets ANDs them into the Report's row
  fetch, its footer aggregates, and its CSV export alike — the active
  facet selection also survives the Report's own column-header sort,
  pagination, and CSV download links (though not a saved view, which
  doesn't yet capture facets). Entity-backed reports only, same
  restriction as `computed`/`highlight`.
- **`form "Title" of <entity> { ... }`** — create/edit form. `fields`
  lists writable columns; `item <field> [as <kind> [(...)]] [attrs
  (...)]` picks a widget and/or sets `id`/`class`/attributes on that
  field's wrapper (at least one of the two required). Blank by default;
  `?edit_<n>=<id>` switches it into edit mode with a Delete button.
  `after_save -> page <Name> (field: param, ...)` — Oracle APEX's
  Branch after a DML process: redirects there instead of the default
  (back to the same page/anchor) after a successful create or update,
  forwarding the just-saved row's own field values as query params
  under new names (same `(field, target param)` shape as a report's
  `link:`). `field` must be `"id"` or one of this form's own `fields:`
  — the only data available to forward without a second DB round trip.
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
- **`dynamic_content "Label" calls <module> (config...)`** — Oracle
  APEX's "PL/SQL Dynamic Content" region: the named action module's
  returned string, rendered once per page load as trusted HTML (not
  escaped — the app author wrote the module). Validated against the
  action registry at sync time, same as `action` below; a module that
  fails at runtime shows its error inline instead of failing the whole
  page (same soft-fail precedent as `Report::before_load`).
- **`calendar "Title" of <entity> { date: <field> title: <field> [link: page <Name>] }`**
  — Oracle APEX's Calendar region: a month grid, one entry per row of
  `<entity>` bucketed by `date` (cast to `date`, so a `timestamp` field
  works too); `title` names the field shown on each entry; `link:`, if
  given, makes each entry a link to that page forwarding the row's id.
  Read-only, unpaginated (a month only ever has 28-31 cells), and always
  sourced from the entity's own data table — no `source: query` or
  collection-backed entities, same restriction as `Form`/`EditableTable`.
  Prev/Next controls step a `?cal<n>=YYYY-MM` query parameter one month
  at a time, defaulting to the current month when absent.
- **`map "Title" of <entity> { lat: <field> lng: <field> title: <field> [link: page <Name>] }`**
  — Oracle APEX's Map region: a dependency-free inline-SVG scatter of
  one entity's rows, each plotted by (`lat`, `lng`) under a simple
  equirectangular projection — no external mapping library or tile
  server, the same "no CDN" precedent as `chart_lib`'s inline SVG
  charts. `title` labels each marker (a hover tooltip); `link:`, if
  given, makes each marker a link to that page forwarding the row's id.
  Read-only, unpaginated, always sourced from the entity's own data
  table (same restriction as `Calendar`), and skips any row whose `lat`
  or `lng` is null.
- **`action "Label" calls <module> (config...)`** — a button running a
  server-side action module; see "Server-side actions".
- **`button "Label" -> page <Name> (extra: param, ...)`** or **`button
  "Label" calls <module> (config...)`** — a standalone button
  independent of any report row: the first form redirects, forwarding
  the current page's own query-string values under new names (same
  shape as a report's `link:` above, but from the page's own context
  instead of a row); the second runs a server-side action, identical to
  `action` above. Two behaviors, one component kind, so the App
  Builder's "Add Component" panel only needs one entry for both — the
  closest pgapp equivalent to Oracle APEX's button (`redirectThisApp`
  vs. a process-submitting button; see "Migrating from Oracle APEX"
  below).
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

## Computed columns & format masks

A `report` over an entity-backed table (no `source:`) can add its own
extra, read-only columns and reformat the display of any column —
APEX's "computed/derived column" and "format mask" concepts:

```text
report "Vendor Bills" of vendor_bills {
  columns: bill_key, amount, tax, due_date
  computed tax: "(t.amount * 0.08)"
  format amount: currency
  format tax: number(2)
  format due_date: date("%m/%d/%Y")
}
```

- **`computed <name>: "<sql>"`** — a scalar SQL expression evaluated in
  the same `SELECT` as the entity's own columns, aliased to `<name>`
  and cast to text like every other column. It may reference the
  current row via `t.<field>` (`t` is the entity table's alias),
  including inside a correlated subquery — e.g. summing a child table
  filtered on the parent's own key. `<name>` must not collide with a
  real field name, and `computed` only applies to entity-backed reports
  (a `source: query` report already runs arbitrary SQL — add the
  expression to that query instead). Once declared, a computed column
  is a first-class report column: it can appear in `columns:`, gets
  its own header, and participates in both the free-text search and the
  single-column filter (filtering resolves it back to its own SQL
  expression rather than a real column, since it isn't one).
- **`format <column>: <mask>`** — reformats that column's raw text
  value at render time only; never touches what's stored or what a
  `form` submits, and a value that doesn't parse the way the mask
  expects (non-numeric text under `currency`, say) renders unchanged
  rather than erroring. `<column>` must already be in the report's
  `columns:` list. Four masks: `currency` (`$1,234.56`), `number` or
  `number(<decimals>)` (thousands-grouped, `number` alone means 0
  decimals), `percent` (rounds to an integer, trailing `%`), and `date`
  or `date("<pattern>")` (reformats an ISO-ish `YYYY-MM-DD` value using
  a small strftime-like subset — `%Y`/`%y`/`%m`/`%d`/`%B`/`%b` — default
  pattern `%Y-%m-%d`, a no-op).

## Query-backed entities

`entity "name" from query <name> { field ... }` — a **read-only**
entity backed by a named query instead of a physical table (the APEX
"view" pattern). No table created; binding a `form`/`editable_table` to
it is a sync-time error; reports over it paginate by `OFFSET`.

## Collections

`entity "name" from collection "name" { field ... }` — an APEX-
Collection-style **read-only** entity backed by a scratch row store
(`pgapp_meta.collections`, a `jsonb` blob per row) instead of a
physical table or a query. Nothing to create at sync time; like
query-backed entities, binding a `form`/`editable_table` to one is a
sync-time error, and reports over it paginate by `OFFSET`.

Collections exist to hold data that didn't come from a table in the
first place — most often an external API response. The `http_request`
action writes to one directly: give it a `collection: "name"` config
and, on a successful (2xx) JSON response, it stores the body instead
of just echoing it back — a JSON array becomes one row per element, a
bare object becomes a single row. `collection_mode: "replace"`
(default) clears any existing rows under that name first in the same
transaction; `"append"` keeps them and continues the row numbering:

```text
entity "products" from collection "products" {
  field name: text
  field price: text
}

report "Products" of products {
  columns: name, price
}

action "Refresh" calls http_request (
  url: "https://api.example.com/products",
  collection: "products"
)
```

**Isolation**: every row is scoped to the browser that wrote it, never
shared. This has nothing to do with login — even an app with no
`auth {}` block gets it, via a dedicated `pgapp_caller` cookie (a
random ID, distinct from the session cookie, minted on first request
and unrelated to whether the visitor is authenticated). The SQL that
reads a collection back is generated by pgapp itself, never
author-written, specifically so the caller-scoping `WHERE` clause
can't be omitted or bypassed by an app's own markup — unlike a named
query, there's no SQL text an app author could get wrong here. Two
different visitors populating a collection under the identical name
see two disjoint sets of rows.

A collection only ever holds what the last action run into it — nothing
refreshes it automatically unless a `report`'s `before_load` says so
(see below).

## Pre-load actions

A `report` may declare `before_load:`, which runs a server-side action
module automatically, on every request, immediately before that report
fetches its rows — the same `ServerAction` registry and `ActionContext`
an `action` component uses, just triggered by a page load instead of a
click:

```text
entity "products" from collection "products" {
  field name: text
  field price: text
}

report "Products" of products {
  columns: name, price
  before_load: http_request (
    url: "https://api.example.com/products",
    collection: "products"
  )
}
```

This is what turns a collection from "shows whatever the last button
click captured" into "always shows fresh data" — no separate refresh
click needed, and no dynamic-action wiring either. `before_load` isn't
limited to `http_request`; it's any registered action module, so a
`call_function` that recomputes something server-side works the same
way.

**Failures are non-fatal.** An unreachable third-party API shouldn't
take the whole page down: if `before_load` fails, the report still
renders with whatever data already exists (from an earlier successful
run, or empty if there's never been one), with the error shown as an
inline warning above the table instead of blocking the page.

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
Ships seven modules:

- **`run_query`** — executes a named query raw (may be a plain
  `UPDATE`/`DELETE`/`INSERT`); binds are still `:name` markers, never
  interpolation.
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
  a collection instead of just echoing it back — see [Collections](#collections).
  A fixed credential (an API key, a service token) that isn't
  user-typed belongs in `{{secret.<name>}}` instead of a literal in the
  markup — see [Secrets](#secrets).
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

### Ajax callback

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
date.rs      <input type=date> for a text field storing "YYYY-MM-DD";
             optional min/max config bound the pickable range
select.rs    <select> over args.choices ("Select List")
switch.rs    boolean toggle, same read_value as checkbox ("Switch")
password.rs  <input type=password>, autocomplete=new-password
color.rs     <input type=color>, falls back to #000000 if invalid
timestamp.rs <input type=datetime-local>; converts timestamptz's
             "YYYY-MM-DD HH:MM:SS+TZ" to/from the control's "T" format
textarea.rs  <textarea>, optional rows config (default 4)
checkbox_group.rs  checkboxes over args.choices, comma-joined value
             via hidden input + pgapp.syncCheckboxGroup ("Checkbox Group")
star_rating.rs     click-to-rate stars, optional max config (default 5)
list_manager.rs    add/remove free-text list, comma-joined value
shuttle.rs   dual <select multiple> with move buttons, comma-joined
             value in selected order
rich_text.rs contenteditable + execCommand toolbar ("Rich Text Editor");
             read_value runs submitted HTML through ammonia::clean
             (allow-list sanitizer) before it's ever persisted
file_browse.rs  "File Browse" — <input type=file>, uploads via a dedicated
             multipart route (not the universal Form extractor); the
             stored value is "<file_uploads id>:<filename>"
```

Adding one: write `src/item_types/<name>.rs`, add one line to
`registry()` — nothing else changes, since everything goes through the
registry by kind string and a generic JSON config; `date.rs` is the
smallest real example. Compile-time plugin point (rebuild + restart,
not hot-loaded); a misspelled `kind` is caught at sync time.

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

**`inline`'s bar/pie/donut marks are colored per category**, cycling
through eight `--chart-1`…`--chart-8` CSS custom properties (a theme
sets these the same way it sets any other design token — see
`themes/shadcn/theme.css`) so a bar chart's bars and a pie/donut's
slices are actually distinguishable instead of one flat `currentColor`
fill; `line`/`area`/`scatter` stay a single accent color, since those
read as one series rather than discrete categories. A theme that
doesn't define `--chart-N` still renders distinct colors — every
`var(--chart-N, ...)` carries the same validated 8-hue fallback (see
`src/render.rs`'s `CHART_PALETTE`) as its default. **Every mark also
carries a native SVG `<title>`**, so hovering any bar, slice, or
line/area/scatter point shows its category and value as a plain
browser tooltip — no JS needed for that either.

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
references the entity's real physical table (`<app>_<entity>`, printed
at startup) and is decoded generically via `to_jsonb`, so there's no
compile-time column-type checking beyond what Postgres itself enforces.

**Write the table name bare (`<app>_<entity>`), never schema-qualified.**
Every connection a named query runs on — at sync time (type inference)
and at request time alike — has its `search_path` pinned to this app's
own `data_schema` first (its workspace's own schema; see [Instance
mode](#instance-mode)). A schema-qualified reference still works
(qualified names ignore `search_path` entirely) but stops working the
moment the app is re-registered into a different workspace — its
tables move, but a hardcoded schema prefix doesn't.

## Authentication & authorization

Opt in with an `auth { }` block:

- Every page requires a signed-in user; only `/:workspace/:app/login` and static
  assets stay public.
- First run bootstraps the admin (`POST /:workspace/:app/setup`, one-time); after
  that, admins manage accounts on the built-in `/:workspace/:app/users` page.
  Users are never declared in markup.
- Passwords are argon2id hashes in `pgapp_meta.users`; sessions are
  server-side rows in `pgapp_meta.sessions` (an HttpOnly, `SameSite=Lax`
  cookie holds only a random token — revoking a session means deleting
  its row).
- A user holds any number of `roles` (free-form strings, comma-separated
  on the Add-user form). `requires: <role>` gates a page (and its
  create/update/delete routes) to a user holding that role, or admin.

Without `auth { }`, an app stays fully public
(`examples/todo.pgapp`); `examples/helpdesk.pgapp` runs behind a login
with an admin-only Agents page.

### Component-level `requires:` and named auth schemes

`requires:` isn't only a page property — any component (`button`,
`form`, `report`, `action`, ...) can carry its own trailing
`requires: <role>`, same slot as `attrs (...)` and combinable with it,
in either order:

```text
button "Approve" calls approve_bill requires: finance
form "Edit rates" of rates { fields: rate } requires: finance attrs (class: "wide")
```

A component a user doesn't have the role for is simply left out of the
page (same "just don't show it" precedent as a role-gated nav item) —
and its own create/update/delete/run route independently re-checks the
same role server-side, so hiding the button is never the only thing
standing between an unauthorized user and the write.

A `requires:` name can also be a reusable **auth scheme** — a named
role group declared once at app scope:

```text
auth_scheme "can_approve" {
  roles: finance, manager
}
```

`requires: can_approve` then passes for anyone holding *either* role.
This is resolved by name at check time: an app with no matching scheme
just treats the name as a literal role, so introducing schemes is
purely additive and never a breaking change to an existing
`requires:`. This is the pgapp analog of an Oracle APEX Authorization
Scheme (see "Migrating from Oracle APEX") — minus the PL/SQL-expression
and SQL-query scheme types APEX also supports, and minus APEX's
separate Access Control allow-list, neither of which pgapp apps have
needed yet.

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
shows on the reload page. The page itself offers an editable textarea
(single-file apps) or "reload from disk" (directory apps), gated to the
`admin` role when auth is enabled.

Not covered: new item types/actions, or the routing table itself, are
still Rust code — those need `cargo build` + restart. Markup changes
(new page/field/entity, a changed `theme:`, a new dynamic action) don't.

## App Builder

`examples/app_builder.pgapp` is a pgapp app, like any other, that lists
every app registered across every workspace in the instance and lets
you drag-and-drop reorder a page's components, add a component of any
of the 13 kinds through a real per-attribute property form (title/
entity/columns/computed columns/format masks/item types/dynamic-action
ops/requires/attrs — whatever that kind supports, as typed fields and
add/remove/reorder row lists, not a raw markup blob), delete it, add/
rename/delete whole pages, jump straight to a live preview, scaffold
brand-new apps, stand up a brand-new workspace from scratch, and — via
each app's own "AppSettings" page — edit its data model
(entities/fields), named queries, navigation menu, and
theme/icons/chart_lib/auth settings, plus delete the app or its whole
workspace outright — an Oracle-APEX-App-Builder-flavored way to build
without hand-editing markup over SSH. What's still Advanced-editor-only
(`header`/`footer` chrome, `auth_scheme` role groups, a directory-based
app's structure) is one click away via the "Advanced" link into the
full-file raw editor every app already has; each component's raw
markup text is also still reachable one click deeper, via its own
"Edit as raw markup" fallback next to the structured editor.

**Available by default, no setup needed.** Every instance auto-provisions
it — at `pgapp instance init` for a new instance, and again (idempotently)
at every `pgapp run` for one that predates this feature — at the fixed
address `/pgapp/builder`, owned by its own reserved `pgapp_builder`
schema (`instance::APP_BUILDER_WORKSPACE_SLUG`/`APP_BUILDER_APP_SLUG` in
`src/instance.rs`), never a user workspace. It owns one real table of
its own (`new_app_requests`, for the "New App" panel below) plus a
handful of query-backed entities reading `pgapp_control.*`/
`pgapp_meta.*` directly (a named query can reference any schema the
shared `pgapp_admin` connection can see — no core changes needed for
that read side). It excludes itself from its own "Apps" listing, and
every mutating admin route on the target side refuses outright (403)
if the target is the App Builder itself — belt and suspenders alongside
the listing's own self-exclusion, since a URL can always be hand-crafted
past whatever a picker declines to link to. Its "Pages"/EditPage screens
also show a small breadcrumb (`bindContextHeader` in `runtime.js`) naming
which app/page is actually being edited, since that's otherwise only
visible in the URL's own query string.

### Editing an existing app's pages

Click through Apps → Pages → a page to reach its editor. Every
mutation here is a small, targeted admin route on *that other app's*
own `/:workspace/:app/admin/pages/...` path (not the App Builder's),
gated the same way `admin/reload` is (the `admin` role, when the
target app has auth enabled), and ends by hot-reloading that one app
in place — no restart:

- **Reorder**: drag a row and drop it — POSTs the new order to
  `.../pages/:page/reorder`.
- **Add page**: a name, on the Pages screen — POSTs to
  `.../pages/add`. Lands empty; add components to it the normal way.
- **Rename page**: per-card pencil button (a themed prompt) on the
  Pages screen — POSTs to `.../pages/:page/rename`. Rewrites the
  page's own declaration *and* every `-> page <old name>` reference to
  it elsewhere in the file (nav items, report `link:`, `link`
  components — see `page_reorder::rename_page`), so nothing dangles.
- **Delete page**: per-card ✕ button (with a confirm dialog) on the
  Pages screen — POSTs to `.../pages/:page/delete`.
- **Add component**: pick a kind (all 13: `text`/`report`/`form`/
  `editable_table`/`chart`/`region`/`action`/`button`/`link`/
  `dynamic_content`/`calendar`/`map`/`faceted_search` — `dynamic_action`
  too, via the structured picker only) to open a blank
  structured form for it (`pgappStructuredEditor` in `runtime.js`) —
  every attribute that kind supports as a real field: scalar text/
  number/select inputs for things like title/entity/query/chart type,
  and add/remove/reorder row lists (`pgappRowList`) for anything the
  grammar itself repeats — a Report's columns/computed columns/format
  masks, a Form's fields/per-field item types, a dynamic action's ops,
  the shared `attrs (...)` extra-attribute list every kind carries.
  Fill it in, Save — the dialog *generates* fresh markup text for that
  one component client-side and POSTs it to
  `.../pages/:page/components/add`, same endpoint the original raw
  editor used. A "Add as raw markup" link next to the kind picker
  reveals the original raw-textarea-plus-starter-template flow, for
  anything the structured form doesn't cover well. The new component
  always lands at the bottom of the page; drag it into place from
  there.
- **Edit**: per-row pencil button opens the same structured editor,
  prefilled from that component's already-resolved, already-typed
  attributes (`GET .../components/:idx/structured`, backed by
  `RuntimeComponent::to_json`) rather than its raw text — a Form's
  `trip_type as popup from query trip_types_lov` shows up as a real
  "popup" dropdown with a "from query" config field already filled in,
  not a string to retype. Save regenerates the whole component's markup
  from the form's current state and POSTs it to
  `.../pages/:page/components/:idx/edit`, replacing the whole block
  (`page_reorder::replace_component`) — genuine APEX-Page-Designer-
  style editing: pick a component, get a property sheet, not a raw text
  box. A `{ }` button next to the pencil opens the original raw-markup
  textarea instead (prefilled via `GET .../components/:idx/source`),
  for anything the structured form doesn't have a dedicated control for
  yet, or to hand-tweak formatting/inline comments the structured
  editor can't preserve (regenerating from typed fields necessarily
  drops any comment that lived *inside* the component's own block,
  since a comment isn't part of its attribute data — a comment
  immediately above the component's own declaration line is untouched
  either way, since that's `page_reorder`'s doing, not the structured
  editor's). Every dropdown (entity, query, page, action, item-type
  kind, chart type, auth scheme) is populated from
  `GET .../pages/:page/app-meta` rather than hand-typed, so a target
  that doesn't exist yet can't be typo'd in.
- **Delete component**: per-row ✕ button (with a confirm dialog) —
  POSTs to `.../pages/:page/components/:idx/delete`.
- **Run this page ↗**: opens the page's real, live URL in a new tab —
  built client-side from this page's own `?target_workspace=`/
  `?target_app=`/`?target_page=`, same params every cross-app action
  here reads.
- **Advanced: edit full app source ↗** (on the Pages screen): a link
  to the target app's own, already-existing `/admin/reload` page — a
  full-file raw markup editor built into *every* app (not something
  the App Builder adds; see "Reloading metadata without a restart"
  below). Entities, queries, nav, header/footer, and app-level
  settings (theme/auth/icons) have no dedicated control in this picker
  — this is how to reach them without SSHing in.

Every add/edit/rename above is validated (`markup::parse_app` on the
whole file, in memory) *before* it's written to disk, so a typo in a
hand-edited component block or a bad rename is rejected with the parse
error instead of corrupting the file.

Every one of these keeps `pgapp_meta` and the target app's own
`.pgapp` file in agreement by construction: the route edits the file
via `src/page_reorder.rs` (a line-based **text splice**, never a
parse-and-regenerate — `markup::page_component_start_lines`/
`app_page_start_lines` and their boundary helpers reuse the real
parser's own page-body/app-body walk, so untouched components and
pages keep their exact original formatting and inline comments; a
comment directly above one, no blank line between, travels with it
when reordered or deleted), then calls `AppEntry::reload()`, which
re-syncs that file straight into `pgapp_meta` (the authoritative
source from that point on — a page or field dropped from the file is
now also deleted from `pgapp_meta`, cascading to its components/saved
views, not just left orphaned) and reloads the in-memory app.
Single-file apps only for now — a directory app's page lives across
more than one file, and splicing across files isn't implemented yet.

The drag itself, the panels, and the per-row/per-card action buttons
are all `runtime.js` (`bindDraggableRows`/`bindAddComponentForm`/
`bindComponentRowActions`/`bindAddPageForm`/`bindPageCardActions`/
`bindAdvancedSourceLink`) — plain HTML5 drag-and-drop, a themed
structured property-sheet modal (`pgappStructuredEditor`, one render
function per kind in `PGAPP_KIND_RENDERERS` plus shared widgets —
`pgappRowList` for repeatable rows, `pgappAttrsEditor`/
`pgappRequiresEditor`/`pgappConfigEditor`/`pgappItemTypesEditor` for
the bits every kind or every field shares) alongside the original raw-
source editor modal (`pgappSourceEditor`, still the "Advanced"/"{ }"
fallback), both built the same way as the existing `pgappPrompt`/
`pgappAlert`/`pgappConfirm` dialogs, and small DOM-built forms injected
into `text` component placeholders (`attrs (id: "...")`), no framework
changes needed to host any of it. Saving in the structured editor never
sends structured data to the server at all — it *generates* markup
text client-side (mirroring the grammar `markup.rs` parses) and submits
that through the exact same raw-text `/components/.../add`|`edit`
routes the original editor already used, so no new write-side route
was needed; only reading a component's current, already-typed
attributes needed one (`GET .../components/:idx/structured`, backed by
`RuntimeComponent::to_json` in `src/meta/types.rs`), plus one more for
its dropdowns' contents (`GET .../pages/:page/app-meta`). Since these
all describe some *other* app's page, every one of them builds that
app's own URL from `?target_workspace=`/`?target_app=`/`?target_page=`
on the **current** (App Builder) page's own URL, not from anything
baked into the markup — forwarded page-to-page the same way any other
cross-page parameter is (a report's `link: <field> -> page <Name>
(<row column>: <param name>)`).

### Creating a brand-new app

The "New App" page scaffolds a fresh single-file app (a starter
`items` entity + page, the same shape `pgapp new` generates) into an
existing workspace — name, target workspace (picked from a list,
excluding the App Builder's own reserved workspace), and theme. Submit
and the same page reloads already processed and **already live**: a
Form writes a pending row, `runtime.js`'s `bindNewAppProcessing` POSTs
to `/pgapp/builder/admin/apps/create-pending` on every load of the
NewApp page (a harmless no-op when nothing's pending), which scaffolds
the file, syncs it into `pgapp_meta`, registers it in `pgapp_control`,
*and* hot-registers it into the running server's `AppState` (see
`AppState::register_app`/`AppEntry::load` in `server.rs`) — so it's
reachable immediately, no `pgapp run` restart needed. This isn't a
`before_load` action like every other "process something automatically"
case in pgapp: hot-registering needs `AppState` access, which action
modules don't have (see `actions/create_app.rs`'s own doc), so this is
a dedicated route instead. Errors (bad theme, unknown/disabled
workspace, a slug collision) land in that same row's `status`/`result`
columns rather than a page-level warning banner, so they stay visible
on every later load too, not just the one right after submission.

### Creating a brand-new workspace

The "New Workspace" page gives the one remaining piece of `pgapp
workspace create` a web equivalent: schema name, an optional slug
(defaults to the schema name), and a choice of either provisioning a
fresh schema + owning role (a password you set) or attaching to a
schema that already exists elsewhere in this database (a
superuser-capable Postgres connection string, pasted in and used
exactly once to run the grant). Submit and the Workspaces report below
confirms it landed.

This is `src/actions/create_workspace.rs`'s two action modules, not an
entity `Form` — deliberately. A plain `action` component only ever
renders a bare button (`render::action_html`), with nowhere to put
typed fields, so `NewWorkspaceForm` is a `dynamic_content` module that
renders a real `<form>` instead, posting to a sibling hidden `action
... calls create_workspace` component (`attrs (style: "display:
none")` — it exists only to be that POST target). And unlike "New
App"'s pending-request-row pattern, `CreateWorkspace` never writes a
row at all: workspace creation needs no `AppState` hot-registration (a
workspace isn't itself "served," an app inside one is), so there's no
architectural reason to persist anything, which matters here
specifically because the "attach to an existing schema" connection
string is a superuser-capable secret typed into a web form — it lives
only in that one request's in-memory parameter map and is never
written to a database row, logged, or echoed into an error message
(every `control::create_workspace_*` error wraps the underlying cause
in a static `.context(...)` rather than interpolating it, and
`anyhow::Error`'s `Display` only ever shows that outermost message).
`ensure_role`/`grant_admin_on_schema` (in `src/control.rs`) are the
same DDL `pgapp workspace create` itself runs, shared by both so the
CLI and this web form can't drift.

### Editing an app's data model, queries, navigation, and settings

Every app's "AppSettings" page (reached from a "Data Model, Queries,
Nav & Settings →" link on the Pages screen) is the App-Builder
counterpart to APEX's Data workshop, Shared Components, and Edit
Application Properties, all in one place:

- **Data Model**: add/edit/delete entities and their fields (name,
  type, required, default) through the same structured field-list
  editor a Form/Report's own field picker already uses. Adding a
  physical entity provisions its table on the next sync exactly like a
  hand-written `entity { }` block would; adding a query-backed one
  (`from query <name>`) just needs an existing query to point at.
  Renaming an entity, or changing an existing field's type, isn't
  supported here — the former needs rewriting every place that entity
  is referenced (unlike a page, which already has that machinery via
  `page_reorder::rename_page`; entities don't yet), and the latter is
  already a hard sync-time error if it doesn't match the physical
  column, so there's nothing this editor could safely do differently.
  Deleting an entity removes its `pgapp_meta` bookkeeping only — its
  physical table (if it has one) is deliberately left in place, same
  "pgapp adds columns but never changes or drops them" caution
  `meta::sync_app`'s `ensure_data_table` already applies to fields.
- **Queries**: add/edit/delete a named query (name + SQL). Deleting
  one still in use (an entity `from query`, a report/chart/region/LOV
  bound to it) is rejected at the next sync with the same "unknown
  query" error a hand-edit would get.
- **Navigation**: add/edit/delete/reorder the nav menu's *top-level*
  items (label + target page) with plain ▲▼ buttons, same convention
  every other repeatable-row editor here uses. A nested submenu item
  shows up as a single, non-editable row ("edit as raw markup via
  Advanced") — same "not covered yet" treatment as anything else the
  structured editors don't have a dedicated control for.
- **App Settings**: theme/icons/chart_lib pickers plus the `auth { }`
  on/off toggle — APEX's "Edit Application Properties," scoped
  deliberately: an `auth_scheme`'s own role list, and which pages
  `requires:` which role, both stay Advanced-editor-only.

All four are line-splice edits in `src/app_editor.rs`, the entity/
query/nav/settings counterpart to `src/page_reorder.rs`'s page/
component splices — same discipline (reusing the real parser's own
walk to find exact line ranges, so untouched content, including
comments and formatting, survives every edit unchanged), same routes
shape (`/:workspace/:app/admin/{entities,queries,nav,settings}...`,
JSON in/out, validated with `markup::parse_app` before writing, ending
in a hot `entry.reload()` — no restart).

### Deleting an app or its whole workspace

Two "danger zone" panels at the bottom of "AppSettings", each with the
same soft/hard choice `pgapp app destroy`/`pgapp workspace destroy`
already have on the CLI:

- **Delete This App**: soft disables it (reversible — its tables and
  rows are untouched, and re-registering it later reactivates it,
  though on an already-running server this takes effect on the next
  `pgapp run`, not immediately, same as the CLI); hard permanently
  drops its own data tables, needs the app's own slug typed in to
  confirm (mirroring the CLI's confirmation), and unregisters it from
  the live server immediately (`AppState::unregister_app`) so it starts
  404ing right away rather than erroring against now-missing tables. No
  superuser connection needed either way — `pgapp_admin` already owns
  every table it created, in any workspace schema it's been granted
  into (see `control::hard_delete_app`).
- **Delete Workspace**: same soft/hard choice, but for the *whole*
  workspace — every app registered in it, torn down together. Hard
  delete needs the workspace's own slug typed in, **plus** a
  superuser-capable Postgres connection string (dropping a schema/role
  needs privilege beyond what a schema-level grant gives `pgapp_admin`,
  and an "attach to an existing schema" workspace was never owned by it
  to begin with) — used once, in memory, to run the `DROP SCHEMA`/`DROP
  ROLE`, and never persisted, the exact same contract as "New
  Workspace"'s existing-schema attach (see `control::hard_delete_workspace`).

Both routes reuse `admin_edit_guard` (the same self-edit guard and
"borrow the target app's own auth" access check as everywhere else in
the App Builder) — the workspace-destroy route's URL still names an
`:app` even though the *operation* isn't scoped to one, because the
global `auth::require_login` middleware resolves every request's auth
context from the `/{workspace}/{app}/...` shape before any route
handler runs, so a workspace-wide action still needs *some* registered
app in the URL to borrow that context from.

### Migrating from Oracle APEX

There's no automated importer for an APEX application export (the
`apexlang` `.apx` format) — a real export is dozens of files covering
far more ground than pgapp's markup does (per-column report formatting,
shared LOVs/authorization/authentication schemes, processes,
session-state protection, and more), and translating that faithfully
is its own project, not a feature of this one. What pgapp does give you
is a concept-for-concept map, so a hand migration knows where each
piece goes:

| Oracle APEX (`apexlang`)                                   | pgapp equivalent                                                             |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `page N (name, title, ...)`                                   | `page "Title" { ... }` (see "Markup")                                        |
| `region ... (type: interactiveReport, source { tableName })`  | `report "Title" of <entity> { columns: ... }`                                |
| `region ... (type: staticContent)` / a plain query region     | `region "Label" from query <name> { columns: ... }`                          |
| `region ... (type: cards)` / `region ... (type: list)`        | `report "Title" of <entity> { display: cards }` / `{ display: list }` — a display mode on the same `report`, not a separate region kind |
| `region ... (type: calendar)`                                  | `calendar "Title" of <entity> { date: <field> title: <field> }` |
| `region ... (type: map)`                                        | `map "Title" of <entity> { lat: <field> lng: <field> title: <field> }` — an inline-SVG scatter, no external mapping library/tile server |
| a region's `column NAME (heading, layout, appearance, ...)`   | one name in `columns:`, plus `format <column>: <mask>` for display formatting (see "Computed columns & format masks") — no per-column heading override yet (see "App Builder" for what's GUI-editable today) |
| a column's "Derived Column"/computation                       | `computed <name>: "<sql>"` on the report (see "Computed columns & format masks") |
| `region`'s `link { target: { page, items } }`                 | a report's `link: <field> -> page <Name> (extra: param, ...)`               |
| `button ... (behavior { action: redirectThisApp, target })`  | `button "Label" -> page <Name> (extra: param, ...)`                          |
| `button ... (behavior { action: submit }, serverSideCondition { whenButtonPressed })` + a `process` | `button "Label" calls <action_name> (config...)` — write the process's logic as a registered action module (see "Server-side actions") instead of PL/SQL |
| a page item (`P7_TRIP_ID`, etc.)                              | a `Form`/`EditableTable` field, or a cross-page query-string parameter        |
| `dynamicAction ... (when { event }, action { action: ... })`  | `on <event> of <item> { show/hide/toggle/set/refresh }` (native APEX events beyond click/change aren't supported) |
| shared LOV (`shared-components/lovs.apx`)                     | `query <name> { sql: "..." }` referenced inline from `item ... as popup from query <name>` — not a standalone reusable object yet |
| `authentication` scheme                                       | the app-level `auth { }` block (on/off) |
| `authorization` scheme                                        | `requires: <role>` on a page or component, or a named, reusable `auth_scheme { roles: ... }` referenced by name (see "Component-level requires: and named auth schemes") — no PL/SQL-expression or SQL-query scheme types, no separate Access Control allow-list |
| `process` (PL/SQL)                                            | a registered Rust action module (`src/actions.rs`) called from `before_load`, `action`, or a `button calls` |

For each APEX page: create the matching pgapp `page`, then migrate its
regions/reports one at a time, using the App Builder's raw component
editor (see "App Builder" above) to iterate quickly without restarting
the server. Anything with no row in this table (per-column formatting,
LOVs/auth schemes as objects, processes as a distinct type) has no
pgapp equivalent yet — reimplement that logic in the target page's own
query SQL, a registered action module, or a dynamic action, whichever
fits.

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
contain hyphens), `apex_universal` (evokes Oracle APEX's classic
Universal Theme / Theme 42: white regions, a bold title underlined in
signature blue, rectangular low-radius buttons, a plain white top nav
bar, and a light-gray Interactive Report-style table header — no dark
mode, same as the original; used by `examples/venpay.pgapp`).

To add another design system: `themes/<name>/theme.css` + `theme:
<name>` — no Rust changes.

## Routes

Every route lives under `/:workspace/:app` — the workspace an app is
registered into, plus its own URL slug (see "Multi-app routing"). Since
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
- `GET  /:workspace/:app/admin/pages-list`              — every page name in that app, for the App Builder's "Target page" dropdown
- `POST /:workspace/:app/admin/pages/:page/reorder`     — the App Builder's drag-and-drop save (see "App Builder")
- `POST /:workspace/:app/admin/pages/add`                          — the App Builder's "Add Page" (see "App Builder")
- `POST /:workspace/:app/admin/pages/:page/rename`                 — the App Builder's "Rename page" (see "App Builder")
- `POST /:workspace/:app/admin/pages/:page/delete`                 — the App Builder's "Delete page" (see "App Builder")
- `POST /:workspace/:app/admin/pages/:page/components/add`         — the App Builder's "Add Component" (raw markup, any kind — see "App Builder")
- `GET  /:workspace/:app/admin/pages/:page/components/:idx/source` — a component's exact current markup, for the App Builder's "Edit" panel
- `POST /:workspace/:app/admin/pages/:page/components/:idx/edit`   — the App Builder's full-property "Edit" (see "App Builder")
- `POST /:workspace/:app/admin/pages/:page/components/:idx/delete` — the App Builder's per-row "Delete" (see "App Builder")
- `POST /pgapp/builder/admin/apps/create-pending`                  — the App Builder's "New App" processing step (see "App Builder")
- `GET  /:workspace/:app/runtime.js`                    — the DB-stored `pgapp` JS runtime
- `GET  /:workspace/:app/chart-lib.js`                  — the active pluggable chart library's JS (404 when `chart_lib` is the built-in `inline`)

A `Form` switches into edit mode via `?edit_<n>=<id>` (`<n>` = its
0-based position on the page); a `Report`'s pagination uses
`?r<n>_after=`/`?r<n>_before=` (entity-backed) or `?r<n>_page=`
(query-sourced) the same way.

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
a normal single-app directory (see "Scaffolding a new app"), and both
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

## Scaffolding a new app

`pgapp new`/`pgapp create` generates a minimal, runnable starter app —
one entity, one page with the classic Report+Form CRUD pattern, a nav
link to it. Both modes are pure file scaffolders: neither touches a
database, so the generated `.pgapp` file still needs registering into a
workspace (`pgapp app create`, or `pgapp run --workspace` —
see "Instance mode" below) before it's actually served:

```bash
# Non-interactive — for scripts/CI:
pgapp new "My Project"                    # -> my_project.pgapp
pgapp new Inventory inventory.pgapp        # explicit path
pgapp new Inventory --dir --theme vivid    # a directory scaffold instead

# Interactive (prompts for name/theme/single-file-or-directory):
pgapp create
```

`pgapp app create [--workspace <slug>]` runs this same
interactive scaffold and registers the result into a workspace in one
step — the more direct path for anything beyond scripts/CI (see
"Instance mode").

`cargo pgapp create` (the `cargo-pgapp` binary `cargo install --path .`
also installs — see Quickstart) is the exact same `pgapp create`, just
reachable as a `cargo` subcommand for anyone who'd rather type it that
way.

See `pgapp new --help` for every flag.

## Instance mode

The only deployment model: a durable, database-backed instance with a
dedicated `pgapp_admin` Postgres role, and every app registered into
exactly one workspace's own schema (a team's own credentials, different
access grants — not just a separate `pgapp_control` row). There's no
lighter-weight, workspace-less way to run pgapp.

There is exactly **one instance, globally, per machine** (technically
per `PGAPP_HOME` — see below) — not one per database. `pgapp instance
init` refuses if one is already set up (`pgapp instance destroy` first
if you want to point at a different database), and every other
instance/workspace/app/secret/run command needs no `<dbname>` argument
at all: there's nothing to disambiguate, so none of the examples below
ever pass one.

The commands below assume `pgapp` is installed (`cargo install --path
.` — see "Quickstart").

**Instance** = one target database, one dedicated `pgapp_admin`
Postgres login role the server operates as from then on:

```bash
pgapp instance init
```

Prompts for a superuser-capable connection string, the database name,
a password to set for the new `pgapp_admin` role, and a separate local
CLI admin password. The database name can name a **brand-new or an
already-existing** database — either way `instance init` only ever
*adds* to it: a missing database is auto-created, an existing one is
connected to as-is and left otherwise untouched, and either way `pgapp`
only ever creates its own `pgapp_meta`/`pgapp_control` schemas and
`pgapp_admin` role in it — nothing already in that database (other
schemas, other applications' tables) is read, altered, or dropped by
`instance init` itself. Two different secrets, two different fates:

- `pgapp_admin`'s Postgres password is **never written to disk** — a
  one-way hash can't be used to reconnect, so every later command reads
  it fresh from `PGAPP_ADMIN_DB_PASSWORD`.
- The CLI admin password *is* stored, but only as an argon2 hash, in
  the single instance file `~/.pgapp/instance.json` (`0600`, override
  the base directory with `PGAPP_HOME`) — it just gates who's allowed
  to run instance/workspace/app commands against this instance at all,
  checked interactively (or via `PGAPP_CLI_ADMIN_PASSWORD` for
  scripts), and has nothing to do with Postgres auth.

**Workspace** = `(schema, slug)` — `schema` is the actual Postgres
schema an app's data tables live in; `slug` is just the short name you
use to refer to it in every later command (`--workspace <slug>`) and
defaults to the schema name if you don't give one:

```bash
pgapp workspace create [--schema <name>] [--slug <slug>]
```

Whether `schema` is treated as new or existing is **auto-detected**
(`pg_namespace`), not asked: missing → pgapp creates it, with its own
owning login role (password prompted, or `--password` for scripts —
`pgapp_admin` is granted membership + USAGE/CREATE); already there →
pgapp only asks to be granted USAGE/CREATE into it (via a connection
that can actually perform that grant — `pgapp_admin` has no privileges
of its own on a schema it didn't create). Passing neither `--schema`
nor `--slug` falls back to prompting for the schema name only — same
auto-detection either way. An app registered into a workspace gets its
entity tables there — transparently to its own markup: named queries
keep referencing their tables by bare name (see [Named
queries](#named-queries)), never the schema, so the same app runs
unmodified regardless of which workspace it's registered into.

**App** = `(workspace, app slug)` — `workspace` says which workspace's
schema the app's tables live in; `app slug` is the app's own URL
identifier (`/<workspace>/<slug>/...`, unique within that workspace,
not instance-wide) and defaults to a slugified version of the app name
you enter (`"My Project"` → `my_project`) if you don't give one:

```bash
pgapp app create [--workspace <slug>] [--slug <app-slug>]
```

Same prompts as `pgapp new`'s interactive flow (name, theme, file vs.
directory), plus a workspace picker (lists every registered workspace
and lets you choose) when `--workspace` isn't given. Then serve it —
and every other enabled app across the whole instance, same "the
registry decides what's served" rule as multi-app routing:

```bash
pgapp run <file>.pgapp [--workspace <slug>]
```

**Destroy**, for all three, always needs the CLI admin password first:

- `pgapp instance destroy` — always a hard delete: drops every
  workspace schema/role pgapp itself created, `pgapp_meta`/
  `pgapp_control`, the `pgapp_admin` role, and the local instance file.
  Asks for a superuser connection fresh (never stored) and requires
  typing the database name to confirm.
- `pgapp workspace destroy <slug> [--hard|--soft]` — soft just disables
  the registry row (schema/data untouched, reversible); hard drops the
  schema and, if pgapp created it, its owning role too (again via a
  fresh superuser connection) — refuses without an extra typed
  confirmation if apps are still registered in it.
- `pgapp app destroy <slug> [--workspace <slug>] [--hard|--soft]` —
  soft disables; hard drops its entity tables and `pgapp_meta` rows
  (using `pgapp_admin`'s own connection — it already owns whatever it
  created). `--workspace` disambiguates if that slug happens to be
  registered in more than one workspace (see "Multi-app routing").

`pgapp workspace list` and `pgapp app list` show what's currently
registered.

## Secrets

A fixed credential an action needs — an API key, a service account
token — that isn't user-typed and shouldn't sit in plaintext in the
markup file. Managed with the same instance-mode CLI:

```bash
pgapp secret set <name> (--workspace <slug> | --app <slug>)
pgapp secret list (--workspace <slug> | --app <slug>)
pgapp secret rm <name> (--workspace <slug> | --app <slug>)
```

`set` prompts for the value interactively rather than taking it as an
argument (`--value` exists for scripts, but — unlike the prompt — it
lands in shell history and `ps`). Referenced from markup as
`{{secret.<name>}}`, anywhere `http_request` already accepts
`{{item}}` (`url`/`body`/`headers`/`token`/`username`/`password`/
`key_value`):

```text
action "Create ticket" calls http_request (
  url: "https://api.example.com/tickets",
  auth: "bearer",
  token: "{{secret.api_token}}"
)
```

An app-scoped secret shadows a workspace-scoped one of the same name —
same precedent a page-scoped named query already sets over an
app-scoped one. Storage lives in `pgapp_control` (pgapp managing
itself, the same registry `workspace`/`app` commands use), not
`pgapp_meta` — untouched by a markup resync, so a secret survives
every `admin/reload` and app rebuild for free, exactly like the
workspace/app registry itself does.

**Encrypted, never hashed.** A hash is one-way — right for the CLI
admin password above, which is only ever *compared*, but useless for a
secret that has to be sent back out in plaintext (an `Authorization`
header). Secrets are AES-256-GCM encrypted at rest instead. The key
itself never touches this database — same "never written to disk"
story as `pgapp_admin`'s own Postgres password: it's read fresh from
`PGAPP_SECRET_KEY` (64 hex characters — `openssl rand -hex 32`) by
every command or request that actually needs a value, and isn't
required at all otherwise (`secret list`/`rm`, or an app with no
`{{secret...}}` references, work fine without it ever being set).

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
- Field-level authorization (page- and component-level `requires:`
  exist, per-column doesn't), plus password reset flows (today an admin
  deletes/recreates the account)
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
- Re-registering an already-registered slug into a *different*
  workspace re-points it (same "the registry decides" behavior as
  everywhere else) but doesn't migrate its existing data — the old
  workspace's physical tables are silently orphaned (not dropped, not
  moved), and the app starts over with fresh empty tables in the new
  workspace. Live-verified as part of scrapping classic mode: no
  automatic detection or warning yet.
- `pgapp_meta.apps.name` (the declared `app "Name" { }`) is unique
  **instance-wide**, not per-workspace — two unrelated apps that happen
  to declare the identical name collide even in different workspaces
  (the second sync silently repoints the first's metadata row rather
  than erroring). Give apps distinct names. This is unlike
  `pgapp_control.apps.slug` (the URL identifier, derived from the name
  via `slugify`), which *is* only unique per workspace — two apps named
  differently enough to avoid the `pgapp_meta.apps.name` collision but
  whose slugs happen to match (e.g. "Reports" and "REPORTS") coexist
  fine, routed independently at `/<workspace>/reports/...` each.
