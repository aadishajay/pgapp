[← Back to README](../README.md)

# Theming

- [The contract](#the-contract)
- [Per-instance overrides](#per-instance-overrides)
- [Shipped themes](#shipped-themes)
- [Header icon / assets](#header-icon--assets)
- [Adding a theme](#adding-a-theme)
- [Theme editor (App Builder)](#theme-editor-app-builder)
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
Rust changes. Every theme picker (`pgapp new`'s CLI prompt, the App
Builder's AppSettings dropdown) is generated from `theme::list_themes()`,
a live scan of `themes/*/theme.css` — a hand-dropped directory shows up
everywhere the moment it exists, no separate registration step.

## Theme editor (App Builder)

The App Builder's "Themes" page (its own nav item, not scoped to any
workspace/app — see the page's own doc comment in
`examples/app_builder.pgapp`) lists every theme on disk and lets you:

- **Clone** an existing theme into a new one under a new name — copies
  its `theme.css` and writes a fresh `theme.json` labeled after the new
  name, so the clone doesn't confusingly inherit the source's own label.
- **Edit** the clone's `theme.css` in a plain textarea and **Save** —
  takes effect immediately for every app already using that theme, since
  the route serving `theme.css` (`server::theme_css`) reads the file
  straight off disk on every request; there's no cache to invalidate and
  no reload step, unlike editing an app's own markup.

Backed by four Builder-only admin routes (`GET /admin/themes-list`,
`GET`/`POST /admin/themes/:name/css`, `POST /admin/themes/clone`),
gated by `theme_admin_guard` — the inverse of `admin_edit_guard`: it
*requires* the request be reaching the App Builder's own fixed
workspace/app rather than refusing it, since theme files aren't scoped
to any one app the way markup edits are. No database table backs any
of this — a theme is, and stays, nothing more than a directory under
`themes/`, the same as before this page existed.

## Mobile

No per-app work required: a viewport meta tag, horizontally-scrolling
table wrappers, and each shipped theme's `@media (max-width: 640px)`
rules (nav wraps, report toolbar stacks, floating form becomes a
near-full-width sheet). Multi-level `nav` also has a click-to-toggle
caret (touch has no `:hover`), working alongside the existing hover
behavior.

---

Next: [Charts & icons](./charts.md) · [App Builder](./app-builder.md)
