[← Back to README](../README.md)

# Components reference

A page's body is `Vec<ComponentDef>` (`src/model.rs`) — there's no
fixed "page kind." pgapp ships fourteen component kinds; a page can mix
any number of any of them, in any order.

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
  Classic-Report alignment (default left). See [Reports](./reports.md) for
  pagination, search, saved views, computed columns, and format masks
  in depth.
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
  See [Item types](./forms.md) for the field-widget catalog.
- **`editable_table "Title" of <entity> { ... }`** — Oracle APEX's
  Interactive Grid look: one CSS Grid sharing column tracks between a
  header row and every data row (each an inline-editable form), plus an
  "add new" row. Same `item` syntax as `form`. `page_size: <n>` turns on
  pagination, a search box, and clickable column-header sort — all
  three as one bundle, same as a Report's Interactive Report — with
  offset pagination; omitted, it loads every row unpaginated (the
  historical default, kept for backward compatibility).
- **`chart "Title" from query <name> { type: bar|line|area|scatter|pie|donut x: <col> y: <col> }`**
  — see [Charts](./charts.md).
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
  server. `title` labels each marker (a hover tooltip); `link:`, if
  given, makes each marker a link to that page forwarding the row's id.
  Read-only, unpaginated, always sourced from the entity's own data
  table (same restriction as `Calendar`), and skips any row whose `lat`
  or `lng` is null.
- **`action "Label" calls <module> (config...)`** — a button running a
  server-side action module; see [Server-side actions](./actions.md).
- **`button "Label" -> page <Name> (extra: param, ...)`** or **`button
  "Label" calls <module> (config...)`** — a standalone button
  independent of any report row: the first form redirects, forwarding
  the current page's own query-string values under new names (same
  shape as a report's `link:` above, but from the page's own context
  instead of a row); the second runs a server-side action, identical to
  `action` above. Two behaviors, one component kind, so the App
  Builder's "Add Component" panel only needs one entry for both — the
  closest pgapp equivalent to Oracle APEX's button (`redirectThisApp`
  vs. a process-submitting button; see [Migrating from Oracle APEX](./migration-from-apex.md)
  below).
- **`on <event> of <item> { ... }`** — a client-side dynamic action;
  see [Dynamic actions](./actions.md#dynamic-actions).
- Any component can end with **`attrs (id: "...", class: "...",
  data_foo: "bar")`** to set a custom id/extra class/attribute on its
  wrapper tag — see [Theming](./themes.md).

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

---

Next: [Reports in depth](./reports.md) · [Forms & item types](./forms.md) · [Charts](./charts.md)
