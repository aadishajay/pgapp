-- In-database metadata for applications, entities, fields, pages,
-- components, and navigation. Application data tables live in
-- pgapp_data, generated from this metadata.
create schema if not exists pgapp_meta;
create schema if not exists pgapp_data;

create table if not exists pgapp_meta.apps (
    id         serial primary key,
    name       text not null unique,
    created_at timestamptz not null default now()
);
-- App-level settings, declared in the markup file (`theme:`, `icons:`,
-- `chart_lib:`, `auth { }`) and synced here — configuration is part of
-- the app definition, not the process environment.
alter table pgapp_meta.apps add column if not exists theme text;
alter table pgapp_meta.apps add column if not exists icons text;
alter table pgapp_meta.apps add column if not exists chart_lib text;
alter table pgapp_meta.apps add column if not exists auth_enabled boolean not null default false;
-- Which schema this app's physical data tables live in — 'pgapp_data'
-- for the classic single-workspace flow, or a workspace's own schema
-- name when the app was created via `pgapp app create`/`pgapp run
-- --workspace` (see src/control.rs, src/instance.rs). Every data-table
-- reference in server.rs/meta/sync.rs is qualified by this, not a
-- hardcoded literal.
alter table pgapp_meta.apps add column if not exists data_schema text not null default 'pgapp_data';

-- Authentication: one user store per app. Passwords are argon2 hashes
-- (never plaintext); role is a free-form string checked against a
-- page's required_role ('admin' passes every check). Users are managed
-- at runtime via the built-in /users admin page — never from markup,
-- which is why there's no sync phase for this table.
create table if not exists pgapp_meta.users (
    id            serial primary key,
    app_id        integer not null references pgapp_meta.apps(id) on delete cascade,
    username      text not null,
    password_hash text not null,
    role          text not null default 'user',
    created_at    timestamptz not null default now(),
    unique (app_id, username)
);

-- Server-side login sessions: the browser holds only the random token
-- in an HttpOnly cookie; everything else lives here so sessions can be
-- revoked by deleting rows.
create table if not exists pgapp_meta.sessions (
    token      text primary key,
    app_id     integer not null references pgapp_meta.apps(id) on delete cascade,
    user_id    integer not null references pgapp_meta.users(id) on delete cascade,
    expires_at timestamptz not null
);

create table if not exists pgapp_meta.entities (
    id         serial primary key,
    app_id     integer not null references pgapp_meta.apps(id) on delete cascade,
    name       text not null,
    table_name text not null,
    unique (app_id, name)
);
-- Non-null = a read-only entity backed by a named query instead of a
-- physical table: no table is created and no Form/EditableTable may
-- bind to it (enforced at sync time).
alter table pgapp_meta.entities add column if not exists source_query text;
-- Non-null = a read-only entity backed by a collection instead (see
-- pgapp_meta.collections below) — same read-only restriction as
-- source_query, and mutually exclusive with it.
alter table pgapp_meta.entities add column if not exists source_collection text;

-- APEX-collection-style scratch storage: rows an app writes at runtime
-- (typically an external API response via the http_request action),
-- not part of its declared schema. Scoped by caller_key, not user_id,
-- so it works the same whether or not the app uses auth { } — see
-- server/auth.rs's CALLER_COOKIE. "Only the caller can see it" is
-- enforced by every read going through the server-generated SQL an
-- `entity ... from collection "name"` compiles to (src/server.rs),
-- never a hand-written named query, so there's no WHERE clause an app
-- author could omit or get wrong.
create table if not exists pgapp_meta.collections (
    id         bigserial primary key,
    app_id     integer not null references pgapp_meta.apps(id) on delete cascade,
    caller_key text not null,
    name       text not null,
    seq        integer not null,
    data       jsonb not null,
    created_at timestamptz not null default now()
);
create index if not exists collections_lookup on pgapp_meta.collections (app_id, caller_key, name, seq);

create table if not exists pgapp_meta.fields (
    id            serial primary key,
    entity_id     integer not null references pgapp_meta.entities(id) on delete cascade,
    name          text not null,
    data_type     text not null,
    is_required   boolean not null default false,
    default_value text,
    ordinal       integer not null default 0,
    unique (entity_id, name)
);

