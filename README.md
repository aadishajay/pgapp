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
  nav {
    item "Tasks" -> page Tasks
    item "More" {
      item "About" -> page About
    }
  }

  entity "tasks" {
    field id: id
    field title: text required
    field done: boolean default false
    field created_at: timestamp default now
  }

  page "Tasks" as list of tasks {
    columns: title, done, created_at
    form: title, done
    link: title -> page TaskDetail
    items {
      text "Manage your tasks below. Click a title to see its detail page."
    }
  }

  page "TaskDetail" as detail of tasks {
  }

  page "About" as static {
    items {
      text "pgapp is an Oracle APEX-inspired no/low-code framework built on Postgres."
      link "Back to tasks" -> page Tasks
    }
  }
}
```

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
  column into a link to another page, passing the row's id as `?id=` —
  the classic "click a row to see its detail" pattern.
- `items { }` (any page kind) adds `text "..."` (static copy) and/or
  `link "Label" -> page <Name>` (a link to another page) to the page
  body — content that isn't tied to the entity's table/form.
- Anything that targets a page by name (`nav` items, `link:`, `link`
  items) uses a bare identifier, not a quoted string — restricting link
  targets to the same safe charset as SQL identifiers.

See `src/markup.rs` for the full grammar and `examples/todo.app` for a
working example.

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
`pgapp-btn-primary` / `pgapp-btn-destructive`), `pgapp-inline-form`,
`pgapp-alert` (+ `pgapp-alert-error`), `pgapp-list`, `pgapp-navbar` (+
`pgapp-navbar-item` / `pgapp-navbar-label` / `pgapp-navbar-submenu`),
`pgapp-items`, `pgapp-text` — and nothing else. A **theme** is what
gives those classes an actual appearance. This is the whole contract;
anything that satisfies it is a valid theme, regardless of what design
system it's built on.

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

## Roadmap (not in this slice)

- Multi-app routing (`/:app/:page`) instead of one app per process
- More field types, relationships (foreign keys, lookups)
- A real drag-and-drop builder UI that edits the markup/metadata
- `action`/`flow` blocks in the markup for pluggable server-side logic
  (theming, the CSS/JS asset hooks, and now nav/page items/linking are
  the pluggable extension points so far; actions and flows are next)
- Auth/roles at the page and field level
- Migrating existing data tables when field definitions change (today,
  `ensure_data_table` only handles `CREATE TABLE IF NOT EXISTS`)
