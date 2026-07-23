[← Back to README](../README.md)

# Reports in depth

- [Pagination](#pagination)
- [Report search & saved views](#report-search--saved-views)
- [Computed columns & format masks](#computed-columns--format-masks)
- [Deployment checks](#deployment-checks)
- [Pre-load actions](#pre-load-actions)

See [Components reference](./components.md#components-reference) for the full `report`
grammar (aggregates, control breaks, highlighting, CSV export, display
modes).

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

## Deployment checks

On every sync, each table-backed entity's physical table is verified
against its declared fields via `information_schema`: a type mismatch
or missing column fails startup naming every problem (rather than a
confusing cast error at request time). Extra columns are only warned
about. pgapp adds columns but never changes or drops them.

## Pre-load actions

A `report` may declare `before_load:`, which runs a server-side action
module automatically, on every request, immediately before that report
fetches its rows — the same `ServerAction` registry and `ActionContext`
an `action` component uses (see [Server-side actions](./actions.md)), just
triggered by a page load instead of a click:

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

This is what turns a [collection](./markup.md#collections) from "shows whatever the
last button click captured" into "always shows fresh data" — no
separate refresh click needed, and no dynamic-action wiring either.
`before_load` isn't limited to `http_request`; it's any registered
action module, so a `call_function` that recomputes something
server-side works the same way.

**Failures are non-fatal.** An unreachable third-party API shouldn't
take the whole page down: if `before_load` fails, the report still
renders with whatever data already exists (from an earlier successful
run, or empty if there's never been one), with the error shown as an
inline warning above the table instead of blocking the page.

---

Next: [Actions](./actions.md) · [App Builder](./app-builder.md)
