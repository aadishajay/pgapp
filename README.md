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
- **Pluggable CSS/JS**: drop files in `assets/app.css` / `assets/app.js`
  and they're served and linked into every rendered page automatically.
- **Rust instead of PostgREST**: rather than fronting Postgres with
  PostgREST, pgapp's own Axum server owns routing directly, using the
  metadata to build parameterized SQL on the fly.

## Current status: vertical slice

This is deliberately the *smallest end-to-end loop*, not the whole
framework: one markup file → one app, hardcoded single-tenant, one field
type set (`id`, `text`, `boolean`, `integer`, `timestamp`), one page type
(CRUD list). It exists to prove the architecture end-to-end before
building the bigger pieces (drag-and-drop builder UI, actions/flows,
multi-app routing, auth) on top of it.

## Markup

```text
app "Todo" {
  entity "tasks" {
    field id: id
    field title: text required
    field done: boolean default false
    field created_at: timestamp default now
  }

  page "Tasks" as list of tasks {
    columns: title, done, created_at
    form: title, done
  }
}
```

- `entity` blocks describe a table: each `field` has a type, and
  optionally `required` and/or a `default`.
- `page ... as list of <entity>` describes a CRUD page: `columns` are
  shown in the list view, `form` are editable in create/edit forms.
  Fields left out of both (like `created_at` above) are still stored,
  just not user-editable — Postgres fills them via column defaults.

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

## Running it

Requires a reachable Postgres instance.

```bash
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/pgapp
createdb -U postgres pgapp   # if it doesn't exist yet

cargo run                     # serves examples/todo.app on 127.0.0.1:8080
# or: cargo run -- path/to/your.app
```

On startup it prints the URL for each page, e.g. `http://127.0.0.1:8080/Tasks`.

- `GET  /`                       — index of pages in the app
- `GET  /:page`                  — list view + "add new" form
- `POST /:page`                  — create a row
- `GET  /:page/:id/edit`          — edit form
- `POST /:page/:id/update`        — update a row
- `POST /:page/:id/delete`        — delete a row
- `GET  /api/:entity`             — JSON list for that entity

## Roadmap (not in this slice)

- Multi-app routing (`/:app/:page`) instead of one app per process
- More field types, relationships (foreign keys, lookups)
- A real drag-and-drop builder UI that edits the markup/metadata
- `action`/`flow` blocks in the markup for pluggable server-side logic
  (the current CSS/JS asset hooks are the first pluggable extension
  point; actions and flows are the next one)
- Auth/roles at the page and field level
- Migrating existing data tables when field definitions change (today,
  `ensure_data_table` only handles `CREATE TABLE IF NOT EXISTS`)
