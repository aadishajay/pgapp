[← Back to README](../README.md)

# Migrating from Oracle APEX

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
| `page N (name, title, ...)`                                   | `page "Title" { ... }` (see [Markup](./markup.md))                                        |
| `region ... (type: interactiveReport, source { tableName })`  | `report "Title" of <entity> { columns: ... }`                                |
| `region ... (type: staticContent)` / a plain query region     | `region "Label" from query <name> { columns: ... }`                          |
| `region ... (type: cards)` / `region ... (type: list)`        | `report "Title" of <entity> { display: cards }` / `{ display: list }` — a display mode on the same `report`, not a separate region kind |
| `region ... (type: calendar)`                                  | `calendar "Title" of <entity> { date: <field> title: <field> }` |
| `region ... (type: map)`                                        | `map "Title" of <entity> { lat: <field> lng: <field> title: <field> }` — an inline-SVG scatter, no external mapping library/tile server |
| a region's `column NAME (heading, layout, appearance, ...)`   | one name in `columns:`, plus `heading <column>: "<Display name>"` / `align <column>: left\|center\|right` for the heading/alignment overrides, and `format <column>: <mask>` for display formatting (see [Reports](./reports.md#computed-columns--format-masks)) |
| a column's "Derived Column"/computation                       | `computed <name>: "<sql>"` on the report (see [Reports](./reports.md#computed-columns--format-masks)) |
| `region`'s `link { target: { page, items } }`                 | a report's `link: <field> -> page <Name> (extra: param, ...)`               |
| `button ... (behavior { action: redirectThisApp, target })`  | `button "Label" -> page <Name> (extra: param, ...)`                          |
| `button ... (behavior { action: submit }, serverSideCondition { whenButtonPressed })` + a `process` | `button "Label" calls <action_name> (config...)` — write the process's logic as a registered action module (see [Server-side actions](./actions.md)) instead of PL/SQL |
| a page item (`P7_TRIP_ID`, etc.)                              | a `Form`/`EditableTable` field, or a cross-page query-string parameter        |
| `dynamicAction ... (when { event }, action { action: ... })`  | `on <event> of <item> { show/hide/toggle/set/refresh }` (native APEX events beyond click/change aren't supported) |
| shared LOV (`shared-components/lovs.apx`)                     | `query <name> { sql: "..." }` referenced inline from `item ... as popup from query <name>` — not a standalone reusable object yet |
| `authentication` scheme                                       | the app-level `auth { }` block (on/off) |
| `authorization` scheme                                        | `requires: <role>` on a page or component, or a named, reusable `auth_scheme { roles: ... }` referenced by name (see [Authentication & authorization](./authentication.md#component-level-requires-and-named-auth-schemes)) — no PL/SQL-expression or SQL-query scheme types, no separate Access Control allow-list |
| `process` (PL/SQL)                                            | a registered Rust action module (`src/actions.rs`) called from `before_load`, `action`, or a `button calls` |

For each APEX page: create the matching pgapp `page`, then migrate its
regions/reports one at a time, using the [App Builder](./app-builder.md)'s raw
component editor to iterate quickly without restarting the server.
Anything with no row in this table (per-column formatting, LOVs/auth
schemes as objects, processes as a distinct type) has no pgapp
equivalent yet — reimplement that logic in the target page's own query
SQL, a registered action module, or a dynamic action, whichever fits.

`examples/venpay.pgapp` is a real APEX app hand-ported this way — see
[Getting started](./getting-started.md#bundled-example-apps).

---

Next: [App Builder](./app-builder.md) · [Roadmap](./roadmap.md)
