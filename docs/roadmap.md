[← Back to README](../README.md)

# Roadmap (not in this slice)

pgapp is deliberately the *smallest end-to-end loop*, not the whole
framework — one Postgres schema per workspace, a handful of field
types, existing to prove the architecture before building the bigger
pieces. Known gaps, honestly listed:

- Separate connection *pool* per workspace — [Instance mode](./getting-started.md#instance-mode) gives
  every workspace its own schema/role, but all of them still share one
  `PgPool` per process (matches how APEX itself pools connections; a
  true pool-per-workspace would be a bigger, probably unnecessary,
  change)
- No CLI-driven credential rotation — a workspace/pgapp_admin password
  is set once at creation; changing it means an ad hoc `ALTER ROLE`
  today, no `pgapp instance rotate-password`-style command yet
- More field types and real relationships (foreign keys) — named
  queries cover ad hoc joins today, but no schema-level entity-to-entity
  references yet
- A real drag-and-drop builder UI (today's [App Builder](./app-builder.md) is a
  click-to-select tree + property panel, not free drag-and-drop
  page composition)
- Multi-step `flow` blocks chaining actions/dynamic actions with
  conditions
- runtime.js is seeded once per app; picking up a newer built-in seed
  needs deleting the `pgapp_meta.app_runtime_js` row — no versioned
  upgrade story yet
- Field-level authorization (page- and component-level `requires:`
  exist, per-column doesn't), plus password reset flows (today an admin
  deletes/recreates the account)
- Item type config is always flat strings, even for numeric-looking
  values (Slider's `min`/`max`)
- `ensure_data_table` adds columns to an existing table but doesn't
  handle renames, type changes, or drops
- Separate create vs. edit field lists (a `readonly` field with a
  meaningful default doesn't get nulled out on create)
- `RegionRows` is keyed only by query name per request — a page-scoped
  and an app-scoped query sharing a name would collide (rare, not
  guarded against)
- No validation of a named query's SQL beyond the bind-marker scan — a
  typo surfaces as a runtime error on first use
- A `Report`'s row actions only wire to a `Form` on the *same page*
- CSS-icon packs whose stylesheet is a remote CDN URL need outbound
  network access to actually render glyphs
- Re-registering an already-registered slug into a *different*
  workspace re-points it (same "the registry decides" behavior as
  everywhere else) but doesn't migrate its existing data — the old
  workspace's physical tables are silently orphaned (not dropped, not
  moved), and the app starts over with fresh empty tables in the new
  workspace. Live-verified as part of scrapping classic mode: no
  automatic detection or warning yet.
- `pgapp_meta.apps.name` (the declared `app "Name" { }`) is unique
  **instance-wide**, not per-workspace — two unrelated apps that happen
  to declare the identical name collide even in different workspaces
  (the second sync silently repoints the first's metadata row rather
  than erroring). Give apps distinct names. This is unlike
  `pgapp_control.apps.slug` (the URL identifier, derived from the name
  via `slugify`), which *is* only unique per workspace — two apps named
  differently enough to avoid the `pgapp_meta.apps.name` collision but
  whose slugs happen to match (e.g. "Reports" and "REPORTS") coexist
  fine, routed independently at `/<workspace>/reports/...` each.

Have an opinion on what should come first? Open an issue.
