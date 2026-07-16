-- In-database metadata for applications, entities, fields, pages, page
-- items, and navigation. Application data tables live in pgapp_data,
-- generated from this metadata.
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

-- entity_id is nullable: "static" pages (pure page items, no data) have
-- no backing entity at all.
create table if not exists pgapp_meta.pages (
    id         serial primary key,
    app_id     integer not null references pgapp_meta.apps(id) on delete cascade,
    entity_id  integer references pgapp_meta.entities(id) on delete cascade,
    name       text not null,
    page_type  text not null default 'list', -- 'list' | 'detail' | 'static'
    unique (app_id, name)
);
alter table pgapp_meta.pages alter column entity_id drop not null;
alter table pgapp_meta.pages add column if not exists link_field text;
alter table pgapp_meta.pages
    add column if not exists link_target_page_id integer references pgapp_meta.pages(id);

create table if not exists pgapp_meta.page_fields (
    id             serial primary key,
    page_id        integer not null references pgapp_meta.pages(id) on delete cascade,
    field_id       integer not null references pgapp_meta.fields(id) on delete cascade,
    shown_in_list  boolean not null default false,
    shown_in_form  boolean not null default false,
    ordinal        integer not null default 0,
    unique (page_id, field_id)
);

-- Content placed on a page beyond its entity-bound table/form: static
-- text, a link to another page, or a region rendering a named query's
-- rows (query_name, set only for kind = 'region').
create table if not exists pgapp_meta.page_items (
    id              serial primary key,
    page_id         integer not null references pgapp_meta.pages(id) on delete cascade,
    kind            text not null, -- 'text' | 'link' | 'region'
    label           text not null,
    target_page_id  integer references pgapp_meta.pages(id),
    ordinal         integer not null default 0
);
alter table pgapp_meta.page_items add column if not exists query_name text;

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

-- App-wide chrome shown on every page: same shape as page_items, just
-- scoped to the app rather than one page.
create table if not exists pgapp_meta.header_items (
    id              serial primary key,
    app_id          integer not null references pgapp_meta.apps(id) on delete cascade,
    kind            text not null, -- 'text' | 'link' | 'region'
    label           text not null,
    target_page_id  integer references pgapp_meta.pages(id),
    ordinal         integer not null default 0
);
alter table pgapp_meta.header_items add column if not exists query_name text;

create table if not exists pgapp_meta.footer_items (
    id              serial primary key,
    app_id          integer not null references pgapp_meta.apps(id) on delete cascade,
    kind            text not null, -- 'text' | 'link' | 'region'
    label           text not null,
    target_page_id  integer references pgapp_meta.pages(id),
    ordinal         integer not null default 0
);
alter table pgapp_meta.footer_items add column if not exists query_name text;

-- How each form field is presented: a "static LOV" of choices for
-- radio/popup, a named query supplying `value`/`label` columns instead,
-- or nothing for text/readonly/checkbox.
create table if not exists pgapp_meta.page_field_items (
    id             serial primary key,
    page_id        integer not null references pgapp_meta.pages(id) on delete cascade,
    field_name     text not null,
    item_type      text not null default 'text', -- 'text' | 'readonly' | 'checkbox' | 'radio' | 'popup'
    choices        text[] not null default '{}',
    unique (page_id, field_name)
);
alter table pgapp_meta.page_field_items add column if not exists choices_query text;

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

-- `list` pages normally report on `select * from` their entity table;
-- source_query_name overrides that with a named query instead (writes
-- still go to the entity by id). link_params carries the row-link
-- column's extra forwarded parameters as [{"field": "...", "param":
-- "..."}], since they don't reference another table by id.
alter table pgapp_meta.pages add column if not exists source_query_name text;
alter table pgapp_meta.pages add column if not exists link_params jsonb not null default '[]';

-- The pgapp runtime JS library (item value capture, etc.) lives here,
-- not as a static file: seeded from a built-in default the first time
-- an app is synced (ON CONFLICT DO NOTHING), then freely editable in
-- place afterward without touching the binary.
create table if not exists pgapp_meta.app_runtime_js (
    app_id     integer primary key references pgapp_meta.apps(id) on delete cascade,
    content    text not null,
    updated_at timestamptz not null default now()
);
