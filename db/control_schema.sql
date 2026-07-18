-- pgapp's own control plane: which apps this server serves, and where
-- their markup lives on disk. Deliberately a separate schema from
-- pgapp_meta (the synced runtime metadata *of* an app) and pgapp_data
-- (an app's own rows) — this table is pgapp managing itself, closer to
-- what an APEX workspace/application registry is than to anything an
-- app author's markup ever touches.
create schema if not exists pgapp_control;

create table if not exists pgapp_control.apps (
    id          serial primary key,
    slug        text not null unique,
    markup_path text not null,
    enabled     boolean not null default true,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);
