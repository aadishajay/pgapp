[← Back to README](../README.md)

# App Builder

- [Editing an existing app's pages](#editing-an-existing-apps-pages)
- [Creating a brand-new app](#creating-a-brand-new-app)
- [Creating a brand-new workspace](#creating-a-brand-new-workspace)
- [Editing an app's data model, queries, navigation, and settings](#editing-an-apps-data-model-queries-navigation-and-settings)
- [Deleting an app or its whole workspace](#deleting-an-app-or-its-whole-workspace)
- [SQL Workshop](#sql-workshop)
- [How it's built](#how-its-built)

`examples/app_builder.pgapp` is a pgapp app, like any other, that lists
every app registered across every workspace in the instance — as a
genuinely searchable Interactive Report (search box, column filter,
sort, saved views), same as the Pages listing underneath it, not a
static card grid — and lets you edit a page in an APEX-Page-Designer-
style split view: a clickable component tree on the left, a docked
property editor on the right showing the selected component's full
attribute form (title/entity/columns/computed columns/format masks/
item types/dynamic-action ops/requires/attrs — whatever that kind
supports, as typed fields and add/remove/reorder row lists, not a raw
markup blob) inline, no modal — add a component of any of the 13
kinds, delete it, add/rename/delete whole pages, jump straight to a
live preview, scaffold brand-new apps, stand up a brand-new workspace
from scratch, and — via each app's own "AppSettings" page — edit its
data model (entities/fields, with name suggestions drawn from the
table's real Postgres columns), named queries (with a live "Test
Query" syntax/table/column check and a quick tables-and-columns
reference alongside the SQL box), navigation menu,
theme/icons/chart_lib/auth settings, and secrets (add/list/remove,
the same encrypted-at-rest store `pgapp secret set/list/rm` uses),
plus delete the app or its whole workspace outright — an
Oracle-APEX-App-Builder-flavored way to build without hand-editing
markup over SSH. What's still Advanced-editor-only
(`header`/`footer` chrome, `auth_scheme` role groups, a directory-based
app's structure) is one click away via the "Advanced" link into the
full-file raw editor every app already has (see [Hot reload](./architecture.md#hot-reload));
each component's raw markup text is also still reachable one click
deeper, via its own "Edit as raw markup" fallback next to the
structured editor.

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
also show a small breadcrumb naming which app/page is actually being
edited, since that's otherwise only visible in the URL's own query
string.

## Editing an existing app's pages

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
  structured form for it — every attribute that kind supports as a
  real field: scalar text/number/select inputs for things like
  title/entity/query/chart type, and add/remove/reorder row lists for
  anything the grammar itself repeats — a Report's columns/computed
  columns/format masks, a Form's fields/per-field item types, a dynamic
  action's ops, the shared `attrs (...)` extra-attribute list every
  kind carries. Fill it in, Save — the dialog *generates* fresh markup
  text for that one component client-side and POSTs it to
  `.../pages/:page/components/add`, same endpoint the original raw
  editor used. A "Add as raw markup" link next to the kind picker
  reveals the original raw-textarea-plus-starter-template flow, for
  anything the structured form doesn't cover well. The new component
  always lands at the bottom of the page; drag it into place from
  there.
- **Edit**: clicking a row in the component tree (not a separate
  pencil button — the whole row is the affordance) loads the same
  structured editor into the docked Property Editor alongside it,
  prefilled from that component's already-resolved, already-typed
  attributes rather than its raw text — a Form's
  `trip_type as popup from query trip_types_lov` shows up as a real
  "popup" dropdown with a "from query" config field already filled in,
  not a string to retype. A Report's own Columns list also gets a
  "Column headings & alignment" row-list here — an optional per-column
  `heading "<Display name>"` override plus a left/center/right `align`,
  the same APEX "column heading" concept, distinct from the underlying
  field name. Save regenerates the whole component's markup from the
  panel's current state and POSTs it to
  `.../pages/:page/components/:idx/edit`, replacing the whole block —
  genuine APEX-Page-Designer-style editing: pick a component in the
  tree, get a property sheet right there, not a raw text box, and not
  a modal either. An "Edit as raw markup" button in the panel's footer
  opens the original raw-markup textarea instead, for anything the
  structured form doesn't have a dedicated control for yet, or to
  hand-tweak formatting/inline comments the structured editor can't
  preserve (regenerating from typed fields necessarily drops any
  comment that lived *inside* the component's own block, since a
  comment isn't part of its attribute data — a comment immediately
  above the component's own declaration line is untouched either way,
  since that's `page_reorder`'s doing, not the structured editor's).
  Every dropdown (entity, query, page, action, item-type kind, chart
  type, auth scheme) is populated from a live app-meta endpoint rather
  than hand-typed, so a target that doesn't exist yet can't be typo'd
  in.
- **Delete component**: per-row ✕ button (with a confirm dialog) —
  POSTs to `.../pages/:page/components/:idx/delete`.
- **Run this page ↗**: opens the page's real, live URL in a new tab —
  built client-side from this page's own `?target_workspace=`/
  `?target_app=`/`?target_page=`, same params every cross-app action
  here reads.
- **Advanced: edit full app source ↗** (on the Pages screen): a link
  to the target app's own, already-existing `/admin/reload` page — a
  full-file raw markup editor built into *every* app (see [Hot
  reload](./architecture.md#hot-reload)). Entities, queries, nav, header/footer, and
  app-level settings (theme/auth/icons) have no dedicated control in
  this picker — this is how to reach them without SSHing in.

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

## Creating a brand-new app

The "New App" page scaffolds a fresh single-file app (a starter
`items` entity + page, the same shape `pgapp new` generates) into an
existing workspace — name, target workspace (picked from a list,
excluding the App Builder's own reserved workspace), and theme. Submit
and the same page reloads already processed and **already live**: a
Form writes a pending row, `runtime.js`'s `bindNewAppProcessing` POSTs
to `/pgapp/builder/admin/apps/create-pending` on every load of the
NewApp page (a harmless no-op when nothing's pending), which scaffolds
the file, syncs it into `pgapp_meta`, registers it in `pgapp_control`,
*and* hot-registers it into the running server's `AppState` — so it's
reachable immediately, no `pgapp run` restart needed. This isn't a
`before_load` action like every other "process something automatically"
case in pgapp: hot-registering needs `AppState` access, which action
modules don't have, so this is a dedicated route instead. Errors (bad
theme, unknown/disabled workspace, a slug collision) land in that same
row's `status`/`result` columns rather than a page-level warning
banner, so they stay visible on every later load too, not just the one
right after submission.

## Creating a brand-new workspace

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
renders a bare button, with nowhere to put typed fields, so
`NewWorkspaceForm` is a `dynamic_content` module that renders a real
`<form>` instead, posting to a sibling hidden `action ... calls
create_workspace` component (`attrs (style: "display: none")` — it
exists only to be that POST target). And unlike "New App"'s
pending-request-row pattern, `CreateWorkspace` never writes a row at
all: workspace creation needs no `AppState` hot-registration (a
workspace isn't itself "served," an app inside one is), so there's no
architectural reason to persist anything, which matters here
specifically because the "attach to an existing schema" connection
string is a superuser-capable secret typed into a web form — it lives
only in that one request's in-memory parameter map and is never
written to a database row, logged, or echoed into an error message.
`ensure_role`/`grant_admin_on_schema` (in `src/control.rs`) are the
same DDL `pgapp workspace create` itself runs, shared by both so the
CLI and this web form can't drift.

## Editing an app's data model, queries, navigation, and settings

Every app's "AppSettings" page (reached from a "Data Model, Queries,
Nav & Settings →" link on the Pages screen) is the App-Builder
counterpart to APEX's Data workshop, Shared Components, and Edit
Application Properties, all in one place:

- **Data Model**: add/edit/delete entities and their fields (name,
  type, a real checkbox for required, default) through the same
  structured field-list editor a Form/Report's own field picker
  already uses. For an *existing* physical entity, the Name field is a
  datalist — not a hard dropdown — suggesting real column names read
  straight from `information_schema.columns`, scoped to the app's own
  `data_schema`; typing something not in the list is still fine (a
  brand-new field the next sync will add a column for). Adding a
  physical entity provisions its table on the next sync exactly like a
  hand-written `entity { }` block would; adding a query-backed one
  (`from query <name>`) just needs an existing query to point at.
  Renaming an entity, or changing an existing field's type, isn't
  supported here — the former needs rewriting every place that entity
  is referenced (unlike a page, which already has that machinery), and
  the latter is already a hard sync-time error if it doesn't match the
  physical column, so there's nothing this editor could safely do
  differently. Deleting an entity removes its `pgapp_meta` bookkeeping
  only — its physical table (if it has one) is deliberately left in
  place, same "pgapp adds columns but never changes or drops them"
  caution [deployment checks](./reports.md#deployment-checks) already apply to fields.
- **Queries**: add/edit/delete a named query (name + SQL), with the
  same schema-metadata endpoint powering a quick "Available tables"
  reference (table name, its entity name, and its real columns) right
  under the SQL box, and a **Test Query** button that POSTs the SQL to
  a test endpoint — real syntax/table/column validation via the same
  Postgres `Describe` round-trip a sync already runs on every named
  query, now reachable before saving, not just after, reporting either
  the bind names it detected or Postgres's own error message. Deleting
  a query still in use (an entity `from query`, a report/chart/region/
  LOV bound to it) is rejected at the next sync with the same "unknown
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
- **Secrets**: add/update (name + value) and remove app-scoped
  secrets — the same encrypted-at-rest store (`src/secrets.rs`,
  AES-256-GCM) the CLI's `pgapp secret set/list/rm` already uses (see
  [Secrets](./secrets.md)), and what an `http_request` action's
  `{{secret.name}}` resolves against. Only names ever come back to the
  browser; a value, once saved, can be overwritten but never displayed
  again. Setting one still needs `PGAPP_SECRET_KEY` set in the server
  process's own environment — without it, Add/Update fails with a
  clear error instead of silently no-op'ing.

The first four (Data Model/Queries/Navigation/App Settings) are all
line-splice edits in `src/app_editor.rs`, the entity/query/nav/settings
counterpart to `src/page_reorder.rs`'s page/component splices — same
discipline (reusing the real parser's own walk to find exact line
ranges, so untouched content, including comments and formatting,
survives every edit unchanged), same routes shape
(`/:workspace/:app/admin/{entities,queries,nav,settings}...`, JSON
in/out, validated with `markup::parse_app` before writing, ending in a
hot `entry.reload()` — no restart). Secrets don't touch the markup file
at all — they're rows in `pgapp_control.secrets`, scoped by the app's
own control-plane id, so there's no reload step and no file to keep in
sync.

## Deleting an app or its whole workspace

Two "danger zone" panels, each with the same soft/hard choice `pgapp
app destroy`/`pgapp workspace destroy` already have on the CLI:

- **Delete This App**: soft disables it (reversible — its tables and
  rows are untouched, and re-registering it later reactivates it,
  though on an already-running server this takes effect on the next
  `pgapp run`, not immediately, same as the CLI); hard permanently
  drops its own data tables, needs the app's own slug typed in to
  confirm (mirroring the CLI's confirmation), and unregisters it from
  the live server immediately so it starts 404ing right away rather
  than erroring against now-missing tables. No superuser connection
  needed either way — `pgapp_admin` already owns every table it
  created, in any workspace schema it's been granted into.
- **Delete Workspace**: same soft/hard choice, but for the *whole*
  workspace — every app registered in it, torn down together. Hard
  delete needs the workspace's own slug typed in, **plus** a
  superuser-capable Postgres connection string (dropping a schema/role
  needs privilege beyond what a schema-level grant gives `pgapp_admin`,
  and an "attach to an existing schema" workspace was never owned by it
  to begin with) — used once, in memory, to run the `DROP SCHEMA`/`DROP
  ROLE`, and never persisted, the exact same contract as "New
  Workspace"'s existing-schema attach.

Both routes reuse the same self-edit guard and "borrow the target
app's own auth" access check as everywhere else in the App Builder —
the workspace-destroy route's URL still names an `:app` even though the
*operation* isn't scoped to one, because the global auth middleware
resolves every request's auth context from the `/{workspace}/{app}/...`
shape before any route handler runs, so a workspace-wide action still
needs *some* registered app in the URL to borrow that context from.

## SQL Workshop

A "SQL Workshop →" link on WorkspaceDetail (next to the Delete-Workspace
panel) opens an Oracle-APEX-SQL-Workshop equivalent: ad hoc **SQL
Commands** plus a read-only **Object Browser**, both scoped to that
workspace's own `data_schema` — not a new trust boundary, since
`pgapp_admin` already holds ALL PRIVILEGES on every table there (the
same access the Data Model/Queries editors above already exercise).
Like Delete Workspace, the URL's `:app` segment is only ever borrowed to
resolve auth context; every app registered into one workspace shares
that workspace's own schema, so it never changes which schema either
panel runs against.

- **SQL Commands** (`POST /:workspace/:app/admin/sql/execute`,
  `admin_sql_execute` in `src/server.rs`): runs one statement at a time
  against the workspace's schema. A simple keyword sniff on the
  statement's leading word (`select`/`with`/`table`/`values`/`show`/
  `explain`) decides whether to treat it as a result-returning query
  (real column order via Postgres's own `Describe`, then rendered as a
  grid) or a plain command (`rows_affected` only) — imperfect (a leading
  comment defeats it) but degrades to a clear error rather than a wrong
  answer. No `:bind` variables — paste literal SQL, the same way psql
  would take it. The editor is vendored CodeMirror 5 (SQL mode, bracket
  matching, Ctrl-Space autocomplete) plus `sql-formatter` for the
  **Format** button — not Monaco, whose real footprint (~30MB, an AMD
  loader, many chunk files) doesn't fit "lightweight" or this repo's
  single-flat-`assets/`-directory, no-bundler asset model; see
  `assets/VENDORED_LICENSES.md`. Ctrl/Cmd+Enter runs the current buffer.
- **Object Browser** (`GET /:workspace/:app/admin/sql/table-detail`,
  `GET /:workspace/:app/admin/sql/table-data`): a filterable list of
  every table in the schema (the same `/admin/tables-list` the entity
  editor's table picker already uses); picking one shows tabbed
  Columns/Data/Indexes/Constraints/DDL panes. Data is paginated
  (25 rows/page) through the same `run_named_query_page` machinery a
  named query's own report pagination uses, via a synthetic
  `select * from "table"` built once the table name has been checked
  against `information_schema.tables` (the one place here that splices
  an identifier into raw SQL rather than binding it, so it's validated
  first). The DDL pane is hand-assembled from real column/index/
  constraint metadata for a quick copy-paste reference — it is
  *not* pg_dump-exact (no partitioning, no tablespace, no storage
  parameters, no comments).

## How it's built

The drag itself, the tree-plus-property-panel split, and the per-row/
per-card action buttons are all `runtime.js` — plain HTML5 drag-and-drop,
a re-parenting trick that combines the "Components" region and
"Properties" placeholder (two separate markup components) into one
flex row purely in the DOM, and the same per-kind structured renderers
(one function per kind, plus shared widgets for repeatable rows,
attrs, requires, config, and item types) rendered directly into that
docked panel instead of a modal — "Add Component" is the one place a
modal still appears, since a brand-new component has no existing row
to dock a panel next to yet. Saving in the structured editor never
sends structured data to the server at all — it *generates* markup
text client-side (mirroring the grammar `markup.rs` parses) and submits
that through the exact same raw-text `/components/.../add`|`edit`
routes the original editor already used, so no new write-side route
was needed; only reading a component's current, already-typed
attributes needed one, plus one more for its dropdowns' contents.
Since these all describe some *other* app's page, every one of them
builds that app's own URL from `?target_workspace=`/`?target_app=`/
`?target_page=` on the **current** (App Builder) page's own URL, not
from anything baked into the markup — forwarded page-to-page the same
way any other cross-page parameter is.

**A structured editor's `generate()` must cover every attribute the kind
supports, or Save silently drops whatever it doesn't** — since it
*regenerates the whole component's markup from the form's current
state*, any attribute the form has no field for (rather than a field
the user just left blank) never makes it into the new markup, even
though the old value was there a moment ago. The Report editor learned
this the hard way early on: its `aggregate`/`break_on`/`highlight`/`display`
properties were readable but had no corresponding form fields, so
opening a Report that used any of them and clicking Save quietly
deleted them from the markup. All four now have dedicated fields, so
round-tripping an existing Report through the editor with no changes
reproduces every property unchanged (a `format`/`heading`/`align`
targeting the default still gets normalized away, but nothing with a
non-default value disappears).

---

Next: [Migrating from Oracle APEX](./migration-from-apex.md) · [Roadmap](./roadmap.md)
