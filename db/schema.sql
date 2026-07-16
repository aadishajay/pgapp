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

create table if not exists pgapp_meta.entities (
    id         serial primary key,
    app_id     integer not null references pgapp_meta.apps(id) on delete cascade,
    name       text not null,
    table_name text not null,
    unique (app_id, name)
);

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

-- The pgapp runtime JS library (item value capture, etc.) lives here,
-- not as a static file: seeded from a built-in default the first time
-- an app is synced (ON CONFLICT DO NOTHING), then freely editable in
-- place afterward without touching the binary.
create table if not exists pgapp_meta.app_runtime_js (
    app_id     integer primary key references pgapp_meta.apps(id) on delete cascade,
    content    text not null,
    updated_at timestamptz not null default now()
);
