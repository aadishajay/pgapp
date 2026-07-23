[← Back to README](../README.md)

# The `.pgapp` markup language

- [Full example](#full-example)
- [Grammar notes](#grammar-notes)
- [One file, or a directory](#one-file-or-a-directory)
- [Query-backed entities](#query-backed-entities)
- [Binding an entity to an existing table](#binding-an-entity-to-an-existing-table)
- [Collections](#collections)
- [Named queries](#named-queries)

## Full example

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

## Grammar notes

- `header { }` / `footer { }` (optional) — app-wide chrome, restricted
  to `text`/`link`/`region`.
- `nav { }` (optional) — the nav bar; each `item "Label"` is a leaf
  (`-> page <Name>`) or a group (`{ nested items }`). A leaf whose
  target page has `requires: <role>` only shows for a user holding
  that role; a group left with no visible children disappears too.
- `entity` blocks describe a table: each `field` has a type, optionally
  `required` and/or a `default`.
- `page "Name" { ... }` — a name plus an ordered list of components
  (see [Components](./components.md)), and optionally `requires: <role>`.
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

## One file, or a directory

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
[Scaffolding](./getting-started.md#scaffolding-a-new-app)) follows one specific convention on top of that
mechanism, though: `pages/<name>.pgapp` (one file per page) and
`shared_components/<kind>/<name>.pgapp` (entities today; queries and
auth_schemes too, once an app has any) — the same split-by-kind shape
a real Oracle APEX application export uses
(`pages/page_NNNNN.sql`, `shared_components/<kind>/...`). It's a
starting-point convention the scaffold follows, not something
`src/source.rs` enforces — nothing stops an author from reorganizing
afterward.

`examples/helpdesk-modular/` is the helpdesk app split this way, and
`examples/nexus-erp/` pushes the same mechanism to a 200-page,
60-entity scale — see [Getting started](./getting-started.md#bundled-example-apps).

## Query-backed entities

`entity "name" from query <name> { field ... }` — a **read-only**
entity backed by a named query instead of a physical table (the APEX
"view" pattern). No table created; binding a `form`/`editable_table` to
it is a sync-time error; reports over it paginate by `OFFSET`.

## Binding an entity to an existing table

`entity "name" from table "existing_table_name" { field ... }` — unlike
`from query`/`from collection`, this is a normal, **writable** entity
(`Form`/`EditableTable` both work) whose physical table already exists
under a name other than the entity's own slug. Use it to point pgapp at
a table something else created — a legacy table, one owned by another
tool, or one two apps deliberately share.

Sync-time behavior is different from an entity pgapp owns: the table
must already exist (a clear error if it doesn't — pgapp never creates
or alters a `from table` entity's table the way `ensure_data_table`
does for its own), and there's no cross-app collision guard, since
pointing two entities at the same existing table is the point, not a
mistake. Declared fields are still checked against the table's real
columns (`meta::sync_app`'s usual `verify_data_table` check), and the
connecting role needs its own Postgres-level privileges on that table
— pgapp doesn't grant access to a table it didn't create.

The App Builder's entity editor offers this as "existing table" under
Source, with a picker listing every table actually in the app's schema
— including ones no entity has claimed yet.

## Collections

`entity "name" from collection "name" { field ... }` — an APEX-
Collection-style **read-only** entity backed by a scratch row store
(`pgapp_meta.collections`, a `jsonb` blob per row) instead of a
physical table or a query. Nothing to create at sync time; like
query-backed entities, binding a `form`/`editable_table` to one is a
sync-time error, and reports over it paginate by `OFFSET`.

Collections exist to hold data that didn't come from a table in the
first place — most often an external API response. The `http_request`
action (see [Server-side actions](./actions.md)) writes to one directly: give it a
`collection: "name"` config and, on a successful (2xx) JSON response,
it stores the body instead of just echoing it back — a JSON array
becomes one row per element, a bare object becomes a single row.
`collection_mode: "replace"` (default) clears any existing rows under
that name first in the same transaction; `"append"` keeps them and
continues the row numbering:

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
(see [Reports: pre-load actions](./reports.md#pre-load-actions)).

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
on that app), so schema drift (`alter column ... type bigint`) is
picked up automatically or rejected loudly at sync time, never silently
wrong at runtime. An explicit `:name::type` cast still works (a
redundant no-op under the automatic one).

`radio`/`popup` queries alias columns `value` and optionally `label`
(defaults to `value`). A `report`'s `source:` query needs an `id`
column plus whatever `columns` reference. A `region` has no
requirements. A `chart` needs whatever `x`/`y` name. Query SQL
references the entity's real physical table — just the entity's own
slug (printed at startup), no app-name prefix — and is decoded
generically via `to_jsonb`, so there's no compile-time column-type
checking beyond what Postgres itself enforces.

**Write the table name bare (the entity's own slug), never
schema-qualified.** Every connection a named query runs on — at sync
time (type inference) and at request time alike — has its
`search_path` pinned to this app's own `data_schema` first (its
workspace's own schema; see [Instance mode](./getting-started.md#instance-mode)). A
schema-qualified reference still works (qualified names ignore
`search_path` entirely) but stops working the moment the app is
re-registered into a different workspace — its tables move, but a
hardcoded schema prefix doesn't.

Since the table name is just the entity's slug, two different apps
that share a `data_schema` (a workspace can hold more than one app) can
end up naming the same table. That's not automatically an error —
`sync_app` checks the table's actual *structure*, not which app got
there first: a same-named entity whose declared fields don't conflict
in type with any existing column is just used as-is (including a field
one app declares that the other doesn't — that column gets added,
same as any normal schema evolution, since two entities intentionally
or coincidentally sharing a compatible table is exactly what a [`from
table`](#binding-an-entity-to-an-existing-table) binding exists to
support one step further). Only an actual conflict — the same column
name declared with two different types — is a sync-time error, naming
both the conflicting columns and, when it can identify one, the other
app already using that table. Apps in different workspaces never
interact this way at all, since each workspace is its own schema.

---

Next: [Components reference](./components.md) · [Reports](./reports.md) · [Actions](./actions.md)
