[← Back to README](../README.md)

# Charts & icons

- [Chart types](#chart-types)
- [Pluggable rendering backend](#pluggable-rendering-backend)
- [Icons](#icons)

## Chart types

`type:` is one of six (`model::CHART_TYPES`), checked at sync time:

| type | rendering |
|---|---|
| `bar` | one rect per row |
| `line` | polyline + marker dots |
| `area` | `line`, filled to baseline |
| `scatter` | marker dots only |
| `pie` | wedges from 12 o'clock, side legend |
| `donut` | `pie` with the center punched out |

`bar`/`line`/`area`/`scatter` use `x` (category)/`y` (value); `pie`/
`donut` reuse the same two props as a slice's label/value.

## Pluggable rendering backend

Rendering backend is pluggable (`src/chart_lib.rs`): **`inline`**
(default) computes straight to SVG server-side, no JS; any other name
loads `chart-libs/<name>/chart.js`, and the server instead emits a
`<script type="application/json" class="pgapp-chart-data">` blob
(`{rows, x, y, type}`) for that JS to render however it likes.
Selected with `chart_lib: <name>`. `chart-libs/canvas-bars/` ships as
a working `<canvas>` example.

**`inline`'s bar/pie/donut marks are colored per category**, cycling
through eight `--chart-1`…`--chart-8` CSS custom properties (a theme
sets these the same way it sets any other design token — see
`themes/shadcn/theme.css`) so a bar chart's bars and a pie/donut's
slices are actually distinguishable instead of one flat `currentColor`
fill; `line`/`area`/`scatter` stay a single accent color, since those
read as one series rather than discrete categories. A theme that
doesn't define `--chart-N` still renders distinct colors — every
`var(--chart-N, ...)` carries the same validated 8-hue fallback (see
`src/render.rs`'s `CHART_PALETTE`) as its default. **Every mark also
carries a native SVG `<title>`**, so hovering any bar, slice, or
line/area/scatter point shows its category and value as a plain
browser tooltip — no JS needed for that either.

## Icons

A `Report`/`EditableTable` row's Edit/Delete glyphs come from a
pluggable icon pack (`src/icons.rs`): **`builtin`** (default, two
inline SVGs, no network) or `icons/<name>/pack.json` (`{"stylesheet":
"<url>", "icons": {"edit": {"class": "...", "content": "..."}}}` — Font
Awesome-style or ligature-style like Material Icons both fit). Selected
with `icons: <name>`.

---

Next: [Theming](./themes.md) · [Forms & item types](./forms.md)
