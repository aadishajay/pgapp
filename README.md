<div align="center">

# pgapp

### Postgres is the backend. This is everything else.

Describe a CRUD app in one plain-text file. Get back a live, multi-user
web app — reports, forms, charts, auth, and a point-and-click admin
builder — served by a single Rust binary that talks straight to
Postgres. No ORM. No separate API service. No JS build step.

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Postgres](https://img.shields.io/badge/database-PostgreSQL-4169E1?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![GitHub stars](https://img.shields.io/github/stars/aadishajay/pgapp?style=social)](https://github.com/aadishajay/pgapp/stargazers)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](./docs/roadmap.md)

[Quick start](#quick-start) ·
[Why pgapp](#why-pgapp) ·
[Example](#a-full-app-in-30-lines) ·
[App Builder](#the-app-builder) ·
[Docs](./docs/README.md) ·
[Compared to Supabase/PocketBase/Appsmith](#how-this-compares)

</div>

---

> **Project status:** pre-1.0, actively developed, no stability
> guarantees yet. No `LICENSE` file is in the repo as of this writing
> — see [Roadmap](./docs/roadmap.md) before depending on this in
> production.

## Why pgapp?

If you already keep your data in Postgres, most "app builder" tools
make you stand up a second system to describe your app: a BaaS with
its own auth service and API gateway, a low-code SaaS with its own
hosted runtime, or a Node backend you write and deploy yourself.

pgapp skips that. The **app definition is Postgres-native**: entities,
pages, and components live as rows in `pgapp_meta`, synced from a
plain-text `.pgapp` file you can diff, review, and commit like any
other source file. The **server is one Rust binary** — it reads that
metadata, builds parameterized SQL, and renders HTML. No PostgREST, no
Supabase Studio, no separate builder service, no generated client SDK.
Editing the file (or clicking through the built-in **App Builder** —
see below) and hitting reload is the whole deploy loop.

If you've used Oracle APEX, the shape will feel familiar — pages made
of Reports/Forms/regions, PL/SQL-style server actions, declarative
dynamic actions, Interactive Reports with saved views. pgapp borrows
that model deliberately (it's a genuinely good one for CRUD-heavy
internal tools) and reimplements it as an open, self-hosted, plain-text
alternative that runs on infrastructure you already have: any Postgres
database and a Rust binary.

## Features

- **One file describes the whole app.** Pages, entities, queries, nav,
  auth — a `.pgapp` file (or a directory of them) is the single source
  of truth, synced into Postgres at startup and again on
  `/admin/reload`. No restart to pick up a change.
- **13 component kinds, composed freely.** Report, Form, EditableTable,
  Chart, Calendar, Map, FacetedSearch, Region, DynamicContent, Action,
  Button, Link, Text — any combination, any number, one page. See
  [Components](./docs/components.md).
- **Interactive Reports, not toy tables.** Search, per-column filters,
  private/public saved views, sortable columns, footer aggregates,
  control breaks, row highlighting, CSV export — all declarative, all
  in metadata. See [Reports](./docs/reports.md).
- **19 built-in form field types** (popup LOV, star rating, shuttle,
  rich text with sanitization, file upload, and more), plus a
  one-file-per-type plugin point for your own. See [Forms](./docs/forms.md).
- **6 chart types**, rendered as dependency-free inline SVG by default
  (no client JS, no CDN) — or swap in any JS charting library per app.
  See [Charts](./docs/charts.md).
- **Server-side actions**, the PL/SQL-process analog: named Rust
  modules or a plain PL/pgSQL function call, wired to a button, a
  dynamic action, or a report's `before_load`. Ships HTTP calls, SMTP
  email, and raw SQL execution out of the box. See [Actions](./docs/actions.md).
- **Declarative dynamic actions** — `on change of x { show/hide/set/refresh }`
  — plus an ajax callback op that reaches the server with no page reload.
- **Auth in one block.** `auth { }` turns on argon2-hashed logins,
  server-side sessions, and role-gated pages/components — down to the
  individual button. See [Authentication](./docs/authentication.md).
- **Pluggable everything** — themes, icons, chart backends, form
  widgets — each a small file dropped into its own directory, selected
  by name in markup. Six themes ship, including one styled after
  Oracle APEX's Universal Theme for side-by-side migrations. See
  [Theming](./docs/themes.md).
- **A built-in point-and-click admin app** (the **App Builder**) for
  everything above — no SSH, no hand-editing markup required, though
  you always still can. See [The App Builder](#the-app-builder).
- **Multi-tenant from one process.** Instance → workspace (schema) →
  app is the whole deployment model; one Postgres connection pool
  serves any number of apps across any number of workspaces. See
  [Architecture](./docs/architecture.md).

## Why Postgres?

Because Postgres is already the backend most teams reach for, and it's
capable of more than a JSON row store: real types, `information_schema`
introspection, the wire-protocol `Describe` message for finding out a
query's parameter types before it ever runs, row-level correctness
under concurrent writes. pgapp leans on all of that directly instead of
routing around it — it asks Postgres what a bind parameter's type is
(`:project_id`, not `:project_id::integer`) rather than making you
declare it twice, and it verifies every entity's physical table against
its declared fields at every startup and reload, failing loudly on a
mismatch instead of a confusing runtime cast error. See [Named
queries](./docs/markup.md#named-queries) and [Deployment checks](./docs/reports.md#deployment-checks).

## A full app in 30 lines

```text
app "Todo" {
  nav {
    item "Tasks" -> page Tasks
  }

  entity "tasks" {
    field id: id
    field title: text required
    field priority: text default Medium
    field done: boolean default false
  }

  page "Tasks" {
    report "Tasks" of tasks {
      columns: title, priority, done
      page_size: 10
    }

    form "Add / edit task" of tasks {
      fields: title, priority, done
      item priority as radio ("Low", "Medium", "High")
    }
  }
}
```

That's a searchable, paginated, sortable list with inline create/edit/
delete — the classic Report+Form CRUD pattern — wired up automatically
because the Form and Report share an entity on the same page. No route
handlers, no SQL, no client-side JS written by you. See the
[full markup reference](./docs/markup.md) for named queries, computed
columns, nav menus, and every component kind.

## Screenshots

<table>
<tr>
<td width="50%">

**Interactive Report** — faceted search, sortable columns, and a
visible cue (that `→`) telling you a row is clickable before you ever
touch the mouse.

<img src="marketing/shots/2026-tasks-report.png" alt="pgapp Interactive Report: a faceted, sortable task list with a clickable row cue" width="100%">

</td>
<td width="50%">

**Charts from one line of SQL each** — bar, donut, pie, and more,
every category getting its own consistent color automatically.

<img src="marketing/shots/2026-dashboard-charts.png" alt="pgapp Dashboard: multiple chart types with per-category colors, generated from named queries" width="100%">

</td>
</tr>
<tr>
<td width="50%">

**The App Builder's entity editor** — pick a real table, and its
columns, nullability, and primary key populate the field list for you.

<img src="marketing/shots/2026-appbuilder-entity-editor.png" alt="pgapp App Builder: entity editor with a searchable table picker dialog open" width="100%">

</td>
<td width="50%">

**The App Builder's page designer** — a component tree on the left,
a live property panel on the right, changes saved straight to the
`.pgapp` file.

<img src="marketing/shots/2026-appbuilder-editpage.png" alt="pgapp App Builder: page designer with component tree and property panel" width="100%">

</td>
</tr>
</table>

More in [`marketing/`](./marketing/), including a full
[feature-by-feature tutorial](./marketing/index.html).

## Quick start

You need Postgres reachable somewhere and Rust installed. Install the
binary once:

```bash
cargo install --path .
```

Then set up the one instance pgapp needs (a Postgres database →
workspace schema → registered app — see [Getting
started](./docs/getting-started.md) for what each of these means):

```bash
pgapp instance init                          # once, ever — one instance per machine
pgapp workspace create --schema <name>       # once per schema
pgapp app create --workspace <slug>          # scaffolds + registers a starter app
pgapp run <generated-file>.pgapp --workspace <slug>
```

That prints the app's URL and starts serving it. To try a richer demo
instead of a blank scaffold:

```bash
pgapp run examples/showcase.pgapp --workspace <slug>
```

`examples/showcase.pgapp` is a single-file tour of every component kind
pgapp has — reports, forms, an editable table, all six chart types, a
calendar, a map, faceted search, dynamic content, dynamic actions, and
both server-side action styles — with its Home page linking to every
other page. `examples/helpdesk.pgapp` (auth-gated, seeded ticket data)
and `examples/venpay.pgapp` (a real Oracle APEX app hand-ported over)
are two more. Full walkthrough, including the seed-data steps: [Getting
started](./docs/getting-started.md).

## Architecture

```
 .pgapp markup file
        │  parse
        ▼
    AppDef (in memory)
        │  validate + sync
        ▼
 pgapp_meta.* tables  ──creates──▶  <workspace_schema>.<table>  (your real data)
        │  load
        ▼
    RuntimeApp { pages → components }
        │
        ▼
   Axum router  ──  generic, metadata-driven CRUD + JSON
```

One Rust binary, one shared Postgres connection pool, any number of
apps. Every SQL identifier used at request time is validated at sync
time against the lexer's own identifier charset — safe to splice into
generated SQL; every user *value* is always a bind parameter. Full
diagram, source layout, and the complete route table: [Architecture](./docs/architecture.md).

## Core concepts

- **In-database metadata.** Apps, entities, fields, pages, and
  components are rows in Postgres — the server always serves off the
  database, never off whatever was last parsed on disk.
- **A textual markup language**, APEX-flavored, synced at startup and
  on-demand via `/admin/reload` — no restart to pick up a change.
- **Composable pages.** A page is just an ordered list of independent
  components — any combination, any number, one page.
- **Pluggable everything.** Themes, item types, charts, icons — each a
  small file dropped into its own directory, selected by name.
- **Named queries**, bind-typed automatically from the schema, shared
  across LOVs, regions, charts, report sources, and whole read-only
  entities.
- **A DB-stored JS runtime** — `/:workspace/:app/runtime.js` is a
  metadata row, not a static file, editable without touching the
  binary.

This is deliberately the *smallest end-to-end loop*, not the whole
framework — see [Roadmap](./docs/roadmap.md) for what's genuinely not built yet.

## Example applications

| App | What it shows |
|---|---|
| [`examples/todo.pgapp`](./examples/todo.pgapp) | The minimal shape — one entity, Report+Form CRUD, a chart |
| [`examples/helpdesk.pgapp`](./examples/helpdesk.pgapp) | Two entities, dashboards, both pagination modes, auth, every item type, the `vivid` theme |
| [`examples/venpay.pgapp`](./examples/venpay.pgapp) | A real Oracle APEX app hand-ported over — see [Migrating from Oracle APEX](./docs/migration-from-apex.md) |
| [`examples/showcase.pgapp`](./examples/showcase.pgapp) | Every component kind, all six chart types, all three server-side action styles, in 12 pages |
| [`examples/helpdesk-modular/`](./examples/helpdesk-modular/) | The helpdesk app split across files instead of one — same grammar, zero refactoring |
| [`examples/nexus-erp/`](./examples/nexus-erp/) | 200 pages, 60 entities, 15 files — the load-test fixture: 30 threads sustained ~900 req/s (p50 ~27ms, p99 ~94ms) with zero errors |

## The App Builder

Every pgapp instance ships with a built-in admin app — reachable at
`/pgapp/builder`, provisioned automatically, no setup step — that lets
you build and edit apps by clicking instead of hand-editing markup over
SSH:

- An **APEX-Page-Designer-style split view**: a clickable component
  tree on the left, a docked property editor on the right with a real
  typed form for whatever the selected component supports — not a raw
  markup blob.
- **Add, edit, delete, and reorder** any page or component; **rename a
  page** and every reference to it (nav items, report links) updates
  with it.
- **Edit the data model** — entities and fields, with name suggestions
  drawn straight from the table's real Postgres columns — plus named
  queries (with live syntax/table/column validation before you save),
  the nav menu, and per-app theme/auth settings.
- **Manage secrets** (encrypted at rest) that an action's
  `{{secret.name}}` resolves against.
- **Scaffold a brand-new app or workspace** from the browser, live —
  no restart.
- **Tear down an app or a whole workspace**, soft (reversible) or hard,
  with the same confirmation discipline as the CLI.

It's not a bespoke admin panel bolted onto the framework — **the App
Builder is itself just a pgapp app**, built from the same components
and served the same way, with one deliberate exception: a hardcoded
guard that refuses to let it edit itself, checked twice (once in the
listing, once again server-side on every mutating route).

Full walkthrough: [docs/app-builder.md](./docs/app-builder.md).

## Unique to pgapp

pgapp reimplements a lot of genuinely good ideas from Oracle APEX —
Interactive Reports, dynamic actions, a Page-Designer-style editor —
and that borrowing is not the interesting part. These are the pieces
that aren't an APEX port:

- **The app *is* a portable text file, and the GUI editor never
  breaks that.** Every App Builder mutation is a line-based **text
  splice** into your actual `.pgapp` file (never a parse-and-regenerate
  of the whole thing), so hand-written formatting and comments on
  everything you didn't touch survive byte-for-byte. Click through the
  UI or hand-edit the file — they're the same source of truth, and
  either one stays diff-friendly in git.
- **Bind types come from Postgres itself, not from you.** A named
  query's `:param` markers are typed by asking Postgres's own wire
  protocol `Describe` message what type each one needs to be — fresh
  on every sync — instead of a hand-declared cast or an ORM's model
  layer. Schema drift is caught at reload time, not as a runtime cast
  error.
- **The JS runtime is a database row.** `/:workspace/:app/runtime.js`
  is metadata, not a static asset — editable in place, per app, with no
  rebuild.
- **One process, N tenants, hot-registered.** `pgapp run` adds an app
  to a live server's routing table with no restart and no downtime for
  anything already running; the instance → workspace → app model gives
  each workspace its own schema and Postgres role from one shared
  connection pool.
- **Collections are isolated by construction, not by convention.** A
  per-browser scratch row store (for API responses, session-style
  state) is scoped by a SQL `WHERE` clause pgapp itself generates —
  there is no app-authored SQL path that could leak another caller's
  rows, because there's no author-written SQL involved in reading one
  back at all.
- **The admin builder can't edit itself**, on purpose, checked in two
  independent places — a small, deliberate safety property most
  self-hosted admin tools don't bother with.

## How this compares

pgapp occupies a narrower, more opinionated niche than most of the
tools people reach for first — worth knowing before you pick it:

| | **pgapp** | Supabase | PocketBase | Appsmith / Budibase / ToolJet |
|---|---|---|---|---|
| Core language | Rust | Mostly TS + Postgres extensions | Go | Java / Node (varies by project) |
| App definition | Plain-text file, versioned in git | Dashboard + SQL + client SDKs | Dashboard + client SDKs | Drag-and-drop, stored as JSON |
| Database | Postgres — bring your own | Postgres (managed or self-hosted) | SQLite, embedded | Connects to a DB you already run |
| Runtime shape | One static binary | Multi-container stack (Studio, GoTrue, PostgREST, Realtime, Kong, ...) | One static binary | An app server (Node/Java) you deploy |
| Admin/builder UI | Built in, itself a pgapp app | Separate Studio service | Built-in admin UI | Yes — it's the whole product |
| Best fit | Postgres-first teams wanting typed CRUD apps fast, minimal moving parts | Full BaaS: auth, storage, realtime, edge functions | Small embedded backends, single binary deploys | Internal-tool builders, DB-agnostic, team-oriented |

None of these are strictly "better" — they're solving adjacent but
different problems. If you want a full backend-as-a-service with
auth/storage/realtime/edge-functions, Supabase is the more complete
answer. If you want the smallest possible embedded backend, PocketBase.
If your team wants a drag-and-drop builder that's database-agnostic,
look at Appsmith/Budibase/ToolJet. pgapp is for the case where Postgres
is already the answer to "where does the data live," and you want the
thinnest possible layer on top of it that's still a real web app.

## Documentation

The docs live in [`docs/`](./docs/README.md) — this README is the
pitch, everything below is the reference:

| Doc | What's in it |
|---|---|
| [Getting started](./docs/getting-started.md) | Install, run your first app, bundled demos, scaffolding, instance mode |
| [Architecture](./docs/architecture.md) | The markup → metadata → runtime pipeline, source layout, full route table, multi-app routing |
| [Markup language](./docs/markup.md) | The complete `.pgapp` grammar — entities, pages, queries, directories, collections |
| [Components](./docs/components.md) | Every component kind in depth |
| [Reports](./docs/reports.md) | Pagination, search & saved views, computed columns, format masks, pre-load actions |
| [Forms & item types](./docs/forms.md) | The `ItemType` trait and every built-in field widget |
| [Charts & icons](./docs/charts.md) | Chart types, pluggable rendering backends, icon packs |
| [Authentication](./docs/authentication.md) | `auth {}`, roles, component-level `requires:`, auth schemes |
| [Actions](./docs/actions.md) | Server-side action modules, dynamic actions, ajax callbacks |
| [Theming](./docs/themes.md) | The CSS contract, shipped themes, mobile responsiveness |
| [App Builder](./docs/app-builder.md) | The full point-and-click admin app walkthrough |
| [Migrating from Oracle APEX](./docs/migration-from-apex.md) | Concept-for-concept mapping table |
| [Secrets](./docs/secrets.md) | Encrypted-at-rest credentials for actions |
| [Roadmap](./docs/roadmap.md) | Known gaps, honestly listed |

## Roadmap

pgapp is intentionally the smallest end-to-end loop, not the finished
framework. No real foreign-key relationships yet, no multi-step flow
blocks, no drag-and-drop free page composition, no `Secure`-cookie
story below TLS — the full, honest list (with the reasoning behind
each gap) is in [docs/roadmap.md](./docs/roadmap.md). If one of these
is a dealbreaker for your use case, that's useful to know up front, and
also exactly the kind of thing a PR is welcome for.

## Contributing

Issues and PRs are welcome — this is early, opinionated software, and
outside perspective on where it breaks is genuinely useful. Good first
places to look:

- [docs/roadmap.md](./docs/roadmap.md) for known gaps
- `src/item_types/` to add a new form field type (`date.rs` is the
  smallest real example)
- `src/actions/` to add a new server-side action module
- `themes/` to add a new design system (pure CSS, no Rust)

There's no `CONTRIBUTING.md` or issue templates yet — open an issue or
a PR and it'll get a response.

## License

**No license file has been added to this repository yet.** Until one
is, treat the code as all-rights-reserved for anything beyond reading
it. Adding a permissive license (MIT is the common default for the
Rust ecosystem) is tracked as an open item — see
[docs/roadmap.md](./docs/roadmap.md).