-- A page is just a name: what it's made of lives entirely in
-- `components` below (an ordered list of report/form/editable_table/
-- chart/text/link/region blocks).
create table if not exists pgapp_meta.pages (
    id     serial primary key,
    app_id integer not null references pgapp_meta.apps(id) on delete cascade,
    name   text not null,
    unique (app_id, name)
);
-- Earlier schema versions had page-level entity/kind/link columns —
-- superseded entirely by `components`.
alter table pgapp_meta.pages drop column if exists entity_id;
alter table pgapp_meta.pages drop column if exists page_type;
alter table pgapp_meta.pages drop column if exists link_field;
alter table pgapp_meta.pages drop column if exists link_target_page_id;
alter table pgapp_meta.pages drop column if exists source_query_name;
alter table pgapp_meta.pages drop column if exists link_params;
-- Authorization: `requires: <role>` in the page's markup. Null = any
-- signed-in user (or everyone, when the app has no auth block).
alter table pgapp_meta.pages add column if not exists required_role text;

-- One independently-rendered piece of a page (page_id set), or of the
-- app-wide header/footer chrome (page_id null, slot = 'header' |
-- 'footer'). `kind` names a component (report/form/editable_table/
-- chart/text/link/region) and `config` is that component's entire
-- definition as a generic JSON blob (title, entity, columns, item
-- types, chart axes, link targets by page *name*, ...) — the same
-- "generic config" pattern used for item types, extended up to the
-- whole-component level so adding a new component kind never requires
-- a schema change here. Page/entity/item-type references inside
-- `config` are validated by name at sync time (see meta::sync_app),
-- mirroring how item kinds are checked against the item type registry.
create table if not exists pgapp_meta.components (
    id      serial primary key,
    app_id  integer not null references pgapp_meta.apps(id) on delete cascade,
    page_id integer references pgapp_meta.pages(id) on delete cascade,
    slot    text, -- null | 'header' | 'footer'
    kind    text not null,
    ordinal integer not null default 0,
    config  jsonb not null default '{}'
);
create index if not exists components_page_idx on pgapp_meta.components (page_id, ordinal);
create index if not exists components_app_slot_idx on pgapp_meta.components (app_id, slot, ordinal);

-- Earlier schema versions modeled page items / form fields as their
-- own tables — all superseded by `components`.
drop table if exists pgapp_meta.page_fields;
drop table if exists pgapp_meta.page_items;
drop table if exists pgapp_meta.page_field_items;
drop table if exists pgapp_meta.header_items;
drop table if exists pgapp_meta.footer_items;

-- The app's (possibly multi-level) navigation bar. Self-referencing
-- parent_id makes a leaf (target_page_id set) or a group (children,
-- no target of its own).
create table if not exists pgapp_meta.nav_items (
    id              serial primary key,
    app_id          integer not null references pgapp_meta.apps(id) on delete cascade,
    parent_id       integer references pgapp_meta.nav_items(id) on delete cascade,
    label           text not null,
    target_page_id  integer references pgapp_meta.pages(id),
    ordinal         integer not null default 0
);

-- Named, reusable SQL queries. page_id null = app-scoped (visible from
-- every page); page_id set = visible only within that page, shadowing
-- an app-scoped query of the same name. sql_text may contain `:name`
-- bind markers (see meta::compile_named_query).
create table if not exists pgapp_meta.named_queries (
    id       serial primary key,
    app_id   integer not null references pgapp_meta.apps(id) on delete cascade,
    page_id  integer references pgapp_meta.pages(id) on delete cascade,
    name     text not null,
    sql_text text not null
);
create unique index if not exists named_queries_scope_name_idx
    on pgapp_meta.named_queries (app_id, coalesce(page_id, 0), name);

-- Saved report views: a named bookmark of one report's filter state
-- (the r<idx>_q / r<idx>_col / r<idx>_val parameters, as a params
-- blob). owner_user_id null = saved outside auth (or by a deleted
-- user); is_public makes a view visible to every signed-in user, not
-- just its owner. Component_idx pins the view to one report on one
-- page.
create table if not exists pgapp_meta.report_views (
    id            serial primary key,
    app_id        integer not null references pgapp_meta.apps(id) on delete cascade,
    page_name     text not null,
    component_idx integer not null,
    name          text not null,
    owner_user_id integer references pgapp_meta.users(id) on delete cascade,
    is_public     boolean not null default false,
    params        jsonb not null default '{}',
    created_at    timestamptz not null default now()
);

-- The pgapp runtime JS library (item value capture, etc.) lives here,
-- not as a static file: seeded from a built-in default the first time
-- an app is synced (ON CONFLICT DO NOTHING), then freely editable in
-- place afterward without touching the binary.
create table if not exists pgapp_meta.app_runtime_js (
    app_id     integer primary key references pgapp_meta.apps(id) on delete cascade,
    content    text not null,
    updated_at timestamptz not null default now()
);
