-- In-database metadata for applications, entities, fields and pages.
-- Application data tables live in pgapp_data, generated from this metadata.
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

create table if not exists pgapp_meta.pages (
    id        serial primary key,
    app_id    integer not null references pgapp_meta.apps(id) on delete cascade,
    entity_id integer not null references pgapp_meta.entities(id) on delete cascade,
    name      text not null,
    page_type text not null default 'crud',
    unique (app_id, name)
);

create table if not exists pgapp_meta.page_fields (
    id             serial primary key,
    page_id        integer not null references pgapp_meta.pages(id) on delete cascade,
    field_id       integer not null references pgapp_meta.fields(id) on delete cascade,
    shown_in_list  boolean not null default false,
    shown_in_form  boolean not null default false,
    ordinal        integer not null default 0,
    unique (page_id, field_id)
);
