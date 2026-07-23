[← Back to README](../README.md)

# Getting started

- [Install & run your first app](#install--run-your-first-app)
- [Bundled example apps](#bundled-example-apps)
- [Scaffolding a new app](#scaffolding-a-new-app)
- [Instance mode (the deployment model)](#instance-mode)
- [Secrets](./secrets.md)

## Install & run your first app

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
[Scaffolding a new app](#scaffolding-a-new-app); `cargo install --path . --bin pgapp` skips
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
password (see [Instance mode](#instance-mode) below for what each one guards).
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
registered (see [Multi-app routing](./architecture.md#multi-app-routing)), `/` instead serves a plain
index page listing every registered app as a link — there's no single
app left to redirect to.

For scripts/CI, skip the prompts entirely: `pgapp new <AppName>`
scaffolds a `.pgapp` file with no database interaction (see
[Scaffolding a new app](#scaffolding-a-new-app) below), then register it explicitly with `app
create`/`run` above.

## Bundled example apps

To try the richer bundled demo instead of a blank scaffold, point `run`
at `examples/helpdesk.pgapp` — but its `call_function` action needs a
PL/pgSQL function that has to exist *before* the first sync, so run
these in order (`$DATABASE_URL` isn't set by pgapp itself — export it
as the same connection string you gave `pgapp instance init`):

```bash
export DATABASE_URL=postgres://user:pass@host:5432/<dbname>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/helpdesk_functions.sql
pgapp run examples/helpdesk.pgapp --workspace <slug>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/helpdesk_seed.sql   # after, once the tables exist
```

Running `pgapp run` again with a *different* `.pgapp` path adds a
second app alongside the first, in the same or a different workspace —
see [Multi-app routing](./architecture.md#multi-app-routing).

`examples/venpay.pgapp` is a third demo: a vendor-payment tracker hand-
ported from a real Oracle APEX application export, showcasing the
migration approach in [Migrating from Oracle APEX](./migration-from-apex.md) (a joined,
read-only view entity for a report that needs columns beyond its own,
and the `button` component's redirect-with-forwarded-params behavior).
No functions/seed script needed — `pgapp run examples/venpay.pgapp
--workspace <slug>` and add data through its own forms.

`examples/showcase.pgapp` is a fourth demo, and the most complete one:
a single-file tutorial app whose 12 pages exercise every component kind
this framework has — reports (computed columns placed among physical
ones, currency/number/date format masks, a per-column aggregate, a
control break, a row highlight rule, and all three display modes),
forms (every built-in item type), an editable table, all six chart
types, a calendar, a map, faceted search, a `dynamic_content` home page
(the one place raw HTML is allowed), three dynamic actions, and both
server-side action styles (`run_query`/`call_function`). Its Home page
links to every other page, so it doubles as a live index. Same
functions-then-sync-then-seed order as helpdesk:

```bash
export DATABASE_URL=postgres://user:pass@host:5432/<dbname>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/showcase_functions.sql
pgapp run examples/showcase.pgapp --workspace <slug>
psql "$DATABASE_URL" -v schema=<workspace_schema> -f examples/showcase_seed.sql   # after, once the tables exist
```

`examples/helpdesk-modular/` is the helpdesk app split across multiple
files (see [One file, or a directory](./markup.md#one-file-or-a-directory)) — run it with `pgapp run
examples/helpdesk-modular --workspace <slug>`; it syncs to the same
metadata as the single-file version.

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

## Scaffolding a new app

`pgapp new`/`pgapp create` generates a minimal, runnable starter app —
one entity, one page with the classic Report+Form CRUD pattern, a nav
link to it. Both modes are pure file scaffolders: neither touches a
database, so the generated `.pgapp` file still needs registering into a
workspace (`pgapp app create`, or `pgapp run --workspace` —
see [Instance mode](#instance-mode) below) before it's actually served:

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
[Instance mode](#instance-mode)).

`cargo pgapp create` (the `cargo-pgapp` binary `cargo install --path .`
also installs — see above) is the exact same `pgapp create`, just
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
.` — see above).

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
queries](./markup.md#named-queries)), never the schema, so the same app runs
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
registry decides what's served" rule as [Multi-app routing](./architecture.md#multi-app-routing):

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
  registered in more than one workspace (see [Multi-app routing](./architecture.md#multi-app-routing)).

The App Builder's own reserved workspace/app (`pgapp`/`builder`) is
exempt from all of this — `workspace destroy pgapp` and `app destroy
builder --workspace pgapp` both refuse outright, `app create`/`run`
refuse to target `--workspace pgapp`, and `pick_workspace`'s own
interactive listing (when `--workspace` is omitted) never offers it as
a choice in the first place. It's created automatically by `instance
init`/every `run` (see [App Builder](./app-builder.md)) and the only way to remove
it is `pgapp instance destroy`, which takes everything else with it
too — same belt-and-suspenders philosophy as the App Builder's own web
self-edit guard, just enforced on the CLI side too.

`pgapp workspace list` and `pgapp app list` show what's currently
registered.

---

Next: [Core markup language](./markup.md) · [Architecture](./architecture.md) · [App Builder](./app-builder.md)
