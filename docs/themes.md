[← Back to README](../README.md)

# Theming

- [The contract](#the-contract)
- [Per-instance overrides](#per-instance-overrides)
- [Shipped themes](#shipped-themes)
- [Header icon / assets](#header-icon--assets)
- [Adding a theme](#adding-a-theme)
- [Mobile](#mobile)

## The contract

Every server-rendered element carries one of a fixed set of
`.pgapp-*` classes (`pgapp-nav`, `pgapp-table`, `pgapp-form`,
`pgapp-field`, `pgapp-input`, `pgapp-btn` + variants, `pgapp-report`,
`pgapp-chart`, `pgapp-popup` + subparts, `pgapp-region`,
`pgapp-editable-table`, `pgapp-alert`, `pgapp-pagination`, `pgapp-icon`,
and a few more — grep `render.rs` for the exhaustive list) and nothing
else. A **theme** just gives those classes an appearance; that's the
whole contract.

## Per-instance overrides

Any component can end with `attrs (id: "...", class: "...", data_foo:
"bar")` — `id`/`class` are reserved (`class` *appends* to the required
class, never replacing it), any other key becomes an attribute (`_` →
`-`). On `form`/`editable_table`, `item <field> attrs (...)` does the
same one level deeper, for just that field's wrapper, independent of
(or combined with) an `as <kind>` override. Both are pure opt-in —
unset, a component/field renders exactly as before.

A theme is `themes/<name>/theme.css` (required) + `theme.json`
(optional, `{"label", "description"}`), selected with `theme: <name>`
(default `shadcn`) — a missing theme refuses startup with a clear
error.

## Shipped themes

`shadcn` (default zinc palette, HSL vars, light/dark),
`plain` (zero design-system assumptions), `vivid` (colorful demo
theme, used by `examples/helpdesk.pgapp`), `google_m3` (Material
Design 3 — tonal surfaces, pill buttons, 4px field radius, 28px dialog
corners; selected as `google_m3` since markup identifiers can't
contain hyphens), `apex_universal` (evokes Oracle APEX's classic
Universal Theme / Theme 42: white regions, a bold title underlined in
signature blue, rectangular low-radius buttons, a plain white top nav
bar, and a light-gray Interactive Report-style table header — no dark
mode, same as the original; used by `examples/venpay.pgapp`), `postgres`
(a `shadcn` derivative re-skinned around Postgres's own brand blue for
just the primary/accent/ring tokens — everything else carries over
unchanged — plus a `.pgapp-brand::before` header icon pointing at
`assets/pgapp-logo.svg`; used by `examples/showcase.pgapp`).

## Header icon / assets

A theme's header icon is the one place a theme needs an image, not just
CSS: `.pgapp-brand::before { background-image: url("assets/pgapp-logo.svg"); }`
— a plain *relative* URL, so it resolves against whatever page it's
loaded from (`/:workspace/:app/theme.css`) to `/:workspace/:app/assets/pgapp-logo.svg`,
without theme.css needing to know its own workspace/app slug. That route
is `server::asset()`, which serves exactly `assets/app.css`, `assets/app.js`,
and `assets/pgapp-logo.svg` from one shared directory (not per-app) — add
another filename there (and its content-type) to serve more.

## Adding a theme

`themes/<name>/theme.css` + `theme: <name>` in the app's markup — no
Rust changes.

## Mobile

No per-app work required: a viewport meta tag, horizontally-scrolling
table wrappers, and each shipped theme's `@media (max-width: 640px)`
rules (nav wraps, report toolbar stacks, floating form becomes a
near-full-width sheet). Multi-level `nav` also has a click-to-toggle
caret (touch has no `:hover`), working alongside the existing hover
behavior.

---

Next: [Charts & icons](./charts.md) · [App Builder](./app-builder.md)
